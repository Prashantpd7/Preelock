import Foundation

/// Thread-safe manager for dynamic website filtering policies (RFC 1035 compliant).
/// Supports Version 2 Policy formats with both "blocklist" and "allowlist" modes,
/// automatic subdomain matching, dynamic file watching with atomic cache invalidation,
/// configurable DNS-over-HTTPS (DoH) bypass protection, and safe fallback against malformed policy mutations.
public final class BlocklistManager: @unchecked Sendable {

    public static let shared = BlocklistManager()

    public static let defaultPolicyName = "blocklist.json"

    private let lock = NSLock()
    private var policy: Policy
    private var rules: Set<String> = []
    private var dohEndpoints: Set<String> = []
    private var adultDomains: Set<String> = []
    private var activeURL: URL?
    private var lastModDate: Date?

    private var reloadTimer: DispatchSourceTimer?
    private let monitorQueue = DispatchQueue(label: "com.preelock.poc.policymonitor", qos: .utility)

    public init() {
        // Initialize with default fallback policy
        let defaultDomains = ["instagram.com", "facebook.com", "reddit.com", "tiktok.com"]
        self.policy = Policy(version: 2, name: "Default Blocklist", mode: .blocklist, domains: defaultDomains, block_doh: true, block_adult: false)
        self.rules = Set(defaultDomains)

        // Load DoH resolver endpoints from policies/doh_endpoints.json or built-in defaults
        loadDoHEndpoints()

        // Load adult domains blocklist from policies/adult-domains.txt or resources
        loadAdultDomains()

        // Locate and load initial policy
        if let initialURL = Self.locateInitialPolicyURL() {
            self.activeURL = initialURL
            _ = loadPolicy(from: initialURL, isInitial: true)
        }
    }

    // MARK: - Initial File Discovery
    private static func locateInitialPolicyURL() -> URL? {
        let cwd = FileManager.default.currentDirectoryPath
        var candidates = [
            cwd + "/policy.json",
            cwd + "/blocklist.json",
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/policy.json",
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/blocklist.json",
            "/Users/Prashant/My_Workspace/PreelockDNSProofOfConcept/policy.json",
            "/Users/Prashant/My_Workspace/PreelockDNSProofOfConcept/blocklist.json"
        ]
        if let execPath = Bundle.main.executablePath ?? CommandLine.arguments.first {
            let execURL = URL(fileURLWithPath: execPath).resolvingSymlinksInPath()
            var dir = execURL.deletingLastPathComponent()
            while dir.lastPathComponent != "PreelockDNSProofOfConcept" && dir.lastPathComponent != "dns-proxy" && dir.path != "/" {
                dir = dir.deletingLastPathComponent()
            }
            if dir.lastPathComponent == "PreelockDNSProofOfConcept" || dir.lastPathComponent == "dns-proxy" {
                candidates.insert(dir.appendingPathComponent("policy.json").path, at: 0)
                candidates.insert(dir.appendingPathComponent("blocklist.json").path, at: 1)
            }
        }
        for path in candidates {
            if FileManager.default.fileExists(atPath: path) {
                return URL(fileURLWithPath: path)
            }
        }
        return nil
    }

    // MARK: - DoH Endpoint Management
    private func loadDoHEndpoints() {
        let builtInDoH = [
            "cloudflare-dns.com",
            "dns.cloudflare.com",
            "one.one.one.one",
            "1dot1dot1dot1.cloudflare-dns.com",
            "dns.google",
            "dns.google.com",
            "dns.quad9.net",
            "quad9.net",
            "doh.opendns.com",
            "dns.opendns.com",
            "dns.cleanbrowsing.org",
            "doh.cleanbrowsing.org",
            "dns.adguard-dns.com",
            "dns.nextdns.io",
            "doh.mullvad.net",
            "dns.controld.com",
            "freedns.controld.com",
            "mask.icloud.com",
            "mask-h2.icloud.com",
            "mask-api.icloud.com",
            "mask.apple-dns.net",
            "mask-tfo.apple-dns.net",
            "mask-api.fe.apple-dns.net",
            "mask-api.fe2.apple-dns.net",
            "use-application-dns.net"
        ]

        var candidates = [
            FileManager.default.currentDirectoryPath + "/policies/doh_endpoints.json",
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/policies/doh_endpoints.json",
            "/Users/Prashant/My_Workspace/PreelockDNSProofOfConcept/policies/doh_endpoints.json"
        ]
        if let execPath = Bundle.main.executablePath ?? CommandLine.arguments.first {
            let execURL = URL(fileURLWithPath: execPath).resolvingSymlinksInPath()
            var dir = execURL.deletingLastPathComponent()
            while dir.lastPathComponent != "PreelockDNSProofOfConcept" && dir.lastPathComponent != "dns-proxy" && dir.path != "/" {
                dir = dir.deletingLastPathComponent()
            }
            if dir.lastPathComponent == "PreelockDNSProofOfConcept" || dir.lastPathComponent == "dns-proxy" {
                candidates.insert(dir.appendingPathComponent("policies/doh_endpoints.json").path, at: 0)
            }
        }

        for path in candidates {
            if let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
               let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let list = json["endpoints"] as? [String] {
                self.dohEndpoints = Set(list.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() })
                print("[INFO] Loaded \(self.dohEndpoints.count) DoH/DoT resolver endpoints from \(path)")
                return
            }
        }

        self.dohEndpoints = Set(builtInDoH)
        print("[INFO] Initialized with \(self.dohEndpoints.count) built-in DoH/DoT resolver endpoints")
    }

    // MARK: - Adult Domain Blocklist Management
    private func loadAdultDomains() {
        let cwd = FileManager.default.currentDirectoryPath
        var candidates = [
            cwd + "/policies/adult-domains.txt",
            cwd + "/src-tauri/resources/adult-domains.txt",
            cwd + "/resources/adult-domains.txt",
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/policies/adult-domains.txt",
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/resources/adult-domains.txt"
        ]
        if let execPath = Bundle.main.executablePath ?? CommandLine.arguments.first {
            let execURL = URL(fileURLWithPath: execPath).resolvingSymlinksInPath()
            let binDir = execURL.deletingLastPathComponent()
            let resDir = binDir.deletingLastPathComponent()
            candidates.insert(resDir.appendingPathComponent("dns-proxy/policies/adult-domains.txt").path, at: 0)
            candidates.insert(resDir.appendingPathComponent("resources/adult-domains.txt").path, at: 1)
            candidates.insert(resDir.appendingPathComponent("adult-domains.txt").path, at: 2)
            candidates.insert(binDir.appendingPathComponent("adult-domains.txt").path, at: 3)

            var dir = execURL.deletingLastPathComponent()
            while dir.lastPathComponent != "Preelock" && dir.lastPathComponent != "dns-proxy" && dir.path != "/" {
                dir = dir.deletingLastPathComponent()
            }
            if dir.lastPathComponent == "Preelock" {
                candidates.insert(dir.appendingPathComponent("src-tauri/resources/adult-domains.txt").path, at: 0)
                candidates.insert(dir.appendingPathComponent("src-tauri/dns-proxy/policies/adult-domains.txt").path, at: 1)
            }
        }

        for path in candidates {
            if FileManager.default.fileExists(atPath: path),
               let content = try? String(contentsOfFile: path, encoding: .utf8) {
                let lines = content.components(separatedBy: .newlines)
                var set = Set<String>()
                set.reserveCapacity(lines.count)
                for line in lines {
                    let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                    if !trimmed.isEmpty && !trimmed.hasPrefix("#") {
                        set.insert(trimmed)
                    }
                }
                self.adultDomains = set
                print("[INFO] Loaded \(self.adultDomains.count) adult domains from \(path)")
                return
            }
        }

        print("[WARN] Could not find adult-domains.txt in candidate locations")
    }

    // MARK: - Matching Logic (Blocklist, Allowlist & DoH Bypass Protection)
    /// Evaluates whether a domain is blocked under the active policy.
    public func isBlocked(_ domain: String) -> (isBlocked: Bool, matchedRule: String?) {
        lock.lock()
        defer { lock.unlock() }

        let normalized = domain.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()

        if normalized.isEmpty {
            return (false, nil)
        }

        // 1. DNS-over-HTTPS (DoH) / DNS-over-TLS (DoT) Bypass Protection
        if policy.block_doh {
            let (dohMatched, dohRule) = matchDoHRule(normalized)
            if dohMatched {
                return (true, "Blocked DoH/DoT resolver endpoint: \(dohRule ?? normalized)")
            }
        }

        // 2. Adult Website Blocking
        if policy.block_adult {
            let (adultMatched, adultRule) = matchAdultRule(normalized)
            if adultMatched {
                return (true, "Blocked adult website: \(adultRule ?? normalized)")
            }
        }

        // 3. User Policy Evaluation (Blocklist vs Allowlist)
        let (matched, matchedDomain) = matchRule(normalized)

        switch policy.mode {
        case .blocklist:
            if matched {
                return (true, matchedDomain ?? normalized)
            } else {
                return (false, nil)
            }
        case .allowlist:
            if matched {
                // In allowlist mode: matched domain is ALLOWED
                return (false, nil)
            } else {
                // In allowlist mode: unlisted domain is BLOCKED
                return (true, "Blocked by allowlist policy '\(policy.name)'")
            }
        }
    }

    /// Evaluates if a domain matches any user rule directly or as a subdomain of a rule.
    private func matchRule(_ normalized: String) -> (matched: Bool, rule: String?) {
        // Exact match
        if rules.contains(normalized) {
            return (true, normalized)
        }

        // Subdomain match (e.g. "graph.instagram.com" matches parent "instagram.com")
        let labels = normalized.split(separator: ".").map(String.init)
        if labels.count > 1 {
            for i in 1..<labels.count {
                let parent = labels[i...].joined(separator: ".")
                if rules.contains(parent) {
                    return (true, parent)
                }
            }
        }

        return (false, nil)
    }

    /// Evaluates if a domain matches any known DoH resolver endpoint.
    private func matchDoHRule(_ normalized: String) -> (matched: Bool, rule: String?) {
        if dohEndpoints.contains(normalized) {
            return (true, normalized)
        }

        let labels = normalized.split(separator: ".").map(String.init)
        if labels.count > 1 {
            for i in 1..<labels.count {
                let parent = labels[i...].joined(separator: ".")
                if dohEndpoints.contains(parent) {
                    return (true, parent)
                }
            }
        }

        return (false, nil)
    }

    /// Evaluates if a domain matches any known adult domain.
    private func matchAdultRule(_ normalized: String) -> (matched: Bool, rule: String?) {
        if adultDomains.contains(normalized) {
            return (true, normalized)
        }

        let labels = normalized.split(separator: ".").map(String.init)
        if labels.count > 1 {
            for i in 1..<labels.count {
                let parent = labels[i...].joined(separator: ".")
                if adultDomains.contains(parent) {
                    return (true, parent)
                }
            }
        }

        return (false, nil)
    }

    // MARK: - Policy Loading & Dynamic Switching
    /// Loads and applies a policy from the specified URL.
    /// On failure (e.g. malformed JSON), retains the existing active policy without crashing.
    @discardableResult
    public func loadPolicy(from url: URL, isInitial: Bool = false) -> Result<Policy, Error> {
        do {
            let data = try Data(contentsOf: url)
            let newPolicy = try Policy.decode(from: data)

            lock.lock()
            self.policy = newPolicy
            self.rules = Set(newPolicy.domains.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }.filter { !$0.isEmpty })
            self.activeURL = url

            // Update modification timestamp for change tracking
            if let attrs = try? FileManager.default.attributesOfItem(atPath: url.path),
               let modDate = attrs[.modificationDate] as? Date {
                self.lastModDate = modDate
            } else {
                self.lastModDate = Date()
            }
            lock.unlock()

            // Invalidate cache immediately on policy update
            DNSCache.shared.clear()

            print("[INFO] Policy successfully loaded: '\(newPolicy.name)' (Mode: \(newPolicy.mode.rawValue), Rules: \(newPolicy.domains.count), Block DoH: \(newPolicy.block_doh), Block Adult: \(newPolicy.block_adult)) from \(url.lastPathComponent)")
            return .success(newPolicy)
        } catch {
            print("[WARN] Failed to load policy from '\(url.path)': \(error). Keeping current policy: '\(self.currentPolicy.name)'.")
            return .failure(error)
        }
    }

    /// Reloads the currently active policy file (e.g., after an edit).
    @discardableResult
    public func reload() -> Result<Policy, Error> {
        lock.lock()
        guard let url = activeURL else {
            lock.unlock()
            return .failure(NSError(domain: "BlocklistManager", code: -1, userInfo: [NSLocalizedDescriptionKey: "No active policy file"]))
        }
        lock.unlock()
        return loadPolicy(from: url)
    }

    /// Switches the active policy to a new file path.
    @discardableResult
    public func switchPolicy(to path: String) -> Result<Policy, Error> {
        let url: URL
        if path.hasPrefix("/") {
            url = URL(fileURLWithPath: path)
        } else {
            let cwd = FileManager.default.currentDirectoryPath
            url = URL(fileURLWithPath: cwd).appendingPathComponent(path)
        }

        guard FileManager.default.fileExists(atPath: url.path) else {
            let err = NSError(domain: "BlocklistManager", code: -2, userInfo: [NSLocalizedDescriptionKey: "Policy file not found: \(url.path)"])
            print("[WARN] \(err.localizedDescription). Keeping current policy.")
            return .failure(err)
        }

        return loadPolicy(from: url)
    }

    // MARK: - Dynamic Polling / Auto-Reload
    /// Starts background polling to detect file modifications without requiring proxy restarts.
    public func startAutoReload(intervalSeconds: Double = 1.0) {
        stopAutoReload()

        let timer = DispatchSource.makeTimerSource(queue: monitorQueue)
        timer.schedule(deadline: .now() + intervalSeconds, repeating: intervalSeconds)
        timer.setEventHandler { [weak self] in
            guard let self = self else { return }
            self.checkForFileModifications()
        }
        timer.resume()
        self.reloadTimer = timer
        print("[INFO] Dynamic policy file monitoring active (poll interval: \(intervalSeconds)s)")
    }

    public func stopAutoReload() {
        reloadTimer?.cancel()
        reloadTimer = nil
    }

    private func checkForFileModifications() {
        lock.lock()
        guard let url = activeURL else {
            lock.unlock()
            return
        }
        let recordedMod = self.lastModDate
        lock.unlock()

        guard let attrs = try? FileManager.default.attributesOfItem(atPath: url.path),
              let currentMod = attrs[.modificationDate] as? Date else {
            return
        }

        if let recordedMod = recordedMod, currentMod > recordedMod {
            print("[INFO] Policy file change detected on '\(url.lastPathComponent)'. Auto-reloading...")
            _ = loadPolicy(from: url)
        }
    }

    // MARK: - Query & Telemetry Accessors
    public var currentPolicy: Policy {
        lock.lock()
        defer { lock.unlock() }
        return policy
    }

    public var activeFilePath: String? {
        lock.lock()
        defer { lock.unlock() }
        return activeURL?.path
    }

    public func allRules() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return Array(rules).sorted()
    }

    public func allDoHEndpoints() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return Array(dohEndpoints).sorted()
    }

    // MARK: - Legacy Compatibility Methods
    public func loadFromStorage() {
        if let url = activeURL {
            _ = loadPolicy(from: url)
        }
    }

    public func addRule(_ rule: String) {
        lock.lock()
        let clean = rule.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !clean.isEmpty && !rules.contains(clean) {
            var updatedDomains = policy.domains
            updatedDomains.append(clean)
            policy = Policy(version: policy.version, name: policy.name, mode: policy.mode, domains: updatedDomains, block_doh: policy.block_doh, block_adult: policy.block_adult)
            rules.insert(clean)
        }
        let url = activeURL
        lock.unlock()

        if let url = url, let data = try? policy.encode() {
            try? data.write(to: url, options: .atomic)
        }
        DNSCache.shared.clear()
    }

    public func removeRule(_ rule: String) {
        lock.lock()
        let clean = rule.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        rules.remove(clean)
        let updatedDomains = policy.domains.filter { $0.lowercased() != clean }
        policy = Policy(version: policy.version, name: policy.name, mode: policy.mode, domains: updatedDomains, block_doh: policy.block_doh, block_adult: policy.block_adult)
        let url = activeURL
        lock.unlock()

        if let url = url, let data = try? policy.encode() {
            try? data.write(to: url, options: .atomic)
        }
        DNSCache.shared.clear()
    }
}

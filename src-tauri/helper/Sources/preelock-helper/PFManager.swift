import Foundation
import Darwin

public enum PFManager {
    public static let pfConfPath = "/etc/pf.conf"
    public static let anchorName = "com.preelock"
    public static let anchorDir = "/etc/pf.anchors"
    public static let anchorFilePath = "/etc/pf.anchors/com.preelock"
    public static let tokenFilePath = "/var/db/preelock/pf_token"

    public static let pfConfStart = "# PREELOCK_PF_START"
    public static let pfConfEnd = "# PREELOCK_PF_END"

    public static let cdnDomainMap: [String: [String]] = [
        "instagram.com": [
            "cdninstagram.com", "static.cdninstagram.com", "scontent.cdninstagram.com",
            "i.instagram.com", "graph.instagram.com", "api.instagram.com",
            "m.instagram.com", "l.instagram.com", "edge-chat.instagram.com", "fbcdn.net"
        ],
        "facebook.com": [
            "fbcdn.net", "facebook.net", "m.facebook.com", "web.facebook.com",
            "l.facebook.com", "graph.facebook.com", "edge-mqtt.facebook.com", "mbasic.facebook.com"
        ],
        "youtube.com": [
            "googlevideo.com", "ytimg.com", "ggpht.com",
            "m.youtube.com", "music.youtube.com", "studio.youtube.com", "youtu.be"
        ],
        "twitter.com": ["twimg.com", "api.twitter.com", "mobile.twitter.com"],
        "x.com": ["twimg.com", "api.x.com", "mobile.x.com", "pro.x.com"],
        "reddit.com": [
            "redditmedia.com", "redd.it", "old.reddit.com", "new.reddit.com",
            "sh.reddit.com", "oauth.reddit.com", "gateway.reddit.com", "gql.reddit.com"
        ],
        "linkedin.com": ["licdn.com"],
        "whatsapp.com": ["whatsapp.net", "fbcdn.net", "web.whatsapp.com"],
        "tiktok.com": ["tiktokcdn.com", "byteoversea.com", "ibyteimg.com", "m.tiktok.com", "vm.tiktok.com"]
    ]

    private static let lock = NSLock()
    private static var pfLoaded = false
    private static var accumulatedIPs = Set<String>()
    /// Domains of the last successful apply. Re-applying an identical
    /// policy (the engine re-publishes every tick) would re-resolve every
    /// host with dig, reload pf and kill states — minutes of work per
    /// minute. `clear()` resets it so a fresh block always re-applies
    /// and re-kills states. `forceRefresh: true` bypasses this guard for
    /// periodic CDN IP re-resolution.
    private static var lastAppliedDomains: [String]? = nil

    public static func resolveDomains(_ domains: [String]) -> (ips: Set<String>, skipped: Int) {
        var allHostsToResolve: Set<String> = []
        for domain in domains {
            let clean = domain.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if clean.isEmpty { continue }
            let bare = clean.hasPrefix("www.") ? String(clean.dropFirst(4)) : clean
            allHostsToResolve.insert(bare)
            allHostsToResolve.insert("www." + bare)
            for prefix in ["m.", "mobile.", "api.", "app.", "web."] {
                if !bare.hasPrefix(prefix) {
                    allHostsToResolve.insert(prefix + bare)
                }
            }

            if let cdns = cdnDomainMap[bare] {
                for c in cdns {
                    allHostsToResolve.insert(c)
                    if !c.contains(".") || c.split(separator: ".").count == 2 {
                        allHostsToResolve.insert("www." + c)
                    }
                }
            }
        }

        let hosts = Array(allHostsToResolve)
        let collectLock = NSLock()
        var result = Set<String>()
        var skippedCount = 0
        let group = DispatchGroup()
        let queue = DispatchQueue(label: "com.preelock.helper.dns", qos: .userInitiated, attributes: .concurrent)
        let semaphore = DispatchSemaphore(value: 12)

        for host in hosts {
            group.enter()
            queue.async {
                semaphore.wait()
                let resolved = resolveHost(host)
                semaphore.signal()

                collectLock.lock()
                for ip in resolved {
                    if isPrivateOrLoopback(ip) || SharedHosting.contains(ip) {
                        skippedCount += 1
                        continue
                    }
                    result.insert(ip)
                }
                collectLock.unlock()
                group.leave()
            }
        }

        group.wait()
        return (result, skippedCount)
    }

    public static func resolveHost(_ host: String) -> [String] {
        var hints = addrinfo(
            ai_flags: AI_ADDRCONFIG,
            ai_family: AF_UNSPEC,
            ai_socktype: SOCK_STREAM,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: nil,
            ai_addr: nil,
            ai_next: nil
        )
        var res: UnsafeMutablePointer<addrinfo>?
        var out = Set<String>()

        if getaddrinfo(host, nil, &hints, &res) == 0, let first = res {
            var ptr: UnsafeMutablePointer<addrinfo>? = first
            while let current = ptr {
                var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                if getnameinfo(
                    current.pointee.ai_addr,
                    current.pointee.ai_addrlen,
                    &buffer,
                    socklen_t(buffer.count),
                    nil,
                    0,
                    NI_NUMERICHOST
                ) == 0 {
                    let ip = String(cString: buffer)
                    if !isPrivateOrLoopback(ip) && !SharedHosting.contains(ip) {
                        out.insert(ip)
                    }
                }
                ptr = current.pointee.ai_next
            }
            freeaddrinfo(first)
        }

        // Always supplement with direct DNS resolution (system + Cloudflare + Google DNS)
        // so /etc/hosts entries or DoH/VPN resolvers cannot hide active edge PoP IPs.
        for ip in resolveViaDig(host) {
            out.insert(ip)
        }

        return Array(out)
    }

    public static func resolveViaDig(_ host: String) -> [String] {
        var results = Set<String>()
        // Query system resolver first, plus public DoH/DNS edge resolvers to catch
        // alternate edge PoP IPs used by browser Secure DNS (DoH) or VPNs.
        let resolvers: [String?] = [nil, "@1.1.1.1", "@8.8.8.8"]
        for resolver in resolvers {
            for qType in ["A", "AAAA"] {
                var args = ["+short", "+time=1", "+tries=1"]
                if let r = resolver {
                    args.append(r)
                }
                args.append(host)
                args.append(qType)
                let (rc, output) = runCommand("/usr/bin/dig", args)
                if rc == 0 && !output.isEmpty {
                    for line in output.components(separatedBy: .newlines) {
                        let ip = line.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !ip.isEmpty, !ip.contains(" ") else { continue }
                        var sin = sockaddr_in()
                        var sin6 = sockaddr_in6()
                        let isV4 = inet_pton(AF_INET, ip, &sin.sin_addr) == 1
                        let isV6 = inet_pton(AF_INET6, ip, &sin6.sin6_addr) == 1
                        if (isV4 || isV6) && !isPrivateOrLoopback(ip) && !SharedHosting.contains(ip) {
                            results.insert(ip)
                        }
                    }
                }
            }
        }
        return Array(results)
    }

    public static func isPrivateOrLoopback(_ ip: String) -> Bool {
        if ip == "0.0.0.0" || ip == "::" || ip == "127.0.0.1" || ip == "::1" || ip.starts(with: "127.") { return true }
        if ip.starts(with: "10.") || ip.starts(with: "192.168.") { return true }
        if ip.starts(with: "169.254.") || ip.starts(with: "fe80:") { return true }
        if ip.starts(with: "172.") {
            let parts = ip.split(separator: ".")
            if parts.count >= 2, let second = Int(parts[1]), second >= 16 && second <= 31 {
                return true
            }
        }
        return false
    }

    private static func isPFEnabled() -> Bool {
        let (rc, output) = runCommand("/sbin/pfctl", ["-s", "info"])
        return rc == 0 && output.contains("Status: Enabled")
    }

    public static func apply(domains: [String], forceRefresh: Bool = false) {
        lock.lock()
        defer { lock.unlock() }

        if domains.isEmpty {
            clearUnlocked()
            return
        }

        let pfActive = pfLoaded && isPFEnabled()
        if !forceRefresh && lastAppliedDomains == domains && pfActive {
            return
        }

        let (freshIPs, _) = resolveDomains(domains)
        if let prev = lastAppliedDomains, Set(prev).isSubset(of: Set(domains)) {
            accumulatedIPs.formUnion(freshIPs)
        } else {
            accumulatedIPs = freshIPs
        }

        writeAnchorFile(ips: accumulatedIPs)
        ensurePFConfHook()

        let wasLoaded = pfActive
        let args: [String] = wasLoaded
            ? ["-a", anchorName, "-f", anchorFilePath]
            : ["-E", "-f", pfConfPath, "-F", "states"]

        let (rc, output) = runCommand("/sbin/pfctl", args)
        if rc == 0 {
            pfLoaded = true
            lastAppliedDomains = domains
            // Kill active state tables for ALL blocked IPs (outbound and inbound, TCP & UDP/QUIC)
            killStates(for: accumulatedIPs)

            // Verify the kill by inspecting pfctl -s states
            verifyStateKill(for: accumulatedIPs)

            if !wasLoaded {
                // Extract and save enable token
                for line in output.components(separatedBy: "\n") {
                    if let range = line.range(of: "Token : ") {
                        let tok = String(line[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
                        if !tok.isEmpty {
                            try? tok.write(toFile: tokenFilePath, atomically: true, encoding: .utf8)
                            break
                        }
                    }
                }
            }
        }
    }

    /// Kills every existing PF state to or from the blocked IP — TCP,
    /// UDP/QUIC and everything else. On macOS `pfctl -k <host>` only kills
    /// states originating from `<host>`, so outbound browser states require
    /// the two-argument destination kill (`-k 0.0.0.0/0 -k <ip>` for IPv4
    /// and `-k ::/0 -k <ip>` for IPv6).
    private static func killStates(for ips: Set<String>) {
        for ip in ips {
            if ip.contains(":") {
                _ = runCommand("/sbin/pfctl", ["-k", "::/0", "-k", ip])
            } else {
                _ = runCommand("/sbin/pfctl", ["-k", "0.0.0.0/0", "-k", ip])
            }
            _ = runCommand("/sbin/pfctl", ["-k", ip])
        }
    }

    private static func verifyStateKill(for ips: Set<String>) {
        let (rc, statesOut) = runCommand("/sbin/pfctl", ["-s", "states"])
        guard rc == 0, !statesOut.isEmpty else { return }

        for ip in ips {
            if statesOut.contains(ip) {
                print("[WARN] State for blocked IP \(ip) still active in PF table. Repeating kill pass...")
                if ip.contains(":") {
                    _ = runCommand("/sbin/pfctl", ["-k", "::/0", "-k", ip])
                    _ = runCommand("/sbin/pfctl", ["-k", ip])
                } else {
                    _ = runCommand("/sbin/pfctl", ["-k", "0.0.0.0/0", "-k", ip])
                    _ = runCommand("/sbin/pfctl", ["-k", ip])
                }
            }
        }
    }

    public static func writeAnchorFile(ips: Set<String>) {
        try? FileManager.default.createDirectory(atPath: anchorDir, withIntermediateDirectories: true)
        var rules = "# Preelock packet filter rules\n"
        for ip in ips.sorted() {
            rules += "block return out quick proto tcp from any to \(ip)\n"
            rules += "block return out quick proto udp from any to \(ip)\n"
            rules += "block drop in quick proto tcp from \(ip) to any\n"
            rules += "block drop in quick proto udp from \(ip) to any\n"
        }
        try? rules.write(toFile: anchorFilePath, atomically: true, encoding: .utf8)
    }

    public static func ensurePFConfHook() {
        guard let conf = try? String(contentsOfFile: pfConfPath, encoding: .utf8) else { return }
        if conf.contains(pfConfStart) && conf.contains(pfConfEnd) { return }

        var newConf = conf
        if !newConf.hasSuffix("\n") { newConf += "\n" }
        newConf += """

        \(pfConfStart)
        anchor "\(anchorName)"
        load anchor "\(anchorName)" from "\(anchorFilePath)"
        \(pfConfEnd)

        """
        try? newConf.write(toFile: pfConfPath, atomically: true, encoding: .utf8)
    }

    public static func stripPFConfHook() {
        guard let conf = try? String(contentsOfFile: pfConfPath, encoding: .utf8) else { return }
        let lines = conf.components(separatedBy: "\n")
        var skip = false
        var kept: [String] = []

        for line in lines {
            if line.trimmingCharacters(in: .whitespaces) == pfConfStart {
                skip = true
                continue
            }
            if line.trimmingCharacters(in: .whitespaces) == pfConfEnd {
                skip = false
                continue
            }
            if !skip {
                kept.append(line)
            }
        }

        var rebuilt = kept.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        if !rebuilt.isEmpty { rebuilt += "\n" }
        try? rebuilt.write(toFile: pfConfPath, atomically: true, encoding: .utf8)
    }

    public static func clear() {
        lock.lock()
        defer { lock.unlock() }
        clearUnlocked()
    }

    private static func clearUnlocked() {
        lastAppliedDomains = nil

        // Flush live anchor rules, tables, and states completely
        _ = runCommand("/sbin/pfctl", ["-a", anchorName, "-F", "all"])

        // Release enable token if held
        if let token = try? String(contentsOfFile: tokenFilePath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
           !token.isEmpty, token.allSatisfy({ $0.isNumber }) {
            _ = runCommand("/sbin/pfctl", ["-X", token])
        }

        try? "".write(toFile: anchorFilePath, atomically: true, encoding: .utf8)
        stripPFConfHook()
        try? FileManager.default.removeItem(atPath: tokenFilePath)
        try? FileManager.default.removeItem(atPath: anchorFilePath)

        accumulatedIPs.removeAll()
        pfLoaded = false
    }

    private static func runCommand(_ path: String, _ args: [String]) -> (Int32, String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            let out = String(data: data, encoding: .utf8) ?? ""
            return (process.terminationStatus, out)
        } catch {
            return (-1, error.localizedDescription)
        }
    }
}

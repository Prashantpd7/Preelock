import Foundation

public enum HostsManager {
    public static let hostsPath = "/etc/hosts"
    public static let backupPath = "/etc/hosts.preelock.bak"
    public static let startMarker = "# === PREELOCK BLOCKLIST START ==="
    public static let endMarker = "# === PREELOCK BLOCKLIST END ==="

    public static func apply(domains: [String]) throws {
        let content = (try? String(contentsOfFile: hostsPath, encoding: .utf8)) ?? ""

        // Create pristine backup if none exists
        if !FileManager.default.fileExists(atPath: backupPath) {
            let clean = stripMarkers(from: content)
            try? clean.write(toFile: backupPath, atomically: true, encoding: .utf8)
        }

        let updated = inject(domains: domains, into: content)
        try updated.write(toFile: hostsPath, atomically: true, encoding: .utf8)
        flushDNS()
    }

    public static func clear() throws {
        guard let content = try? String(contentsOfFile: hostsPath, encoding: .utf8) else { return }
        let stripped = stripMarkers(from: content)
        try stripped.write(toFile: hostsPath, atomically: true, encoding: .utf8)
        flushDNS()
    }

    public static func inject(domains: [String], into content: String) -> String {
        let clean = stripMarkers(from: content)
        let block = buildBlock(domains: domains)
        if block.isEmpty {
            return clean
        }
        var res = clean
        if !res.isEmpty && !res.hasSuffix("\n") {
            res += "\n"
        }
        return res + block
    }

    public static let serviceSubdomains: [String: [String]] = [
        "instagram.com": [
            "i.instagram.com", "graph.instagram.com", "api.instagram.com",
            "m.instagram.com", "l.instagram.com", "edge-chat.instagram.com",
            "cdninstagram.com", "static.cdninstagram.com", "scontent.cdninstagram.com"
        ],
        "facebook.com": [
            "m.facebook.com", "web.facebook.com", "l.facebook.com",
            "graph.facebook.com", "edge-mqtt.facebook.com", "mbasic.facebook.com"
        ],
        "youtube.com": [
            "m.youtube.com", "music.youtube.com", "studio.youtube.com", "youtu.be"
        ],
        "reddit.com": [
            "old.reddit.com", "new.reddit.com", "sh.reddit.com",
            "oauth.reddit.com", "gateway.reddit.com", "gql.reddit.com", "redd.it"
        ],
        "x.com": ["api.x.com", "mobile.x.com", "pro.x.com"],
        "twitter.com": ["api.twitter.com", "mobile.twitter.com"],
        "tiktok.com": ["m.tiktok.com", "vm.tiktok.com"],
        "whatsapp.com": ["web.whatsapp.com"],
        "discord.com": ["ptb.discord.com", "canary.discord.com"]
    ]

    private static let canaryDomains: [String] = [
        "mask.icloud.com",
        "mask-h2.icloud.com",
        "use-application-dns.net"
    ]

    private static let nonWwwPrefixes: Set<String> = [
        "m.", "i.", "l.", "api.", "app.", "web.", "old.", "new.", "sh.",
        "oauth.", "gateway.", "gql.", "static.", "scontent.", "graph.",
        "edge-chat.", "edge-mqtt.", "music.", "studio.", "mobile.",
        "ptb.", "canary.", "vm.", "mbasic.", "pro.", "mask.", "mask-h2."
    ]

    public static func expandDomains(allDomains: [String], userDomains: [String]) -> [String] {
        var expanded = Set(
            allDomains
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { isValidDomain($0) }
        )

        if expanded.isEmpty {
            return []
        }

        for raw in userDomains {
            let clean = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard isValidDomain(clean) else { continue }
            let bare = clean.hasPrefix("www.") ? String(clean.dropFirst(4)) : clean
            expanded.insert(bare)
            expanded.insert("www.\(bare)")

            if let subs = serviceSubdomains[bare] {
                for s in subs {
                    expanded.insert(s)
                }
            }
            for prefix in ["m.", "mobile.", "api.", "app.", "web."] {
                if !bare.hasPrefix(prefix) && bare != "use-application-dns.net" {
                    expanded.insert("\(prefix)\(bare)")
                }
            }
        }

        for c in canaryDomains {
            expanded.insert(c)
        }

        return Array(expanded).sorted()
    }

    public static func buildBlock(domains: [String]) -> String {
        let safeDomains = domains
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { isValidDomain($0) }

        let uniqueDomains = Array(Set(safeDomains)).sorted()
        if uniqueDomains.isEmpty {
            return ""
        }

        var lines: [String] = []
        lines.append(startMarker)
        lines.append("# Managed by Preelock. Do not edit this section manually.")

        for d in uniqueDomains {
            let bare = d.hasPrefix("www.") ? String(d.dropFirst(4)) : d
            lines.append("0.0.0.0 \(bare)")
            lines.append(":: \(bare)")
            let skipWww = bare == "use-application-dns.net"
                || nonWwwPrefixes.contains(where: { bare.hasPrefix($0) })
            if !skipWww {
                lines.append("0.0.0.0 www.\(bare)")
                lines.append(":: www.\(bare)")
            }
        }

        lines.append(endMarker)
        return lines.joined(separator: "\n") + "\n"
    }

    public static func stripMarkers(from content: String) -> String {
        let lines = content.components(separatedBy: "\n")
        var skipRanges: [ClosedRange<Int>] = []
        var pendingStart: Int?

        for (i, line) in lines.enumerated() {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed == startMarker {
                pendingStart = i
            } else if trimmed == endMarker, let s = pendingStart {
                skipRanges.append(s...i)
                pendingStart = nil
            }
        }

        // If markers are mismatched, refuse to truncate
        if pendingStart != nil {
            return content
        }

        var kept: [String] = []
        for (i, line) in lines.enumerated() {
            if skipRanges.contains(where: { $0.contains(i) }) { continue }
            kept.append(line)
        }

        while kept.last?.isEmpty == true {
            kept.removeLast()
        }
        return kept.joined(separator: "\n") + (content.hasSuffix("\n") || !kept.isEmpty ? "\n" : "")
    }

    public static func flushDNS() {
        let p1 = Process()
        p1.executableURL = URL(fileURLWithPath: "/usr/bin/dscacheutil")
        p1.arguments = ["-flushcache"]
        try? p1.run()
        p1.waitUntilExit()

        let p2 = Process()
        p2.executableURL = URL(fileURLWithPath: "/usr/bin/killall")
        p2.arguments = ["-HUP", "mDNSResponder"]
        try? p2.run()
        p2.waitUntilExit()
    }

    private static func isValidDomain(_ d: String) -> Bool {
        if d.isEmpty || d.count > 253 || d.contains(" ") || d.contains("/") || d.contains(":") {
            return false
        }
        return !d.hasPrefix(".") && !d.hasSuffix(".")
    }
}

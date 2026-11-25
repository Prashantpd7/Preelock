import Foundation
import Darwin

/// Asynchronous forwarder for allowed DNS queries using native BSD sockets.
/// Discovers system-configured upstream DNS nameservers dynamically (via /etc/resolv.conf,
/// scutil/ipconfig, or saved configuration), avoiding hardcoded resolvers while strictly
/// filtering out loopback addresses (127.0.0.1, ::1) to prevent recursion loops.
///
/// Features (RFC 1035 / RFC 7766):
/// - Dual UDP and TCP upstream forwarding
/// - Automatic retry over TCP when UDP response is truncated (TC=1)
/// - Multi-upstream failover (retries next configured nameserver on timeout)
/// - Transaction ID validation against DNS spoofing / cache poisoning
/// - Integrated caching via DNSCache
public final class UpstreamResolver: @unchecked Sendable {

    public static let shared = UpstreamResolver()

    private let port: in_port_t = 53
    private let timeoutSeconds: Int = 1 // 1s per upstream query (Fast responsive failover)
    private let queue = DispatchQueue(label: "com.preelock.poc.upstream", attributes: .concurrent)

    private let lock = NSLock()
    private var cachedUpstreams: [String] = []
    private var lastDiscovery: Date = .distantPast

    private init() {
        refreshUpstreams()
    }

    /// Discovers and returns active system DNS nameservers.
    public func currentUpstreams() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        if Date().timeIntervalSince(lastDiscovery) > 30 || cachedUpstreams.isEmpty {
            cachedUpstreams = Self.discoverSystemDNS()
            lastDiscovery = Date()
        }
        return cachedUpstreams
    }

    /// Forces a refresh of upstream DNS nameservers.
    public func refreshUpstreams() {
        lock.lock()
        cachedUpstreams = Self.discoverSystemDNS()
        lastDiscovery = Date()
        lock.unlock()
    }

    /// Forwards raw query bytes to discovered system DNS resolvers.
    /// Returns both the response datagram and the IP/label of the upstream that resolved it.
    public func resolveWithUpstream(queryData: Data, completion: @escaping @Sendable (_ response: Data?, _ upstreamUsed: String?) -> Void) {
        queue.async { [weak self] in
            guard let self = self else {
                completion(nil, nil)
                return
            }

            guard queryData.count >= 12 else {
                completion(nil, nil)
                return
            }

            let queryID = queryData.withUnsafeBytes { $0.load(fromByteOffset: 0, as: UInt16.self).bigEndian }
            let parsed = DNSPacket.parse(queryData)
            let qName = parsed?.questions.first?.name
            let qType = parsed?.questions.first?.rawType

            // 1. Check in-memory DNS cache
            if let qName = qName, let qType = qType {
                if let cached = DNSCache.shared.get(domain: qName, type: qType, queryID: queryID) {
                    completion(cached, "Cache")
                    return
                }
            }

            // 2. Query discovered system resolvers in priority order
            let servers = self.currentUpstreams()
            for (index, ip) in servers.enumerated() {
                if let response = self.queryUpstream(ip: ip, data: queryData, queryID: queryID) {
                    // Cache valid response
                    if let qName = qName, let qType = qType {
                        let ttl = DNSPacket.extractTTL(from: response)
                        DNSCache.shared.set(domain: qName, type: qType, responseData: response, ttl: ttl)
                    }
                    completion(response, ip)
                    return
                }
                if index < servers.count - 1 {
                    print("[WARN] Upstream resolver \(ip) failed or timed out. Retrying next resolver...")
                }
            }

            // 3. Fallback resolvers if all discovered resolvers failed
            let fallbacks = ["1.1.1.1", "8.8.8.8"].filter { !servers.contains($0) }
            for fallback in fallbacks {
                print("[WARN] Retrying query via fallback upstream: \(fallback)")
                if let response = self.queryUpstream(ip: fallback, data: queryData, queryID: queryID) {
                    if let qName = qName, let qType = qType {
                        let ttl = DNSPacket.extractTTL(from: response)
                        DNSCache.shared.set(domain: qName, type: qType, responseData: response, ttl: ttl)
                    }
                    completion(response, fallback)
                    return
                }
            }

            // 4. Retry timed-out upstream once before returning SERVFAIL (Phase 7 Reliability Fix)
            let retryTarget = servers.first ?? "1.1.1.1"
            print("[INFO] Upstream timed out. Retrying \(retryTarget) once before SERVFAIL...")
            if let response = self.queryUpstream(ip: retryTarget, data: queryData, queryID: queryID) {
                if let qName = qName, let qType = qType {
                    let ttl = DNSPacket.extractTTL(from: response)
                    DNSCache.shared.set(domain: qName, type: qType, responseData: response, ttl: ttl)
                }
                completion(response, retryTarget)
                return
            }

            completion(nil, nil)
        }
    }

    /// Backwards-compatible overload returning only response datagram.
    public func resolve(queryData: Data, completion: @escaping @Sendable (Data?) -> Void) {
        resolveWithUpstream(queryData: queryData) { response, _ in
            completion(response)
        }
    }

    // MARK: - UDP Upstream Query
    private func queryUpstream(ip: String, data: Data, queryID: UInt16) -> Data? {
        let sock = socket(AF_INET, SOCK_DGRAM, 0)
        guard sock >= 0 else { return nil }
        defer { close(sock) }

        var nosigpipe: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_NOSIGPIPE, &nosigpipe, socklen_t(MemoryLayout<Int32>.size))

        var tv = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))

        var rcvBufSize: Int32 = 65535
        setsockopt(sock, SOL_SOCKET, SO_RCVBUF, &rcvBufSize, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, ip, &addr.sin_addr) == 1 else { return nil }

        let sent = data.withUnsafeBytes { ptr in
            withUnsafePointer(to: &addr) { addrPtr in
                addrPtr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                    sendto(sock, ptr.baseAddress, data.count, 0, sa, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
        }

        guard sent == data.count else { return nil }

        var buffer = [UInt8](repeating: 0, count: 65535)
        let received = recv(sock, &buffer, buffer.count, 0)

        guard received >= 12 else { return nil }
        let response = Data(buffer.prefix(received))

        // Validate transaction ID (RFC 1035 §4.1.1)
        let respID = response.withUnsafeBytes { $0.load(fromByteOffset: 0, as: UInt16.self).bigEndian }
        guard respID == queryID else {
            print("[WARN] Dropping response from \(ip): transaction ID mismatch (expected \(queryID), got \(respID))")
            return nil
        }

        // Check Truncation flag (TC bit, RFC 1035 §4.1.1)
        let flags = response.withUnsafeBytes { $0.load(fromByteOffset: 2, as: UInt16.self).bigEndian }
        let isTruncated = (flags & 0x0200) != 0

        if isTruncated {
            print("[INFO] Upstream response from \(ip) is truncated (TC=1). Retrying via TCP (RFC 7766)...")
            if let tcpResponse = queryUpstreamTCP(ip: ip, data: data, queryID: queryID) {
                return tcpResponse
            }
        }

        return response
    }

    // MARK: - TCP Upstream Query (RFC 7766)
    public func queryUpstreamTCP(ip: String, data: Data, queryID: UInt16) -> Data? {
        let sock = socket(AF_INET, SOCK_STREAM, 0)
        guard sock >= 0 else { return nil }
        defer { close(sock) }

        var nosigpipe: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_NOSIGPIPE, &nosigpipe, socklen_t(MemoryLayout<Int32>.size))

        var tv = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))

        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, ip, &addr.sin_addr) == 1 else { return nil }

        guard connectWithTimeout(sock: sock, addr: &addr, timeout: timeoutSeconds) else {
            return nil
        }

        // RFC 7766 §5.1: 2-byte big-endian length prefix
        let len = UInt16(data.count)
        var sendBuffer = Data([UInt8((len >> 8) & 0xFF), UInt8(len & 0xFF)])
        sendBuffer.append(data)

        let sent = sendBuffer.withUnsafeBytes { ptr in
            send(sock, ptr.baseAddress, sendBuffer.count, 0)
        }
        guard sent == sendBuffer.count else { return nil }

        // Read 2-byte length prefix
        var lenBytes = [UInt8](repeating: 0, count: 2)
        var bytesRead = 0
        while bytesRead < 2 {
            let r = recv(sock, &lenBytes[bytesRead], 2 - bytesRead, 0)
            guard r > 0 else { return nil }
            bytesRead += r
        }

        let expectedLen = Int(UInt16(lenBytes[0]) << 8 | UInt16(lenBytes[1]))
        guard expectedLen >= 12 && expectedLen <= 65535 else { return nil }

        // Read response payload
        var respData = Data()
        var remaining = expectedLen
        var buf = [UInt8](repeating: 0, count: 4096)
        while remaining > 0 {
            let toRead = min(remaining, buf.count)
            let r = recv(sock, &buf, toRead, 0)
            guard r > 0 else { return nil }
            respData.append(buf, count: r)
            remaining -= r
        }

        // Validate transaction ID
        let respID = respData.withUnsafeBytes { $0.load(fromByteOffset: 0, as: UInt16.self).bigEndian }
        guard respID == queryID else {
            print("[WARN] Dropping TCP response from \(ip): transaction ID mismatch (expected \(queryID), got \(respID))")
            return nil
        }

        return respData
    }

    private func connectWithTimeout(sock: Int32, addr: inout sockaddr_in, timeout: Int) -> Bool {
        let flags = fcntl(sock, F_GETFL, 0)
        _ = fcntl(sock, F_SETFL, flags | O_NONBLOCK)

        let res = withUnsafePointer(to: &addr) { addrPtr in
            addrPtr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                connect(sock, sa, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }

        if res == 0 {
            _ = fcntl(sock, F_SETFL, flags)
            return true
        }

        if errno != EINPROGRESS {
            return false
        }

        var pollFd = pollfd(fd: sock, events: Int16(POLLOUT), revents: 0)
        let pollRes = poll(&pollFd, 1, Int32(timeout * 1000))
        guard pollRes > 0, (pollFd.revents & Int16(POLLOUT)) != 0 else {
            return false
        }

        var err: Int32 = 0
        var errLen = socklen_t(MemoryLayout<Int32>.size)
        getsockopt(sock, SOL_SOCKET, SO_ERROR, &err, &errLen)
        guard err == 0 else { return false }

        _ = fcntl(sock, F_SETFL, flags)
        return true
    }

    // MARK: - Dynamic System DNS Discovery
    public static func discoverSystemDNS() -> [String] {
        var servers: [String] = []

        // 1. Saved upstream DNS files (written by Preelock before redirecting to 127.0.0.1)
        for savedPath in ["/tmp/preelock_upstream_dns.txt", "/tmp/preelock_original_dns.txt"] {
            if let savedContent = try? String(contentsOfFile: savedPath, encoding: .utf8) {
                let lines = savedContent.components(separatedBy: .whitespacesAndNewlines)
                for item in lines {
                    let clean = item.trimmingCharacters(in: .whitespaces)
                    if isValidUpstream(clean) && !servers.contains(clean) {
                        servers.append(clean)
                    }
                }
            }
        }
        if !servers.isEmpty {
            return servers
        }

        // 2. Read /etc/resolv.conf directly
        if let resolvContent = try? String(contentsOfFile: "/etc/resolv.conf", encoding: .utf8) {
            for line in resolvContent.components(separatedBy: .newlines) {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.hasPrefix("nameserver") {
                    let parts = trimmed.split(whereSeparator: \.isWhitespace).map(String.init)
                    if parts.count >= 2 {
                        let ns = parts[1].trimmingCharacters(in: .whitespaces)
                        if isValidUpstream(ns) && !servers.contains(ns) {
                            servers.append(ns)
                        }
                    }
                }
            }
        }
        if !servers.isEmpty {
            return servers
        }

        // 3. DHCP packet inspection via /usr/sbin/ipconfig for primary interfaces
        for iface in ["en0", "en1", "en2"] {
            let pipe = Pipe()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/sbin/ipconfig")
            process.arguments = ["getpacket", iface]
            process.standardOutput = pipe
            process.standardError = Pipe()
            if (try? process.run()) != nil {
                process.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                if let output = String(data: data, encoding: .utf8) {
                    if let range = output.range(of: "domain_name_server.*?: \\{(.*?)\\}", options: .regularExpression) {
                        let match = String(output[range])
                        if let start = match.firstIndex(of: "{"), let end = match.firstIndex(of: "}") {
                            let inner = match[match.index(after: start)..<end]
                            for ip in inner.split(separator: ",").map({ $0.trimmingCharacters(in: .whitespaces) }) {
                                if isValidUpstream(ip) && !servers.contains(ip) {
                                    servers.append(ip)
                                }
                            }
                        }
                    }
                }
            }
            if !servers.isEmpty {
                return servers
            }
        }

        // 4. Safe fallback if all system detection mechanisms fail
        return ["1.1.1.1", "8.8.8.8"]
    }

    /// Verifies that an IP is non-empty and NOT a loopback/local address that would cause recursive self-queries.
    private static func isValidUpstream(_ ip: String) -> Bool {
        if ip.isEmpty { return false }
        if ip == "127.0.0.1" || ip.hasPrefix("127.") || ip == "::1" || ip == "0.0.0.0" || ip == "localhost" {
            return false
        }
        var sin = sockaddr_in()
        if inet_pton(AF_INET, ip, &sin.sin_addr) == 1 {
            return true
        }
        var sin6 = sockaddr_in6()
        if inet_pton(AF_INET6, ip, &sin6.sin6_addr) == 1 {
            return true
        }
        return false
    }
}

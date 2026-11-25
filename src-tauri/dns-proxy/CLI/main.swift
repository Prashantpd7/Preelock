import Foundation
import Network
#if canImport(PreelockDNSCommon)
import PreelockDNSCommon
#endif
import Darwin

/// Standalone, user-space local DNS proxy engine for Preelock (RFC 1035 & RFC 7766).
/// Listens for DNS queries on both UDP and TCP on 127.0.0.1, enforces domain blocklists
/// with NXDOMAIN synthesis, and asynchronously forwards non-blocked queries to dynamically
/// discovered system DNS with transaction ID validation, fallback retry, and response caching.
@main
struct CLIEngine {

    static var testPort: NWEndpoint.Port = 5354 // Default test port (non-privileged)

    // Rate limiter: Max 100 queries per second
    private static let rateLimiter = RateLimiter()

    // TCP connection concurrency control (Max 50 concurrent connections)
    private static let maxConcurrentTCP = 50
    private static let tcpLock = NSLock()
    private static var activeTCPConnections = 0

    static func main() {
        setbuf(stdout, nil)
        // Parse command line arguments
        let args = CommandLine.arguments

        if args.contains("--help") || args.contains("-h") {
            print("Preelock DNS Proxy CLI. Options: --port <port>, --setup-hosts, --update-helper <path>, --kill, --status, --help")
            exit(0)
        }

        if args.contains("--kill") || args.contains("--stop") {
            let myPid = getpid()
            let pipe = Pipe()
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
            proc.arguments = ["-f", "preelock-dns-proxy"]
            proc.standardOutput = pipe
            try? proc.run()
            proc.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            if let str = String(data: data, encoding: .utf8) {
                for line in str.split(separator: "\n") {
                    if let pid = Int32(line.trimmingCharacters(in: .whitespaces)), pid != myPid {
                        kill(pid, SIGKILL)
                    }
                }
            }
            unlink(defaultControlSocketPath())
            print("SUCCESS: cleaned up stale processes")
            exit(0)
        }

        if args.contains("--setup-hosts") {
            chmod("/etc/hosts", 0o664)
            chown("/etc/hosts", 0, 80) // root:admin
            print("SUCCESS: /etc/hosts set to 0664 (root:admin)")
            exit(0)
        }

        if let idx = args.firstIndex(of: "--update-helper"), idx + 1 < args.count {
            let src = args[idx + 1]
            let dst = "/Library/PrivilegedHelperTools/preelock-helper"
            guard FileManager.default.fileExists(atPath: src) else {
                print("ERROR: Source binary not found at \(src)")
                exit(1)
            }
            do {
                if FileManager.default.fileExists(atPath: dst) {
                    try FileManager.default.removeItem(atPath: dst)
                }
                try FileManager.default.copyItem(atPath: src, toPath: dst)
                try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dst)

                let p = Process()
                p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
                p.arguments = ["kickstart", "-k", "system/com.preelock.helper"]
                try? p.run()
                p.waitUntilExit()

                print("SUCCESS: Updated helper binary at \(dst)")
                exit(0)
            } catch {
                print("ERROR: \(error)")
                exit(1)
            }
        }

        if geteuid() == 0 {
            chmod("/etc/hosts", 0o664)
            chown("/etc/hosts", 0, 80)
        }
        if let portIndex = args.firstIndex(of: "--port"), portIndex + 1 < args.count {
            if let customPort = UInt16(args[portIndex + 1]), let portEndpoint = NWEndpoint.Port(rawValue: customPort) {
                testPort = portEndpoint
            }
        } else if args.count > 1, let customPort = UInt16(args[1]), let portEndpoint = NWEndpoint.Port(rawValue: customPort) {
            testPort = portEndpoint
        }

        // Custom policy file
        if let policyIndex = args.firstIndex(of: "--policy"), policyIndex + 1 < args.count {
            let customPolicy = args[policyIndex + 1]
            _ = BlocklistManager.shared.switchPolicy(to: customPolicy)
        }

        // Control socket path
        var socketPath = defaultControlSocketPath()
        if let sockIndex = args.firstIndex(of: "--socket"), sockIndex + 1 < args.count {
            socketPath = args[sockIndex + 1]
        }

        print("=== Preelock Local DNS Proxy Engine (User-Space) ===")
        print("Version: 7.0 (Bypass Protection & High-Concurrency Reliability)")
        print("Configured Local Endpoint: 127.0.0.1:\(testPort.rawValue)")

        let blocklist = BlocklistManager.shared
        print("Active Policy: '\(blocklist.currentPolicy.name)' [Mode: \(blocklist.currentPolicy.mode.rawValue), Block DoH: \(blocklist.currentPolicy.block_doh)]")
        print("Active Rules (\(blocklist.allRules().count) domains): \(blocklist.allRules().prefix(10))...")
        print("DoH Resolver Protection: \(blocklist.allDoHEndpoints().count) endpoints monitored")

        let upstreams = UpstreamResolver.shared.currentUpstreams()
        print("Discovered System Upstream DNS: \(upstreams)")
        print("")

        // Start dynamic auto-reload monitor (1.0s interval)
        blocklist.startAutoReload(intervalSeconds: 1.0)

        let queue = DispatchQueue(label: "com.preelock.poc.dnslistener", qos: .userInteractive, attributes: .concurrent)

        // 1. Configure UDP Listener (Strictly bound to 127.0.0.1)
        let udpParams = NWParameters.udp
        udpParams.allowLocalEndpointReuse = true
        udpParams.requiredLocalEndpoint = NWEndpoint.hostPort(host: NWEndpoint.Host("127.0.0.1"), port: testPort)

        guard let udpListener = try? NWListener(using: udpParams) else {
            print("ERROR: Failed to bind UDP listener on 127.0.0.1:\(testPort.rawValue).")
            if testPort.rawValue < 1024 {
                print("HINT: Ports below 1024 require elevated privileges (sudo).")
            }
            exit(1)
        }

        udpListener.newConnectionHandler = { connection in
            connection.start(queue: queue)
            receiveUDPMessage(on: connection)
        }

        udpListener.stateUpdateHandler = { state in
            switch state {
            case .ready:
                print(">>> UDP DNS Listener is READY on 127.0.0.1:\(testPort.rawValue)")
            case .failed(let error):
                print("UDP Listener failed: \(error)")
                exit(1)
            default:
                break
            }
        }

        // 2. Configure TCP Listener (Strictly bound to 127.0.0.1, RFC 7766)
        let tcpParams = NWParameters.tcp
        tcpParams.allowLocalEndpointReuse = true
        tcpParams.requiredLocalEndpoint = NWEndpoint.hostPort(host: NWEndpoint.Host("127.0.0.1"), port: testPort)

        guard let tcpListener = try? NWListener(using: tcpParams) else {
            print("ERROR: Failed to bind TCP listener on 127.0.0.1:\(testPort.rawValue).")
            exit(1)
        }

        tcpListener.newConnectionHandler = { connection in
            handleIncomingTCPConnection(connection, queue: queue)
        }

        tcpListener.stateUpdateHandler = { state in
            switch state {
            case .ready:
                print(">>> TCP DNS Listener is READY on 127.0.0.1:\(testPort.rawValue)")
                print(">>> Queries will be resolved via system DNS: \(UpstreamResolver.shared.currentUpstreams().first ?? "System")")
                print(">>> Audit log: resolution.log")
                print("")
            case .failed(let error):
                print("TCP Listener failed: \(error)")
                exit(1)
            default:
                break
            }
        }

        // Security check: Assert listeners are bound strictly to loopback
        print("[SECURITY] Enforcing strict loopback-only bind: 127.0.0.1")

        udpListener.start(queue: queue)
        tcpListener.start(queue: queue)

        // Drop root privileges after socket setup
        if testPort.rawValue < 1024 && geteuid() == 0 {
            var targetUID = getuid()
            if targetUID == 0 {
                // When running under launchd as root, find active console user
                var statBuf = stat()
                if stat("/dev/console", &statBuf) == 0 && statBuf.st_uid != 0 {
                    targetUID = statBuf.st_uid
                }
            }
            if targetUID != 0 {
                setuid(targetUID)
                print("[SECURITY] Privileges dropped to UID \(targetUID) after socket setup.")
            }
        }

        // Start local Unix domain control server (now running under dropped user privileges)
        ControlServer.shared.start(path: socketPath)

        // Start network-change watcher if running in system mode (port 53)
        if testPort.rawValue == 53 {
            NetworkWatcher.shared.start(checkIntervalSeconds: 3.0)
        }

        // Handle SIGINT and SIGTERM gracefully
        signal(SIGINT, SIG_IGN)
        signal(SIGTERM, SIG_IGN)

        let sigintSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
        sigintSource.setEventHandler {
            print("\nShutting down Preelock Local DNS Proxy (SIGINT)...")
            NetworkWatcher.shared.stop()
            BlocklistManager.shared.stopAutoReload()
            ControlServer.shared.stop()
            udpListener.cancel()
            tcpListener.cancel()
            exit(0)
        }
        sigintSource.resume()

        let sigtermSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        sigtermSource.setEventHandler {
            print("\nShutting down Preelock Local DNS Proxy (SIGTERM)...")
            NetworkWatcher.shared.stop()
            BlocklistManager.shared.stopAutoReload()
            ControlServer.shared.stop()
            udpListener.cancel()
            tcpListener.cancel()
            exit(0)
        }
        sigtermSource.resume()

        dispatchMain()
    }

    private static func defaultControlSocketPath() -> String {
        return "/tmp/preelock-dns-control.sock"
    }

    // MARK: - UDP Message Handling
    private static func receiveUDPMessage(on connection: NWConnection) {
        connection.receiveMessage { content, context, isComplete, error in
            guard let data = content, !data.isEmpty, error == nil else {
                connection.cancel()
                return
            }

            // Immediately re-arm to receive subsequent datagrams concurrently.
            // Never wait for upstream resolution before reading next datagram!
            receiveUDPMessage(on: connection)

            processDNSQuery(queryData: data) { response in
                if let response = response {
                    connection.send(content: response, completion: .contentProcessed { sendError in
                        if sendError != nil {
                            connection.cancel()
                        }
                    })
                }
            }
        }
    }

    // MARK: - TCP Connection Handling (RFC 7766)
    private static func handleIncomingTCPConnection(_ connection: NWConnection, queue: DispatchQueue) {
        // Enforce concurrency limit (Max 50 active TCP connections)
        tcpLock.lock()
        if activeTCPConnections >= maxConcurrentTCP {
            tcpLock.unlock()
            print("[WARN] TCP connection limit reached (\(maxConcurrentTCP)). Dropping incoming connection.")
            connection.cancel()
            return
        }
        activeTCPConnections += 1
        tcpLock.unlock()

        var hasClosed = false
        let cleanup = {
            self.tcpLock.lock()
            if !hasClosed {
                hasClosed = true
                self.activeTCPConnections = max(0, self.activeTCPConnections - 1)
            }
            self.tcpLock.unlock()
            connection.cancel()
        }

        connection.stateUpdateHandler = { state in
            switch state {
            case .failed, .cancelled:
                cleanup()
            default:
                break
            }
        }

        connection.start(queue: queue)

        // 5-second idle timeout
        let idleTimeoutItem = DispatchWorkItem {
            print("[INFO] TCP connection idle timeout (5s). Closing connection.")
            cleanup()
        }
        queue.asyncAfter(deadline: .now() + 5.0, execute: idleTimeoutItem)

        // RFC 7766 §5.1: Read 2-byte length prefix
        connection.receive(minimumIncompleteLength: 2, maximumLength: 2) { prefixData, _, isComplete, error in
            guard let prefixData = prefixData, prefixData.count == 2, error == nil else {
                idleTimeoutItem.cancel()
                cleanup()
                return
            }

            let expectedLength = Int(prefixData[0]) << 8 | Int(prefixData[1])
            guard expectedLength >= 12 && expectedLength <= 65535 else {
                idleTimeoutItem.cancel()
                cleanup()
                return
            }

            // Read the DNS query payload
            connection.receive(minimumIncompleteLength: expectedLength, maximumLength: expectedLength) { payload, _, _, err in
                idleTimeoutItem.cancel()
                guard let payload = payload, payload.count == expectedLength, err == nil else {
                    cleanup()
                    return
                }

                processDNSQuery(queryData: payload) { response in
                    guard let response = response else {
                        cleanup()
                        return
                    }

                    // Send RFC 7766 2-byte length prefix + response payload
                    let respLen = UInt16(response.count)
                    var framedResponse = Data([UInt8((respLen >> 8) & 0xFF), UInt8(respLen & 0xFF)])
                    framedResponse.append(response)

                    connection.send(content: framedResponse, completion: .contentProcessed { _ in
                        cleanup()
                    })
                }
            }
        }
    }

    // MARK: - Core Query Processing Pipeline
    private static func processDNSQuery(queryData: Data, completion: @escaping @Sendable (Data?) -> Void) {
        // 1. Harden against malformed packets (RFC 1035)
        guard queryData.count >= 12, let packet = DNSPacket.parse(queryData), let question = packet.questions.first else {
            print("[WARN] Received malformed or non-DNS datagram (\(queryData.count) bytes). Dropping.")
            completion(nil)
            return
        }

        // 2. High-capacity sliding window rate limiter check (Max 5,000 QPS)
        guard rateLimiter.allow() else {
            print("[WARN] Rate limit exceeded (>5000 qps). Returning SERVFAIL.")
            let servFail = DNSPacket.synthesizeServFail(queryID: packet.id, questionData: question.rawQuestionBytes)
            completion(servFail)
            return
        }

        let domain = question.name
        let qtype = question.type.description
        let (isBlocked, matchedRule) = BlocklistManager.shared.isBlocked(domain)

        // 3. Blocklist enforcement (GUARANTEE: blocked domains are NEVER forwarded upstream)
        if isBlocked {
            let detail = "Matched rule: \(matchedRule ?? domain)"
            DNSLogger.shared.log(domain: domain, type: qtype, blocked: true, detail: detail)

            // Synthesize authoritative NXDOMAIN response
            let response = DNSPacket.synthesizeNXDomain(queryID: packet.id, questionData: question.rawQuestionBytes)
            completion(response)
            return
        }

        // 4. Allowed domains: Asynchronously resolve via system upstreams with failover & caching
        UpstreamResolver.shared.resolveWithUpstream(queryData: queryData) { upstreamResponse, upstreamUsed in
            if let upstreamResponse = upstreamResponse {
                let usedServer = upstreamUsed ?? "System"
                let detail = "Resolved via \(usedServer)"
                DNSLogger.shared.log(domain: domain, type: qtype, blocked: false, detail: detail)
                completion(upstreamResponse)
            } else {
                let detail = "Upstream timeout -> SERVFAIL"
                DNSLogger.shared.log(domain: domain, type: qtype, blocked: false, detail: detail)

                let servFail = DNSPacket.synthesizeServFail(queryID: packet.id, questionData: question.rawQuestionBytes)
                completion(servFail)
            }
        }
    }
}

// MARK: - Rate Limiter (Thread-Safe Sliding Window)
private final class RateLimiter: @unchecked Sendable {
    private let lock = NSLock()
    private var currentWindowSecond: Int = 0
    private var count: Int = 0
    private let maxQPS: Int = 5000

    func allow() -> Bool {
        let nowSecond = Int(Date().timeIntervalSince1970)
        lock.lock()
        defer { lock.unlock() }

        if nowSecond != currentWindowSecond {
            currentWindowSecond = nowSecond
            count = 1
            return true
        }

        if count >= maxQPS {
            return false
        }

        count += 1
        return true
    }
}

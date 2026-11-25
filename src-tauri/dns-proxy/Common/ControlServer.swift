import Foundation
import Darwin

/// Local control interface operating over a Unix domain socket (AF_UNIX).
/// Allows local management processes (and the Preelock desktop app) to query proxy status,
/// trigger policy reloads, and switch active per-session policies without proxy restarts.
/// Strictly local: does not listen on any network interface or port.
public final class ControlServer: @unchecked Sendable {

    public static let shared = ControlServer()

    private let socketQueue = DispatchQueue(label: "com.preelock.poc.controlserver", qos: .utility)
    private var serverFd: Int32 = -1
    private var isRunning = false
    private let startTime = Date()

    public private(set) var socketPath: String = ""

    private init() {}

    /// Starts the Unix domain socket control server at the specified path.
    public func start(path: String) {
        self.socketPath = path

        socketQueue.async { [weak self] in
            guard let self = self else { return }
            self.runServer(path: path)
        }
    }

    /// Stops the control server and deletes the socket file.
    public func stop() {
        isRunning = false
        if serverFd >= 0 {
            close(serverFd)
            serverFd = -1
        }
        if !socketPath.isEmpty {
            unlink(socketPath)
        }
    }

    private func runServer(path: String) {
        // Ensure parent directory exists
        let dir = URL(fileURLWithPath: path).deletingLastPathComponent().path
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)

        // Remove any stale socket
        unlink(path)

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else {
            print("[WARN] Failed to create control socket: \(errno)")
            return
        }
        self.serverFd = fd

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        _ = withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: 104) { dest in
                strncpy(dest, path, 103)
            }
        }

        let len = socklen_t(MemoryLayout<sockaddr_un>.size)
        let bindRes = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                bind(fd, sa, len)
            }
        }

        guard bindRes == 0 else {
            print("[WARN] Failed to bind control socket at \(path): \(errno)")
            close(fd)
            return
        }

        guard listen(fd, 10) == 0 else {
            print("[WARN] Failed to listen on control socket: \(errno)")
            close(fd)
            return
        }

        // Set socket file permissions so local Preelock processes can connect (0666)
        chmod(path, S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH)

        isRunning = true
        print("[INFO] Control socket active at \(path)")

        while isRunning {
            var clientAddr = sockaddr_un()
            var clientLen = socklen_t(MemoryLayout<sockaddr_un>.size)
            let clientFd = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                    accept(fd, sa, &clientLen)
                }
            }

            guard clientFd >= 0 else {
                if !isRunning { break }
                continue
            }

            handleClient(clientFd)
        }
    }

    private func handleClient(_ clientFd: Int32) {
        var buffer = [UInt8](repeating: 0, count: 4096)
        let bytesRead = recv(clientFd, &buffer, buffer.count, 0)

        guard bytesRead > 0, let commandLine = String(bytes: buffer.prefix(bytesRead), encoding: .utf8) else {
            close(clientFd)
            return
        }

        let trimmed = commandLine.trimmingCharacters(in: .whitespacesAndNewlines)
        let response = processCommand(trimmed) + "\n"

        if let data = response.data(using: .utf8) {
            _ = data.withUnsafeBytes { ptr in
                send(clientFd, ptr.baseAddress, data.count, 0)
            }
        }

        close(clientFd)
    }

    private func processCommand(_ cmd: String) -> String {
        let parts = cmd.split(separator: " ", maxSplits: 1).map(String.init)
        guard let action = parts.first?.lowercased() else {
            return "ERROR: Empty command"
        }

        switch action {
        case "status":
            let uptime = Int(Date().timeIntervalSince(startTime))
            let current = BlocklistManager.shared.currentPolicy
            let activeFile = BlocklistManager.shared.activeFilePath ?? "unknown"

            let statusDict: [String: Any] = [
                "status": "running",
                "uptime_seconds": uptime,
                "policy_name": current.name,
                "policy_mode": current.mode.rawValue,
                "active_file": activeFile,
                "domain_count": current.domains.count
            ]

            if let json = try? JSONSerialization.data(withJSONObject: statusDict, options: [.prettyPrinted, .sortedKeys]),
               let str = String(data: json, encoding: .utf8) {
                return str
            }
            return "STATUS: running, policy: \(current.name) (\(current.mode.rawValue)), domains: \(current.domains.count)"

        case "reload":
            switch BlocklistManager.shared.reload() {
            case .success(let policy):
                return "OK: Reloaded active policy '\(policy.name)' (\(policy.domains.count) domains)"
            case .failure(let error):
                return "ERROR: Failed to reload policy: \(error.localizedDescription)"
            }

        case "set-policy":
            guard parts.count > 1 else {
                return "ERROR: Missing path argument for set-policy. Usage: set-policy <path>"
            }
            let path = parts[1]
            switch BlocklistManager.shared.switchPolicy(to: path) {
            case .success(let policy):
                return "OK: Switched active policy to '\(policy.name)' from '\(path)' (\(policy.domains.count) domains)"
            case .failure(let error):
                return "ERROR: Failed to switch policy: \(error.localizedDescription)"
            }

        case "recent-domains":
            // The desktop app uses this to know which website the user
            // is on without the browser extension and without macOS
            // Accessibility permission: the most recently resolved
            // domains are (almost always) the active tab's site and its
            // assets. Returns the last 20 entries as JSON:
            // {"domains": [{"domain": "...", "timestamp": 0.0, "blocked": false}]}
            let entries = DNSLogger.shared.recentEntries()
            let recent = entries.suffix(20).map { entry -> [String: Any] in
                [
                    "domain": entry.domain,
                    "timestamp": entry.timestamp.timeIntervalSince1970,
                    "blocked": entry.blocked
                ]
            }
            let dict: [String: Any] = ["domains": Array(recent)]
            if let json = try? JSONSerialization.data(withJSONObject: dict, options: [.sortedKeys]),
               let str = String(data: json, encoding: .utf8) {
                return str
            }
            return "ERROR: Failed to serialize recent domains"

        case "stop", "shutdown", "exit":
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) {
                ControlServer.shared.stop()
                exit(0)
            }
            return "OK: Shutting down DNS proxy"

        default:
            return "ERROR: Unknown command '\(cmd)'. Available: status, reload, set-policy <path>, recent-domains, stop"
        }
    }
}

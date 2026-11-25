import Foundation
import Darwin

class HelperApp {
    private let server = SocketServer()
    private var refreshTimer: DispatchSourceTimer?

    func run() {
        let args = CommandLine.arguments

        if args.count <= 1 || args.contains("--help") || args.contains("-h") {
            printUsage()
            exit(0)
        }

        let subcommand = args[1]

        switch subcommand {
        case "daemon":
            runDaemon()

        case "apply":
            handleApplyCLI(args: Array(args.dropFirst(2)))

        case "clear":
            handleClearCLI(args: Array(args.dropFirst(2)))

        case "status":
            handleStatusCLI()

        case "refresh":
            handleRefreshCLI()

        case "remove":
            handleRemoveCLI()

        case "update":
            handleUpdateCLI(args: Array(args.dropFirst(2)))

        default:
            print("Unknown subcommand: \(subcommand)")
            printUsage()
            exit(1)
        }
    }

    private func runDaemon() {
        print("Starting Preelock privileged helper daemon...")

        // Handle signals
        signal(SIGTERM) { _ in
            print("Received SIGTERM, shutting down...")
            exit(0)
        }
        signal(SIGINT) { _ in
            print("Received SIGINT, shutting down...")
            exit(0)
        }
        signal(SIGPIPE, SIG_IGN)

        // Start socket server
        server.start()

        // Drop any expired session blocks on daemon startup (prevent zombie blocks)
        var state = StateStore.load()
        let now = UInt64(Date().timeIntervalSince1970)
        let strict = state.strictUntil ?? 0
        let strictSecs = strict > 100_000_000_000 ? (strict / 1000) : strict

        if now >= strictSecs {
            // Strict mode has expired or was not set; drop stale session blocks
            state.activeOwners.removeValue(forKey: "session")
            if state.activeOwners.isEmpty {
                state.strictUntil = nil
            }
            StateStore.save(state)
        }

        // Restore any still-valid persisted block (e.g. active strict session or active app lock)
        let activeDomains = StateStore.allDomains()
        let fwDomains = StateStore.firewallDomains()
        if !activeDomains.isEmpty {
            print("Restoring \(activeDomains.count) active blocked domains after startup...")
            let expandedHosts = HostsManager.expandDomains(allDomains: activeDomains, userDomains: fwDomains)
            try? HostsManager.apply(domains: expandedHosts)
            if !fwDomains.isEmpty {
                PFManager.apply(domains: fwDomains, forceRefresh: true)
            } else {
                PFManager.clear()
            }
        } else {
            try? HostsManager.clear()
            PFManager.clear()
        }

        // Schedule periodic 2-minute PF refresh (accumulates rotated CDN IPs and restores PF after VPN/sleep)
        let timer = DispatchSource.makeTimerSource(queue: DispatchQueue.global(qos: .utility))
        timer.schedule(deadline: .now() + 120, repeating: 120)
        timer.setEventHandler {
            let fwDomains = StateStore.firewallDomains()
            if !fwDomains.isEmpty {
                print("Running scheduled PF rule re-resolution for \(fwDomains.count) domains...")
                PFManager.apply(domains: fwDomains, forceRefresh: true)
            }
        }
        timer.resume()
        refreshTimer = timer

        RunLoop.main.run()
    }

    private func handleApplyCLI(args: [String]) {
        var domains: [String] = []
        var owner = "cli"
        var strict: UInt64? = nil

        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--domains" || arg == "-d") && i + 1 < args.count {
                domains = args[i + 1].split(separator: ",").map { String($0) }
                i += 2
            } else if (arg == "--owner" || arg == "-o") && i + 1 < args.count {
                owner = args[i + 1]
                i += 2
            } else if (arg == "--strict-until" || arg == "-s") && i + 1 < args.count {
                strict = UInt64(args[i + 1])
                i += 2
            } else {
                i += 1
            }
        }

        let payload: [String: Any] = [
            "command": "apply",
            "owner": owner,
            "domains": domains,
            "strict_until": strict as Any
        ]
        sendSocketOrExecuteDirect(payload: payload)
    }

    private func handleClearCLI(args: [String]) {
        var owner = "session"
        var force = false

        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--owner" || arg == "-o") && i + 1 < args.count {
                owner = args[i + 1]
                i += 2
            } else if arg == "--force" || arg == "-f" {
                force = true
                i += 1
            } else {
                i += 1
            }
        }

        let payload: [String: Any] = [
            "command": "clear",
            "owner": owner,
            "force": force
        ]
        sendSocketOrExecuteDirect(payload: payload)
    }

    private func handleStatusCLI() {
        let payload: [String: Any] = ["command": "status"]
        sendSocketOrExecuteDirect(payload: payload)
    }

    private func handleRefreshCLI() {
        let payload: [String: Any] = ["command": "refresh"]
        sendSocketOrExecuteDirect(payload: payload)
    }

    private func handleRemoveCLI() {
        let payload: [String: Any] = ["command": "remove"]
        sendSocketOrExecuteDirect(payload: payload)
    }

    private func handleUpdateCLI(args: [String]) {
        var srcPath: String?
        var i = 0
        while i < args.count {
            if (args[i] == "--binary" || args[i] == "-b") && i + 1 < args.count {
                srcPath = args[i + 1]
                i += 2
            } else {
                i += 1
            }
        }
        guard let src = srcPath, FileManager.default.fileExists(atPath: src) else {
            print("{\"status\":\"error\",\"message\":\"valid --binary <path> required\"}")
            exit(1)
        }
        let dst = "/Library/PrivilegedHelperTools/preelock-helper"
        do {
            if FileManager.default.fileExists(atPath: dst) {
                try FileManager.default.removeItem(atPath: dst)
            }
            try FileManager.default.copyItem(atPath: src, toPath: dst)
            let attrs: [FileAttributeKey: Any] = [.posixPermissions: 0o755]
            try FileManager.default.setAttributes(attrs, ofItemAtPath: dst)

            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
            p.arguments = ["kickstart", "-k", "system/com.preelock.helper"]
            try? p.run()
            p.waitUntilExit()

            print("{\"status\":\"ok\",\"updated\":true}")
            exit(0)
        } catch {
            print("{\"status\":\"error\",\"message\":\"\(error.localizedDescription)\"}")
            exit(1)
        }
    }

    private func sendSocketOrExecuteDirect(payload: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let jsonStr = String(data: data, encoding: .utf8) else {
            print("{\"status\":\"error\",\"message\":\"failed to serialize request\"}")
            return
        }

        // Try talking to running socket server first
        if let response = sendToSocket(jsonStr) {
            print(response)
            return
        }

        // Fall back to direct local execution (requires root privileges)
        let response = server.processCommand(jsonStr)
        print(response)
    }

    private func sendToSocket(_ msg: String) -> String? {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        defer { close(fd) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
        withUnsafeMutablePointer(to: &addr.sun_path.0) { ptr in
            _ = SocketServer.socketPath.withCString { cstr in
                strncpy(ptr, cstr, maxLen - 1)
            }
        }

        let addrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
        let connectRes = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                connect(fd, sa, addrLen)
            }
        }

        guard connectRes == 0 else { return nil }

        guard let data = (msg + "\n").data(using: .utf8) else { return nil }
        _ = data.withUnsafeBytes { ptr in
            write(fd, ptr.baseAddress, data.count)
        }

        var buffer = [UInt8](repeating: 0, count: 65536)
        let bytesRead = read(fd, &buffer, buffer.count)
        guard bytesRead > 0 else { return nil }

        return String(data: Data(buffer[0..<bytesRead]), encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func printUsage() {
        print("""
        Preelock Privileged Helper

        Usage:
          preelock-helper daemon
          preelock-helper apply --domains <d1,d2,...> [--owner <owner>] [--strict-until <timestamp>]
          preelock-helper clear [--owner <owner>] [--force]
          preelock-helper status
          preelock-helper refresh
          preelock-helper remove
        """)
    }
}

let app = HelperApp()
app.run()

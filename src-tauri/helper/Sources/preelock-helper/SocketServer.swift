import Foundation
import Darwin

public class SocketServer {
    public static let socketPath = "/var/run/preelock-helper.sock"
    private var serverFd: Int32 = -1
    private var isRunning = false
    private let queue = DispatchQueue(label: "com.preelock.helper.socket", qos: .userInitiated)

    // `apply` is acknowledged IMMEDIATELY and the expensive work
    // (DNS resolution, pfctl, state kills, hosts rewrite) runs on this
    // serial queue. Before this, a single apply could take >3 s; the
    // engine waited on it every tick, starved its own IPC loop, and the
    // GUI reported "Snapshot timed out after 10s" while the timer froze.
    // Coalescing keeps a burst of policy updates down to the latest one.
    private let applyQueue = DispatchQueue(label: "com.preelock.helper.apply", qos: .utility)
    private let applyLock = NSLock()
    private var pendingApplyDomains: [String]? = nil
    private var applyWorkerRunning = false
    private var lastHostsDomains: [String]? = nil

    public init() {}

    public func start() {
        unlink(SocketServer.socketPath)

        serverFd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard serverFd >= 0 else {
            print("Failed to create Unix socket: \(errno)")
            return
        }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
        withUnsafeMutablePointer(to: &addr.sun_path.0) { ptr in
            _ = SocketServer.socketPath.withCString { cstr in
                strncpy(ptr, cstr, maxLen - 1)
            }
        }

        let addrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
        let bindRes = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                bind(serverFd, sa, addrLen)
            }
        }

        guard bindRes == 0 else {
            print("Failed to bind socket at \(SocketServer.socketPath): \(errno)")
            close(serverFd)
            return
        }

        // Grant read/write permission to local processes
        chmod(SocketServer.socketPath, 0o666)

        guard listen(serverFd, 16) == 0 else {
            print("Failed to listen on socket: \(errno)")
            close(serverFd)
            return
        }

        isRunning = true
        print("Preelock helper socket listening at \(SocketServer.socketPath)")

        queue.async { [weak self] in
            self?.acceptLoop()
        }
    }

    public func stop() {
        isRunning = false
        if serverFd >= 0 {
            close(serverFd)
            serverFd = -1
        }
        unlink(SocketServer.socketPath)
    }

    private func acceptLoop() {
        while isRunning {
            var clientAddr = sockaddr_un()
            var clientLen = socklen_t(MemoryLayout<sockaddr_un>.size)
            let clientFd = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                    accept(serverFd, sa, &clientLen)
                }
            }

            guard clientFd >= 0 else {
                if !isRunning { break }
                continue
            }

            var noSigpipe: Int32 = 1
            setsockopt(clientFd, SOL_SOCKET, SO_NOSIGPIPE, &noSigpipe, socklen_t(MemoryLayout<Int32>.size))

            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.handleClient(clientFd)
            }
        }
    }

    private func handleClient(_ fd: Int32) {
        defer { close(fd) }

        var buffer = [UInt8](repeating: 0, count: 65536)
        var requestData = Data()
        while true {
            let bytesRead = read(fd, &buffer, buffer.count)
            guard bytesRead > 0 else { break }
            requestData.append(contentsOf: buffer[0..<bytesRead])
            if buffer[0..<bytesRead].contains(0x0a) {
                break
            }
        }
        guard !requestData.isEmpty else { return }

        guard let requestStr = String(data: requestData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) else {
            return
        }

        let responseStr = processCommand(requestStr)
        if let responseData = (responseStr + "\n").data(using: .utf8) {
            _ = responseData.withUnsafeBytes { ptr in
                write(fd, ptr.baseAddress, responseData.count)
            }
        }
    }

    private func enqueueApply(_ domains: [String]) {
        applyLock.lock()
        pendingApplyDomains = domains
        let startWorker = !applyWorkerRunning
        if startWorker { applyWorkerRunning = true }
        applyLock.unlock()
        guard startWorker else { return }
        applyQueue.async { [weak self] in
            guard let self else { return }
            while true {
                self.applyLock.lock()
                let next = self.pendingApplyDomains
                self.pendingApplyDomains = nil
                if next == nil { self.applyWorkerRunning = false }
                self.applyLock.unlock()
                guard let current = next else { break }
                let fwDomains = StateStore.firewallDomains()
                if fwDomains.isEmpty {
                    PFManager.clear()
                } else {
                    PFManager.apply(domains: fwDomains)
                }
                let _ = current
            }
        }
    }

    public func processCommand(_ line: String) -> String {
        guard let data = line.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let cmd = json["command"] as? String else {
            return "{\"status\":\"error\",\"message\":\"invalid json format\"}"
        }

        switch cmd {
        case "apply":
            let owner = json["owner"] as? String ?? "session"
            let domains = json["domains"] as? [String] ?? []
            let strict = json["strict_until"] as? UInt64

            let combined = StateStore.apply(owner: owner, domains: domains, strictUntil: strict)
            let expandedHosts = HostsManager.expandDomains(
                allDomains: combined,
                userDomains: StateStore.firewallDomains()
            )
            var hostsChanged = false
            applyLock.lock()
            if lastHostsDomains != expandedHosts {
                lastHostsDomains = expandedHosts
                hostsChanged = true
            }
            applyLock.unlock()
            if hostsChanged {
                if expandedHosts.isEmpty {
                    try? HostsManager.clear()
                } else {
                    try? HostsManager.apply(domains: expandedHosts)
                }
            }
            enqueueApply(combined)
            return "{\"status\":\"ok\",\"blocked_count\":\(combined.count),\"queued\":true}"

        case "clear":
            let owner = json["owner"] as? String ?? "session"
            let force = json["force"] as? Bool ?? false

            let (success, remaining, errorMsg) = StateStore.clear(owner: owner, force: force)
            if !success {
                return "{\"status\":\"error\",\"message\":\"\(errorMsg ?? "cannot clear")\"}"
            }

            if remaining.isEmpty {
                applyLock.lock()
                lastHostsDomains = nil
                applyLock.unlock()
                try? HostsManager.clear()
                PFManager.clear()
            } else {
                let fwDomains = StateStore.firewallDomains()
                let expandedHosts = HostsManager.expandDomains(
                    allDomains: remaining,
                    userDomains: fwDomains
                )
                applyLock.lock()
                lastHostsDomains = expandedHosts
                applyLock.unlock()
                try? HostsManager.apply(domains: expandedHosts)
                if fwDomains.isEmpty {
                    PFManager.clear()
                } else {
                    PFManager.apply(domains: fwDomains)
                }
            }
            return "{\"status\":\"ok\",\"remaining_count\":\(remaining.count)}"

        case "status":
            let domains = StateStore.allDomains()
            let strict = StateStore.getStrictUntil()
            let strictStr = strict != nil ? "\(strict!)" : "null"
            let domainsJson = (try? String(data: JSONSerialization.data(withJSONObject: domains), encoding: .utf8)) ?? "[]"
            return "{\"status\":\"ok\",\"running\":true,\"version\":2,\"domains\":\(domainsJson),\"strict_until\":\(strictStr)}"

        case "refresh":
            let fwDomains = StateStore.firewallDomains()
            if !fwDomains.isEmpty {
                PFManager.apply(domains: fwDomains, forceRefresh: true)
            }
            return "{\"status\":\"ok\",\"refreshed_count\":\(fwDomains.count)}"

        case "remove":
            applyLock.lock()
            lastHostsDomains = nil
            applyLock.unlock()
            try? HostsManager.clear()
            PFManager.clear()
            StateStore.reset()
            return "{\"status\":\"ok\",\"removed\":true}"

        default:
            return "{\"status\":\"error\",\"message\":\"unknown command: \(cmd)\"}"
        }
    }
}

import Foundation

/// Monitors the active network interface and default gateway on macOS.
/// Automatically re-applies loopback DNS redirection (127.0.0.1) if macOS reverts
/// DNS configuration after a Wi-Fi network switch, DHCP lease renewal, or VPN connection.
public final class NetworkWatcher: @unchecked Sendable {

    public static let shared = NetworkWatcher()

    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "com.preelock.poc.networkwatcher", qos: .utility)
    private var isEnabled = false

    private init() {}

    /// Starts polling the active network service DNS at regular intervals.
    public func start(checkIntervalSeconds: Double = 3.0) {
        stop()
        isEnabled = true

        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + checkIntervalSeconds, repeating: checkIntervalSeconds)
        t.setEventHandler { [weak self] in
            guard let self = self, self.isEnabled else { return }
            self.verifyAndReapplyDNS()
        }
        t.resume()
        self.timer = t
        print("[INFO] Network-change watcher started (check interval: \(checkIntervalSeconds)s)")
    }

    /// Stops network monitoring.
    public func stop() {
        isEnabled = false
        timer?.cancel()
        timer = nil
    }

    /// RULE 1: Preelock must never change system DNS. This used to point
    /// the active service at 127.0.0.1 every 3 s, which both violated that
    /// rule and fought the startup safety net that restores DNS. It now
    /// only reports what it sees; blocking is done by /etc/hosts + PF.
    public func verifyAndReapplyDNS() {
        guard let service = detectActiveNetworkService() else { return }
        let currentDNS = getDNSServers(for: service)
        if currentDNS.contains("127.0.0.1") || currentDNS.contains("localhost") {
            print("[WARN] Service '\(service)' still points at loopback DNS; Preelock will not modify system DNS (RULE 1).")
        }
    }

    public func detectActiveNetworkService() -> String? {
        let pipe = Pipe()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/sbin/route")
        process.arguments = ["-n", "get", "default"]
        process.standardOutput = pipe
        process.standardError = Pipe()
        guard (try? process.run()) != nil else { return "Wi-Fi" }
        process.waitUntilExit()

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let output = String(data: data, encoding: .utf8) else { return "Wi-Fi" }

        var iface = "en0"
        for line in output.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("interface:") {
                let parts = trimmed.split(separator: ":").map { $0.trimmingCharacters(in: .whitespaces) }
                if parts.count >= 2 {
                    iface = parts[1]
                }
            }
        }

        let hwPipe = Pipe()
        let hwProcess = Process()
        hwProcess.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        hwProcess.arguments = ["-listallhardwareports"]
        hwProcess.standardOutput = hwPipe
        hwProcess.standardError = Pipe()
        guard (try? hwProcess.run()) != nil else { return "Wi-Fi" }
        hwProcess.waitUntilExit()

        let hwData = hwPipe.fileHandleForReading.readDataToEndOfFile()
        guard let hwOutput = String(data: hwData, encoding: .utf8) else { return "Wi-Fi" }

        var currentPort = ""
        for line in hwOutput.components(separatedBy: .newlines) {
            if line.hasPrefix("Hardware Port: ") {
                currentPort = String(line.dropFirst("Hardware Port: ".count))
            } else if line.hasPrefix("Device: ") {
                let dev = String(line.dropFirst("Device: ".count))
                if dev == iface {
                    return currentPort
                }
            }
        }

        return "Wi-Fi"
    }

    private func getDNSServers(for service: String) -> [String] {
        let pipe = Pipe()
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        p.arguments = ["-getdnsservers", service]
        p.standardOutput = pipe
        p.standardError = Pipe()
        guard (try? p.run()) != nil else { return [] }
        p.waitUntilExit()

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let output = String(data: data, encoding: .utf8) else { return [] }

        return output.components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }
    }

    private func setDNSServers(for service: String, servers: [String]) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        p.arguments = ["-setdnsservers", service] + servers
        p.standardOutput = Pipe()
        p.standardError = Pipe()
        try? p.run()
        p.waitUntilExit()
    }
}

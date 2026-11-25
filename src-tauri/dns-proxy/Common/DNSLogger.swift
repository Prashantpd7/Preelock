import Foundation

/// Thread-safe audit logger for DNS queries.
/// Records blocked and allowed query events for verification in the UI and log files.
/// All console and file operations are strictly serialized to eliminate race conditions,
/// garbled text, or interleaved log lines under concurrent query loads.
public final class DNSLogger: @unchecked Sendable {

    public static let shared = DNSLogger()

    public struct LogEntry: Identifiable, Codable {
        public let id: UUID
        public let timestamp: Date
        public let domain: String
        public let type: String
        public let blocked: Bool
        public let detail: String

        public init(domain: String, type: String, blocked: Bool, detail: String) {
            self.id = UUID()
            self.timestamp = Date()
            self.domain = domain
            self.type = type
            self.blocked = blocked
            self.detail = detail
        }

        public var formattedTime: String {
            let formatter = DateFormatter()
            formatter.dateFormat = "HH:mm:ss.SSS"
            return formatter.string(from: timestamp)
        }

        public var consoleString: String {
            let status = blocked ? "[BLOCKED]" : "[ALLOWED]"
            return "\(formattedTime) \(status) \(domain) (\(type)) - \(detail)"
        }
    }

    private let lock = NSLock()
    private var entries: [LogEntry] = []
    private static let maxMemoryEntries = 200
    private static let logFileName = "resolution.log"

    public var onLogAdded: ((LogEntry) -> Void)?

    private init() {}

    public func log(domain: String, type: String, blocked: Bool, detail: String) {
        let entry = LogEntry(domain: domain, type: type, blocked: blocked, detail: detail)

        // Strict lock serialization for both in-memory state, stdout, and file I/O
        lock.lock()
        defer { lock.unlock() }

        entries.append(entry)
        if entries.count > Self.maxMemoryEntries {
            entries.removeFirst(entries.count - Self.maxMemoryEntries)
        }

        // Print atomically to stdout and flush immediately
        print(entry.consoleString)
        fflush(stdout)

        // Write atomically to resolution.log
        appendToFile(entry)

        DispatchQueue.main.async { [weak self] in
            self?.onLogAdded?(entry)
        }
    }

    public func recentEntries() -> [LogEntry] {
        lock.lock()
        defer { lock.unlock() }
        return entries
    }

    private var logFileURL: URL {
        // Guarantee resolution.log is always anchored inside repository/proxy root
        if let execPath = Bundle.main.executablePath ?? CommandLine.arguments.first {
            let execURL = URL(fileURLWithPath: execPath).resolvingSymlinksInPath()
            var dir = execURL.deletingLastPathComponent()
            while dir.lastPathComponent != "PreelockDNSProofOfConcept" && dir.lastPathComponent != "dns-proxy" && dir.path != "/" {
                dir = dir.deletingLastPathComponent()
            }
            if dir.lastPathComponent == "PreelockDNSProofOfConcept" || dir.lastPathComponent == "dns-proxy" {
                return dir.appendingPathComponent(Self.logFileName)
            }
        }
        return URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent(Self.logFileName)
    }

    private func appendToFile(_ entry: LogEntry) {
        let fileURL = logFileURL
        let line = entry.consoleString + "\n"
        guard let data = line.data(using: .utf8) else { return }

        let dir = fileURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        if let handle = try? FileHandle(forWritingTo: fileURL) {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.synchronize()
            try? handle.close()
        } else {
            try? data.write(to: fileURL, options: .atomic)
        }
    }
}

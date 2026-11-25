import Foundation

public struct HelperState: Codable {
    public var activeOwners: [String: [String]]
    public var strictUntil: UInt64?
    public var lastRefresh: UInt64

    public init(activeOwners: [String: [String]] = [:], strictUntil: UInt64? = nil, lastRefresh: UInt64 = 0) {
        self.activeOwners = activeOwners
        self.strictUntil = strictUntil
        self.lastRefresh = lastRefresh
    }
}

public class StateStore {
    public static let stateDir = "/var/db/preelock"
    public static let stateFile = "/var/db/preelock/helper_state.json"

    private static let lock = NSLock()
    private static var current = load()

    public static func load() -> HelperState {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: stateFile)),
              let state = try? JSONDecoder().decode(HelperState.self, from: data) else {
            return HelperState()
        }
        return state
    }

    public static func save(_ state: HelperState) {
        try? FileManager.default.createDirectory(atPath: stateDir, withIntermediateDirectories: true)
        if let data = try? JSONEncoder().encode(state) {
            try? data.write(to: URL(fileURLWithPath: stateFile))
        }
    }

    public static func apply(owner: String, domains: [String], strictUntil: UInt64?) -> [String] {
        lock.lock()
        defer { lock.unlock() }

        var state = load()
        let cleaned = domains
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
        
        state.activeOwners[owner] = Array(Set(cleaned)).sorted()

        if let strict = strictUntil {
            // Sanitize milliseconds to seconds if 11+ digits (epoch ms)
            let strictSecs = strict > 100_000_000_000 ? (strict / 1000) : strict
            if strictSecs > (state.strictUntil ?? 0) {
                state.strictUntil = strictSecs
            }
        }
        state.lastRefresh = UInt64(Date().timeIntervalSince1970)
        save(state)
        current = state
        return allDomains(from: state)
    }

    public static func clear(owner: String, force: Bool = false) -> (success: Bool, remainingDomains: [String], error: String?) {
        lock.lock()
        defer { lock.unlock() }

        var state = load()
        let now = UInt64(Date().timeIntervalSince1970)

        if !force, let strict = state.strictUntil {
            let strictSecs = strict > 100_000_000_000 ? (strict / 1000) : strict
            if now < strictSecs {
                return (false, allDomains(from: state), "Strict mode is active until \(strictSecs). Cannot clear block early.")
            }
        }

        state.activeOwners.removeValue(forKey: owner)
        // If the session owner is cleared or no owners remain, reset strictUntil
        if owner == "session" || state.activeOwners.isEmpty {
            state.strictUntil = nil
        }
        state.lastRefresh = now
        save(state)
        current = state
        return (true, allDomains(from: state), nil)
    }

    public static func allDomains() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return allDomains(from: load())
    }

    public static func firewallDomains() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return firewallDomains(from: load())
    }

    public static func getStrictUntil() -> UInt64? {
        lock.lock()
        defer { lock.unlock() }
        let state = load()
        let now = UInt64(Date().timeIntervalSince1970)
        if let strict = state.strictUntil {
            let strictSecs = strict > 100_000_000_000 ? (strict / 1000) : strict
            if now < strictSecs {
                return strictSecs
            }
        }
        return nil
    }

    public static func reset() {
        lock.lock()
        defer { lock.unlock() }
        try? FileManager.default.removeItem(atPath: stateFile)
        current = HelperState()
    }

    private static func allDomains(from state: HelperState) -> [String] {
        var set = Set<String>()
        for list in state.activeOwners.values {
            for d in list {
                set.insert(d)
            }
        }
        return Array(set).sorted()
    }

    private static func firewallDomains(from state: HelperState) -> [String] {
        var set = Set<String>()
        for (owner, list) in state.activeOwners {
            if owner != "adult_block" {
                for d in list {
                    set.insert(d)
                }
            }
        }
        return Array(set).sorted()
    }
}

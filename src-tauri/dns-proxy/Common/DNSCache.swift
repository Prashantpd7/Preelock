import Foundation

/// High-performance, thread-safe in-memory cache for resolved DNS responses.
/// Respects upstream TTLs clamped between 5s (min) and 60s (max) to prevent stale cache entries
/// while mitigating upstream query floods.
/// On cache hit, transaction ID is rewritten to match the incoming client query.
public final class DNSCache: @unchecked Sendable {

    public static let shared = DNSCache()

    private struct CacheKey: Hashable {
        let domain: String
        let type: UInt16
    }

    private struct CacheEntry {
        let responseData: Data
        let expiry: Date
        let createdAt: Date
    }

    private let lock = NSLock()
    private var entries: [CacheKey: CacheEntry] = [:]
    private let maxEntries: Int = 1000

    public private(set) var hitCount: Int = 0
    public private(set) var missCount: Int = 0

    private init() {}

    /// Looks up a cached response for the specified domain and record type.
    /// If valid, rewrites the response's transaction ID to match `queryID` and returns it.
    public func get(domain: String, type: UInt16, queryID: UInt16) -> Data? {
        let key = CacheKey(domain: domain.lowercased(), type: type)
        let now = Date()

        lock.lock()
        defer { lock.unlock() }

        guard let entry = entries[key] else {
            missCount += 1
            return nil
        }

        if now >= entry.expiry {
            entries.removeValue(forKey: key)
            missCount += 1
            return nil
        }

        hitCount += 1
        var response = entry.responseData
        guard response.count >= 2 else { return nil }

        // Rewrite transaction ID to match the client's query ID (RFC 1035 §4.1.1)
        response[0] = UInt8((queryID >> 8) & 0xFF)
        response[1] = UInt8(queryID & 0xFF)
        return response
    }

    /// Stores a response in the cache with TTL clamped between 5s and 60s.
    public func set(domain: String, type: UInt16, responseData: Data, ttl: UInt32?) {
        guard responseData.count >= 12 else { return }

        // Clamp TTL between 5 and 60 seconds
        let rawTTL = ttl ?? 30
        let clampedTTL = min(max(rawTTL, 5), 60)
        let now = Date()
        let expiry = now.addingTimeInterval(TimeInterval(clampedTTL))

        let key = CacheKey(domain: domain.lowercased(), type: type)
        let entry = CacheEntry(responseData: responseData, expiry: expiry, createdAt: now)

        lock.lock()
        defer { lock.unlock() }

        // If capacity exceeded, purge expired entries first
        if entries.count >= maxEntries {
            entries = entries.filter { $0.value.expiry > now }
            // If still over capacity, remove arbitrary oldest entry
            if entries.count >= maxEntries {
                if let oldestKey = entries.min(by: { $0.value.createdAt < $1.value.createdAt })?.key {
                    entries.removeValue(forKey: oldestKey)
                }
            }
        }

        entries[key] = entry
    }

    /// Removes all cached entries (useful during configuration reloads or testing).
    public func clear() {
        lock.lock()
        entries.removeAll()
        hitCount = 0
        missCount = 0
        lock.unlock()
    }

    /// Returns the current number of cached entries.
    public var count: Int {
        lock.lock()
        defer { lock.unlock() }
        return entries.count
    }
}

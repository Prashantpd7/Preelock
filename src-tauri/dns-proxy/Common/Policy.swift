import Foundation

/// Representation of a website filtering policy (Version 2).
/// Supports both blocklist mode (block listed domains, allow all others)
/// and allowlist mode (allow listed domains, block all others),
/// along with configurable DNS-over-HTTPS (DoH) bypass protection.
public struct Policy: Codable, Equatable, Sendable {

    public enum Mode: String, Codable, Sendable {
        case blocklist
        case allowlist
    }

    public let version: Int
    public let name: String
    public let mode: Mode
    public let domains: [String]
    public let block_doh: Bool
    public let block_adult: Bool

    enum CodingKeys: String, CodingKey {
        case version, name, mode, domains, block_doh, block_adult
    }

    public init(version: Int = 2, name: String, mode: Mode, domains: [String], block_doh: Bool = true, block_adult: Bool = false) {
        self.version = version
        self.name = name
        self.mode = mode
        self.domains = domains
        self.block_doh = block_doh
        self.block_adult = block_adult
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.version = try container.decodeIfPresent(Int.self, forKey: .version) ?? 2
        self.name = try container.decodeIfPresent(String.self, forKey: .name) ?? "Custom Policy"
        self.mode = try container.decodeIfPresent(Mode.self, forKey: .mode) ?? .blocklist
        self.domains = try container.decodeIfPresent([String].self, forKey: .domains) ?? []
        self.block_doh = try container.decodeIfPresent(Bool.self, forKey: .block_doh) ?? true
        self.block_adult = try container.decodeIfPresent(Bool.self, forKey: .block_adult) ?? false
    }

    /// Decodes a Policy from raw Data.
    /// Backward-compatible with Version 1 plain JSON arrays (e.g. ["domain1.com", "domain2.com"]).
    public static func decode(from data: Data) throws -> Policy {
        // 1. Try decoding structured Policy v2
        if let policy = try? JSONDecoder().decode(Policy.self, from: data) {
            return policy
        }

        // 2. Backward compatibility: Fallback to plain string array v1
        if let array = try? JSONDecoder().decode([String].self, from: data) {
            return Policy(version: 1, name: "Legacy Blocklist", mode: .blocklist, domains: array, block_doh: true, block_adult: false)
        }

        throw DecodingError.dataCorrupted(
            DecodingError.Context(codingPath: [], debugDescription: "Data is neither a valid Policy v2 nor a legacy [String] array")
        )
    }

    /// Encodes policy as pretty-printed JSON.
    public func encode() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(self)
    }
}

import Foundation

/// High-performance, zero-dependency DNS packet parser and synthesizer (RFC 1035).
/// Operates directly on raw wire-format UDP/TCP datagrams.
public struct DNSPacket {

    // MARK: - DNS Resource Record Types
    public enum RecordType: UInt16, CustomStringConvertible {
        case a = 1
        case ns = 2
        case cname = 5
        case soa = 6
        case ptr = 12
        case mx = 15
        case txt = 16
        case aaaa = 28
        case srv = 33
        case dnskey = 48
        case https = 65
        case any = 255
        case unknown = 0

        public init(rawValue: UInt16) {
            switch rawValue {
            case 1: self = .a
            case 2: self = .ns
            case 5: self = .cname
            case 6: self = .soa
            case 12: self = .ptr
            case 15: self = .mx
            case 16: self = .txt
            case 28: self = .aaaa
            case 33: self = .srv
            case 48: self = .dnskey
            case 65: self = .https
            case 255: self = .any
            default: self = .unknown
            }
        }

        public var description: String {
            switch self {
            case .a: return "A"
            case .ns: return "NS"
            case .cname: return "CNAME"
            case .soa: return "SOA"
            case .ptr: return "PTR"
            case .mx: return "MX"
            case .txt: return "TXT"
            case .aaaa: return "AAAA"
            case .srv: return "SRV"
            case .dnskey: return "DNSKEY"
            case .https: return "HTTPS"
            case .any: return "ANY"
            case .unknown: return "UNKNOWN"
            }
        }
    }

    // MARK: - Question Representation
    public struct Question {
        public let name: String
        public let type: RecordType
        public let rawType: UInt16
        public let `class`: UInt16
        public let rawQuestionBytes: Data
    }

    // MARK: - Parsed Properties
    public let id: UInt16
    public let isResponse: Bool
    public let opcode: UInt8
    public let isAuthoritative: Bool
    public let isTruncated: Bool
    public let recursionDesired: Bool
    public let recursionAvailable: Bool
    public let responseCode: UInt8
    public let questions: [Question]

    // MARK: - Parsing
    public static func parse(_ data: Data) -> DNSPacket? {
        guard data.count >= 12 else { return nil }

        // Header format (12 bytes):
        // 0-1: Transaction ID
        // 2-3: Flags
        // 4-5: QDCOUNT (Question Count)
        // 6-7: ANCOUNT (Answer Count)
        // 8-9: NSCOUNT (Authority Record Count)
        // 10-11: ARCOUNT (Additional Record Count)
        let id = data.withUnsafeBytes { $0.load(fromByteOffset: 0, as: UInt16.self).bigEndian }
        let flags = data.withUnsafeBytes { $0.load(fromByteOffset: 2, as: UInt16.self).bigEndian }
        let qdCount = data.withUnsafeBytes { $0.load(fromByteOffset: 4, as: UInt16.self).bigEndian }

        // Hardening: A query must have at least one question (RFC 1035)
        guard qdCount > 0 else { return nil }

        let isResponse = (flags & 0x8000) != 0
        let opcode = UInt8((flags & 0x7800) >> 11)
        let isAuthoritative = (flags & 0x0400) != 0
        let isTruncated = (flags & 0x0200) != 0
        let recursionDesired = (flags & 0x0100) != 0
        let recursionAvailable = (flags & 0x0080) != 0
        let responseCode = UInt8(flags & 0x000F)

        var questions: [Question] = []
        var offset = 12

        for _ in 0..<qdCount {
            let questionStart = offset
            var nameParts: [String] = []
            var foundTerminator = false

            while offset < data.count {
                let length = Int(data[offset])
                offset += 1

                if length == 0 {
                    foundTerminator = true
                    break // Null byte signals end of domain name
                }

                // Hardening: Question section names must NOT use compression pointers (RFC 1035 §4.1.2)
                if (length & 0xC0) == 0xC0 {
                    return nil
                }

                // Hardening: DNS label length cannot exceed 63 bytes (RFC 1035 §2.3.4)
                if length > 63 {
                    return nil
                }

                guard offset + length <= data.count else { return nil }
                let labelData = data.subdata(in: offset..<(offset + length))
                if let label = String(data: labelData, encoding: .utf8) {
                    nameParts.append(label.lowercased())
                } else {
                    return nil // Non-UTF8 label is malformed
                }
                offset += length
            }

            guard foundTerminator else { return nil }

            guard offset + 4 <= data.count else { return nil }
            let qtypeRaw = data.withUnsafeBytes { $0.load(fromByteOffset: offset, as: UInt16.self).bigEndian }
            let qclassRaw = data.withUnsafeBytes { $0.load(fromByteOffset: offset + 2, as: UInt16.self).bigEndian }
            offset += 4

            let rawQuestionBytes = data.subdata(in: questionStart..<offset)
            let domainName = nameParts.isEmpty ? "." : nameParts.joined(separator: ".")

            // Hardening: Domain name cannot exceed 253 characters (RFC 1035 §2.3.4)
            if domainName.count > 253 {
                return nil
            }

            questions.append(Question(
                name: domainName,
                type: RecordType(rawValue: qtypeRaw),
                rawType: qtypeRaw,
                class: qclassRaw,
                rawQuestionBytes: rawQuestionBytes
            ))
        }

        return DNSPacket(
            id: id,
            isResponse: isResponse,
            opcode: opcode,
            isAuthoritative: isAuthoritative,
            isTruncated: isTruncated,
            recursionDesired: recursionDesired,
            recursionAvailable: recursionAvailable,
            responseCode: responseCode,
            questions: questions
        )
    }

    // MARK: - Synthesizers

    /// Synthesizes an authoritative NXDOMAIN (Name Error, RCODE=3) packet.
    /// This causes browsers and OS network clients to recognize the domain does not exist.
    public static func synthesizeNXDomain(queryID: UInt16, questionData: Data) -> Data {
        var response = Data()

        // Transaction ID (matching query)
        response.append(contentsOf: queryID.bigEndianBytes)

        // Flags: QR=1 (Response), RD=1, RA=1 (Recursion Available), RCODE=3 (NXDomain)
        // 0x8183 = 1000 0001 1000 0011
        let flags: UInt16 = 0x8183
        response.append(contentsOf: flags.bigEndianBytes)

        // Counts: QDCOUNT=1, ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)

        // Echo question section
        response.append(questionData)

        return response
    }

    /// Synthesizes an A (IPv4) record resolving to 0.0.0.0 with a 60-second TTL.
    public static func synthesizeA(queryID: UInt16, questionData: Data, ttl: UInt32 = 60) -> Data {
        var response = Data()

        // ID
        response.append(contentsOf: queryID.bigEndianBytes)

        // Flags: QR=1, AA=1, RA=1, RCODE=0 (NoError)
        // 0x8500 = 1000 0101 0000 0000
        let flags: UInt16 = 0x8500
        response.append(contentsOf: flags.bigEndianBytes)

        // QDCOUNT=1, ANCOUNT=1, NSCOUNT=0, ARCOUNT=0
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)

        // Echo question
        response.append(questionData)

        // Answer Record:
        // Name pointer -> offset 0x0C (12), pointing to QNAME in question section
        response.append(contentsOf: [0xC0, 0x0C])
        // TYPE: A (1)
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        // CLASS: IN (1)
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        // TTL
        response.append(contentsOf: ttl.bigEndianBytes)
        // RDLENGTH: 4 bytes (IPv4)
        response.append(contentsOf: UInt16(4).bigEndianBytes)
        // RDATA: 0.0.0.0
        response.append(contentsOf: [0x00, 0x00, 0x00, 0x00])

        return response
    }

    /// Synthesizes an AAAA (IPv6) record resolving to :: (unspecified) with a 60-second TTL.
    public static func synthesizeAAAA(queryID: UInt16, questionData: Data, ttl: UInt32 = 60) -> Data {
        var response = Data()

        response.append(contentsOf: queryID.bigEndianBytes)
        let flags: UInt16 = 0x8500
        response.append(contentsOf: flags.bigEndianBytes)

        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)

        response.append(questionData)

        // Answer Record:
        response.append(contentsOf: [0xC0, 0x0C]) // Pointer to name
        response.append(contentsOf: UInt16(28).bigEndianBytes) // TYPE: AAAA
        response.append(contentsOf: UInt16(1).bigEndianBytes)  // CLASS: IN
        response.append(contentsOf: ttl.bigEndianBytes)        // TTL
        response.append(contentsOf: UInt16(16).bigEndianBytes) // RDLENGTH: 16 bytes
        response.append(Data(repeating: 0, count: 16))         // RDATA: ::

        return response
    }

    /// Synthesizes a SERVFAIL (Server Failure, RCODE=2) packet.
    /// Used when upstream resolvers time out or return unrecoverable socket errors.
    public static func synthesizeServFail(queryID: UInt16, questionData: Data) -> Data {
        var response = Data()
        response.append(contentsOf: queryID.bigEndianBytes)

        // QR=1, RA=1, RCODE=2 (SERVFAIL) -> 0x8102
        let flags: UInt16 = 0x8102
        response.append(contentsOf: flags.bigEndianBytes)

        response.append(contentsOf: UInt16(1).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(contentsOf: UInt16(0).bigEndianBytes)
        response.append(questionData)

        return response
    }

    // MARK: - TTL Extraction

    /// Extracts the TTL from the first Answer record of a DNS response packet.
    /// Returns nil if the packet is not a response, has no answers, or is malformed.
    public static func extractTTL(from data: Data) -> UInt32? {
        guard data.count >= 12 else { return nil }
        let flags = data.withUnsafeBytes { $0.load(fromByteOffset: 2, as: UInt16.self).bigEndian }
        let isResponse = (flags & 0x8000) != 0
        guard isResponse else { return nil }

        let qdCount = Int(data.withUnsafeBytes { $0.load(fromByteOffset: 4, as: UInt16.self).bigEndian })
        let anCount = Int(data.withUnsafeBytes { $0.load(fromByteOffset: 6, as: UInt16.self).bigEndian })
        guard anCount > 0 else { return nil }

        var offset = 12

        // Skip Question section
        for _ in 0..<qdCount {
            while offset < data.count {
                let length = Int(data[offset])
                offset += 1
                if length == 0 { break }
                if (length & 0xC0) == 0xC0 {
                    offset += 1 // 2-byte pointer (1 byte already consumed)
                    break
                }
                offset += length
            }
            offset += 4 // QTYPE (2) + QCLASS (2)
        }

        guard offset < data.count else { return nil }

        // Skip Name in first Answer record
        while offset < data.count {
            let length = Int(data[offset])
            offset += 1
            if length == 0 { break }
            if (length & 0xC0) == 0xC0 {
                offset += 1 // 2-byte pointer
                break
            }
            offset += length
        }

        // We need at least TYPE (2) + CLASS (2) + TTL (4) = 8 bytes
        guard offset + 8 <= data.count else { return nil }
        let ttl = data.withUnsafeBytes { $0.load(fromByteOffset: offset + 4, as: UInt32.self).bigEndian }
        return ttl
    }
}

// MARK: - Byte Helpers
private extension UInt16 {
    var bigEndianBytes: [UInt8] {
        return [UInt8((self >> 8) & 0xFF), UInt8(self & 0xFF)]
    }
}

private extension UInt32 {
    var bigEndianBytes: [UInt8] {
        return [
            UInt8((self >> 24) & 0xFF),
            UInt8((self >> 16) & 0xFF),
            UInt8((self >> 8) & 0xFF),
            UInt8(self & 0xFF)
        ]
    }
}

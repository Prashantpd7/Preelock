// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "PreelockDNSProxy",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(
            name: "preelock-dns-proxy",
            targets: ["preelock-dns-proxy"]
        )
    ],
    targets: [
        .target(
            name: "PreelockDNSCommon",
            path: "Common"
        ),
        .executableTarget(
            name: "preelock-dns-proxy",
            dependencies: ["PreelockDNSCommon"],
            path: "CLI"
        )
    ]
)

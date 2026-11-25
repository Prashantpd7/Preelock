// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "PreelockHelper",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(
            name: "preelock-helper",
            targets: ["preelock-helper"]
        )
    ],
    targets: [
        .executableTarget(
            name: "preelock-helper",
            path: "Sources/preelock-helper"
        )
    ]
)

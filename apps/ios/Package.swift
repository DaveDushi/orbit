// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "Orbit",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "Orbit", targets: ["Orbit"]),
        .library(name: "OrbitShare", targets: ["OrbitShare"]),
    ],
    targets: [
        .target(
            name: "Orbit",
            dependencies: ["OrbitSharing"],
            path: "Orbit",
            resources: [.process("Assets.xcassets")]
        ),
        .target(name: "OrbitSharing", path: "Shared"),
        .target(name: "OrbitShare", dependencies: ["OrbitSharing"], path: "ShareExtension"),
    ]
)

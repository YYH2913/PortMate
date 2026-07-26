// swift-tools-version: 6.1

import PackageDescription

let package = Package(
    name: "mcp-swift-client-check",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .executable(name: "McpSwiftClientCheck", targets: ["McpSwiftClientCheck"]),
    ],
    dependencies: [
        .package(url: "https://github.com/modelcontextprotocol/swift-sdk.git", exact: "0.12.1"),
        .package(url: "https://github.com/apple/swift-system.git", exact: "1.4.0"),
    ],
    targets: [
        .executableTarget(
            name: "McpSwiftClientCheck",
            dependencies: [
                .product(name: "MCP", package: "swift-sdk"),
                .product(name: "SystemPackage", package: "swift-system"),
            ]
        ),
    ]
)

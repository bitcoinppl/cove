// swift-tools-version: 6.1
import PackageDescription

let package = Package(
    name: "CoveCore",
    platforms: [
        .iOS(.v16)
    ],
    products: [
        .library(name: "CoveCore", targets: ["CoveCore"])
    ],
    targets: [
        .target(
            name: "cove_core_ffi",
            path: "Sources/cove_core_ffi",
            publicHeadersPath: "include",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.linkedLibrary("cove")],
        ),
        .target(
            name: "CoveCore",
            dependencies: ["cove_core_ffi"],
            path: "Sources/CoveCore",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)

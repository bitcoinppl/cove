// swift-tools-version: 6.1
import PackageDescription

let package = Package(
    name: "CoveCore",
    platforms: [
        .iOS(.v17)
    ],
    products: [
        .library(name: "CoveCore", targets: ["CoveCore"])
    ],
    targets: [
        .target(
            name: "ffi",
            path: "Sources/ffi",
            publicHeadersPath: "include",
            linkerSettings: [
                .linkedLibrary("cove")
            ]
        ),
        .target(
            name: "CoveCore",
            dependencies: ["ffi"],
            path: "Sources/CoveCore"
        )
    ]
)

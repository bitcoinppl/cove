// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "CoveSwiftLint",
    platforms: [
        .macOS(.v15),
    ],
    products: [
        .executable(name: "cove-swift-lint", targets: ["CoveSwiftLint"]),
    ],
    dependencies: [
        .package(
            url: "https://github.com/Ryu0118/swift-ast-lint.git",
            exact: "0.3.1"
        ),
        .package(
            url: "https://github.com/swiftlang/swift-syntax.git",
            "600.0.0"..<"700.0.0"
        ),
    ],
    targets: [
        .target(
            name: "CoveLintRules",
            dependencies: [
                .product(name: "SwiftASTLint", package: "swift-ast-lint"),
                .product(name: "SwiftSyntax", package: "swift-syntax"),
            ]
        ),
        .executableTarget(
            name: "CoveSwiftLint",
            dependencies: [
                "CoveLintRules",
                .product(name: "SwiftASTLint", package: "swift-ast-lint"),
            ]
        ),
        .testTarget(
            name: "CoveLintRulesTests",
            dependencies: [
                "CoveLintRules",
                .product(name: "SwiftASTLintTestSupport", package: "swift-ast-lint"),
            ]
        ),
    ]
)

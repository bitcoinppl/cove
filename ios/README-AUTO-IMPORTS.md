# Auto-Importing CoveCore in Your Project

This document explains the various ways to automatically import the CoveCore framework in your Swift files.

## Quick Setup

Run the following command to set up automatic imports:

```bash
./scripts/setup-auto-imports.sh [debug|release] [method]
```

Where:
- The first argument is the build type (`debug` or `release`)
- The second argument is the import method (`precompiled`, `bridging`, or `swift`)

## Available Methods

There are three different methods for auto-importing CoveCore. Each has its advantages:

### Method 1: Precompiled Swift Header (Best Performance)

```bash
./scripts/setup-auto-imports.sh debug precompiled
```

This method uses a Swift file (`CoveImports.swift`) that's precompiled and implicitly imported into all files.

**Pros**:
- Fast compilation
- Pure Swift solution
- Works with all Swift files

**Cons**:
- Requires Xcode 14+ for best support

### Method 2: Objective-C Bridging Header

```bash
./scripts/setup-auto-imports.sh debug bridging
```

This method uses an Objective-C bridging header that's automatically included in all Swift files.

**Pros**:
- Widely supported in all Xcode versions
- Very reliable

**Cons**:
- Small compilation overhead
- Requires Objective-C interop

### Method 3: Swift @_exported Imports (Simplest)

```bash
./scripts/setup-auto-imports.sh debug swift
```

This method adds `import GlobalImports` to your main app file, which re-exports CoveCore.

**Pros**:
- Simple to implement
- No Xcode project modifications
- Pure Swift solution

**Cons**:
- You may need to manually import in some files
- Not as comprehensive as the other methods

## Manual Integration

If you prefer to manually set up auto-imports, you can:

1. Build and integrate the CoveCore XCFramework:
   ```bash
   ./scripts/build-and-integrate-ios.sh
   ```

2. Then, choose one of these methods:
   - Run `./scripts/configure-precompiled-header.sh` for Method 1
   - Run `./scripts/configure-bridging-header.sh` for Method 2
   - Run `./scripts/add-global-imports.sh` for Method 3

## Troubleshooting

If you encounter issues:

1. Clean your build folder in Xcode (Cmd+Shift+K)
2. Restart Xcode
3. Check your build settings to ensure the correct paths are set
4. Try a different import method

## Usage Example

Once set up, you can use CoveCore types directly in your Swift files without importing:

```swift
// No import statement needed!

func example() {
    // Use CoveCore types directly
    let app = App.shared
    // ... rest of your code
}
```
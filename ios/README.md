# CoveCore XCFramework

This is a Swift framework that combines the Rust bindings for the following modules:
- cove
- tap_card
- rust_cktap
- util

## How to Build

Run the build script from the project root:

```bash
./scripts/build-ios.sh [debug|release] [--device] [--sign]
```

Options:
- First argument: Build type (`debug` or `release`). Default is `debug`.
- Second argument: Build for device (`--device` or `true`). Default is simulator only.
- Third argument: Sign the framework (`--sign`). Default is unsigned.

## How to Use in Your Project

1. **Add the XCFramework to your Xcode project**

   Drag and drop the `rust/ios/CoveCore.xcframework` into your Xcode project.
   
   Make sure to check "Copy items if needed" and add to your target.

2. **Link the XCFramework**

   In your target's Build Phases, ensure that the XCFramework is listed under "Link Binary With Libraries".

3. **Import in your Swift code**

   ```swift
   import CoveCore
   ```

4. **Usage Examples**

   The framework provides access to all functions and types defined in the Rust codebase.

   ```swift
   // Initialize the Cove application
   let app = App.shared
   
   // Access utility functions
   let result = UtilFunctions.someFunction()
   
   // Work with wallet functionality
   let wallet = Wallet(...)
   
   // Access TapCard functionality
   let tapCard = TapCard.create(...)
   ```

## Troubleshooting

If you encounter issues with missing symbols or imports, check that:

1. The XCFramework is properly linked to your target
2. You're building for a supported platform (iOS device or simulator)
3. The Swift generated bindings match the version of your Rust code

## Development

When making changes to the Rust code, you'll need to rebuild the XCFramework:

1. Make your changes to the Rust code
2. Run `./scripts/build-ios.sh` to regenerate the bindings and rebuild the XCFramework
3. Clean and rebuild your Xcode project

## Notes

- The framework includes all necessary headers and Swift files
- The combined Swift bindings are also available at `ios/Cove/Cove.swift` for reference
- The XCFramework supports both device and simulator architectures when built with the `--device` flag
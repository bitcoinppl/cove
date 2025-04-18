#!/bin/bash
set -e
set -o pipefail

cd rust

BUILD_TYPE=$1
DEVICE=$2
SIGN=$3

if [ "$BUILD_TYPE" == "release" ] || [ "$BUILD_TYPE" == "--release" ]; then
    BUILD_FLAG="--release"
    BUILD_TYPE="release"
    export RUST_LOG="cove=info"
elif [ "$BUILD_TYPE" == "debug" ] || [ "$BUILD_TYPE" == "--debug" ] ; then
    BUILD_FLAG=""
    BUILD_TYPE="debug"
else
    BUILD_FLAG="--profile $BUILD_TYPE"
fi

# Make sure the directory exists
mkdir -p ios/Cove.xcframework bindings ios/Cove

# Build the dylib
cargo build

# Generate bindings for cove and tap_card
OUTPUT_DIR="./bindings"

# Make sure output directory exists
mkdir -p "$OUTPUT_DIR"

# Generate Swift bindings using the specific uniffi-bindgen-swift command format
echo "Generating Swift bindings..."
DYLIB_PATH="./target/debug/libcove.dylib"

# Generate Swift source files
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --swift-sources

# Generate header files
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --headers

# Generate modulemap
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --modulemap

# Check if the bindings were generated
echo "Checking generated bindings..."
ls -la "$OUTPUT_DIR" || true

echo "BUILD_TYPE: $BUILD_TYPE"
echo "DEVICE: $DEVICE"
echo "SIGN: $SIGN"

if [ $BUILD_TYPE == "release" ] || [ $BUILD_TYPE == "release-smaller" ]; then
    TARGETS=(
        # aarch64-apple-ios-sim \
        aarch64-apple-ios \
        # x86_64-apple-darwin
        # aarch64-apple-darwin
    )
else
    # debug on device or simulator
    if [ "$DEVICE" == "true" ] || [ "$DEVICE" == "--device" ]; then
        TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
    else
        TARGETS=(aarch64-apple-ios-sim)
    fi
fi 
 
LIBRARY_FLAGS=""
echo "Build for targets: ${TARGETS[@]}"
for TARGET in ${TARGETS[@]}; do
    echo "Building for target: ${TARGET} with build type: ${BUILD_TYPE}"
    LIBRARY_FLAGS="$LIBRARY_FLAGS -library ./target/$TARGET/$BUILD_TYPE/libcove.a -headers ./bindings"

    rustup target add $TARGET
    cargo build --target=$TARGET $BUILD_FLAG
done

# Check for generated files
echo "Checking for required binding files..."
ls -la ./bindings

# Create a Swift module to properly combine all the Swift files
echo "Setting up Swift module compilation..."

# Create a temporary directory for the combined Swift module
SWIFT_MODULE_DIR="./swift_module_tmp"
mkdir -p "$SWIFT_MODULE_DIR"

# Copy all Swift files to the temporary directory
cp ./bindings/*.swift "$SWIFT_MODULE_DIR/"

# Create a module map file for the Swift module
cat > "$SWIFT_MODULE_DIR/module.modulemap" << EOF
module CoveBindings {
    header "coveFFI.h"
    header "tap_cardFFI.h"
    header "rust_cktapFFI.h"
    header "utilFFI.h"
    export *
}
EOF

# Create a main Swift file that imports all the other Swift files
cat > "$SWIFT_MODULE_DIR/CoveBindings.swift" << EOF
// Combined Swift bindings for all Cove modules
// This file imports all the individual binding files and re-exports them

@_exported import Foundation

// Import all the generated binding files
@_exported import struct CoveBindings.RustBuffer
@_exported import struct CoveBindings.ForeignBytes
EOF

# Create a header file that includes all the generated headers
cat > "$SWIFT_MODULE_DIR/CoveBindings.h" << EOF
#ifndef CoveBindings_h
#define CoveBindings_h

#include "coveFFI.h"
#include "tap_cardFFI.h"
#include "rust_cktapFFI.h"
#include "utilFFI.h"

#endif /* CoveBindings_h */
EOF

# Copy headers to the temporary directory
cp ./bindings/*.h "$SWIFT_MODULE_DIR/"

# Combine all Swift files into a single file manually for use in the iOS project
echo "Combining Swift bindings into a single file..."
echo "// Combined Swift bindings for all modules - generated $(date)" > ../ios/Cove/Cove.swift

# Utility swift files
if [ -f "./bindings/util.swift" ]; then
    echo -e "\n// === util module ===\n" >> ../ios/Cove/Cove.swift
    cat ./bindings/util.swift >> ../ios/Cove/Cove.swift
fi

# rust_cktap swift files
if [ -f "./bindings/rust_cktap.swift" ]; then
    echo -e "\n// === rust_cktap module ===\n" >> ../ios/Cove/Cove.swift
    cat ./bindings/rust_cktap.swift >> ../ios/Cove/Cove.swift
fi

# tap_card swift files
if [ -f "./bindings/tap_card.swift" ]; then
    echo -e "\n// === tap_card module ===\n" >> ../ios/Cove/Cove.swift
    cat ./bindings/tap_card.swift >> ../ios/Cove/Cove.swift
fi

# Main cove.swift file if it exists
if [ -f "./bindings/cove.swift" ]; then
    echo -e "\n// === cove module ===\n" >> ../ios/Cove/Cove.swift
    cat ./bindings/cove.swift >> ../ios/Cove/Cove.swift
fi

echo "Finished creating combined Swift file at ../ios/Cove/Cove.swift"

# Copy all header files and modulemaps to the bindings directory
echo "Ensuring all header files are in place..."
for HEADER in ./bindings/*.h; do
    echo "Found header: $HEADER"
done

for MODULEMAP in ./bindings/*.modulemap; do
    echo "Found modulemap: $MODULEMAP"
done

# Create a combined module.modulemap for the XCFramework
echo "Creating combined module.modulemap..."
echo "framework module Cove {" > ./bindings/module.modulemap

# Add all umbrella headers
for HEADER in ./bindings/*FFI.h; do
    BASE_NAME=$(basename "$HEADER" FFI.h)
    echo "  umbrella header \"${BASE_NAME}FFI.h\"" >> ./bindings/module.modulemap
done

echo "  export *" >> ./bindings/module.modulemap
echo "}" >> ./bindings/module.modulemap

# Copy our combined module files to the bindings directory for inclusion in the XCFramework
cp "$SWIFT_MODULE_DIR/CoveBindings.h" ./bindings/
cp "$SWIFT_MODULE_DIR/module.modulemap" ./bindings/CoveBindings.modulemap
 
# Recreate XCFramework
rm -rf "ios/Cove.xcframework" || true
xcodebuild -create-xcframework \
        $LIBRARY_FLAGS \
        -output "ios/Cove.xcframework"
 
# Cleanup
# rm -rf bindings

# if [ ! -z $SIGN ] && [ ! -z $SIGNING_IDENTITY ] || [ $SIGN == "--sign" ]; then
#     echo "Signing for distribution: identity: $SIGNING_IDENTITY"
#     codesign --timestamp -v --sign "$SIGNING_IDENTITY" "ios/Cove.xcframework"
# fi

# Clean up temporary directory
if [ -d "$SWIFT_MODULE_DIR" ]; then
    echo "Cleaning up temporary Swift module directory..."
    rm -rf "$SWIFT_MODULE_DIR"
fi

echo "✅ Successfully built Cove.xcframework with combined Swift bindings for cove and tap_card"
echo "📦 Framework located at: $(pwd)/ios/Cove.xcframework"
echo "📄 Swift file created at: $(realpath ../ios/Cove/Cove.swift)"
echo ""
echo "The Swift file contains combined bindings from the following modules:"
for MODULE in $(find ./bindings -name "*.swift" | sort); do
    MODULE_NAME=$(basename "$MODULE" .swift)
    echo "  - $MODULE_NAME"
done

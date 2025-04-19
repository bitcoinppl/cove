#!/bin/bash
set -e
set -o pipefail

cd rust

BUILD_TYPE=$1
DEVICE=$2
SIGN=$3
INTEGRATE=$4

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
mkdir -p ios/CoveCore.xcframework bindings ios/Cove

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
        aarch64-apple-ios
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
    
    # Create a target-specific headers directory to avoid conflicts
    TARGET_HEADERS_DIR="./bindings/${TARGET}"
    mkdir -p "$TARGET_HEADERS_DIR"
    cp ./bindings/*.swift "$TARGET_HEADERS_DIR/"
    cp ./bindings/*.h "$TARGET_HEADERS_DIR/"
    cp ./bindings/*.modulemap "$TARGET_HEADERS_DIR/" 2>/dev/null || true
    
    # Add to library flags
    LIBRARY_FLAGS="$LIBRARY_FLAGS -library ./target/$TARGET/$BUILD_TYPE/libcove.a -headers $TARGET_HEADERS_DIR"

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

# Create a comprehensive umbrella header
cat > "$SWIFT_MODULE_DIR/CoveCore.h" << EOF
#ifndef CoveCore_h
#define CoveCore_h

#include "coveFFI.h"
#include "tap_cardFFI.h"
#include "rust_cktapFFI.h"
#include "utilFFI.h"

#endif /* CoveCore_h */
EOF

# Create a more comprehensive module map
cat > "$SWIFT_MODULE_DIR/module.modulemap" << EOF
framework module CoveCore {
    umbrella header "CoveCore.h"
    
    export *
    module * { export * }
}
EOF

# Copy headers to the Swift module directory
cp ./bindings/*.h "$SWIFT_MODULE_DIR/"

# Combine all Swift files into a single file manually for use in the iOS project
echo "Combining Swift bindings into a single file..."
echo "// Combined Swift bindings for all modules - generated $(date)" > ../ios/Cove/Cove.swift
echo "import Foundation" >> ../ios/Cove/Cove.swift

# Utility swift files first (they might be dependencies)
if [ -f "./bindings/util.swift" ]; then
    echo -e "\n// === util module ===\n" >> ../ios/Cove/Cove.swift
    grep -v "import Foundation" ./bindings/util.swift >> ../ios/Cove/Cove.swift
fi

# rust_cktap swift files
if [ -f "./bindings/rust_cktap.swift" ]; then
    echo -e "\n// === rust_cktap module ===\n" >> ../ios/Cove/Cove.swift
    grep -v "import Foundation" ./bindings/rust_cktap.swift >> ../ios/Cove/Cove.swift
fi

# tap_card swift files
if [ -f "./bindings/tap_card.swift" ]; then
    echo -e "\n// === tap_card module ===\n" >> ../ios/Cove/Cove.swift
    grep -v "import Foundation" ./bindings/tap_card.swift >> ../ios/Cove/Cove.swift
fi

# Main cove.swift file if it exists
if [ -f "./bindings/cove.swift" ]; then
    echo -e "\n// === cove module ===\n" >> ../ios/Cove/Cove.swift
    grep -v "import Foundation" ./bindings/cove.swift >> ../ios/Cove/Cove.swift
fi

echo "Finished creating combined Swift file at ../ios/Cove/Cove.swift"

# Copy our umbrella header and module map to all target-specific directories
for TARGET in ${TARGETS[@]}; do
    TARGET_HEADERS_DIR="./bindings/${TARGET}"
    cp "$SWIFT_MODULE_DIR/CoveCore.h" "$TARGET_HEADERS_DIR/"
    cp "$SWIFT_MODULE_DIR/module.modulemap" "$TARGET_HEADERS_DIR/"
done

# Recreate XCFramework
echo "Creating XCFramework with flags: $LIBRARY_FLAGS"
rm -rf "ios/CoveCore.xcframework" || true
xcodebuild -create-xcframework \
        $LIBRARY_FLAGS \
        -output "ios/CoveCore.xcframework"

# Sign the framework if requested
if [ ! -z "$SIGN" ] && [ "$SIGN" == "--sign" ]; then
    SIGNING_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | awk '{print $2}')
    if [ ! -z "$SIGNING_IDENTITY" ]; then
        echo "Signing for distribution with identity: $SIGNING_IDENTITY"
        codesign --timestamp -v --sign "$SIGNING_IDENTITY" "ios/CoveCore.xcframework"
    else
        echo "Warning: No signing identity found. Framework will not be signed."
    fi
fi

# Clean up temporary directories
if [ -d "$SWIFT_MODULE_DIR" ]; then
    echo "Cleaning up temporary Swift module directory..."
    rm -rf "$SWIFT_MODULE_DIR"
fi

for TARGET in ${TARGETS[@]}; do
    TARGET_HEADERS_DIR="./bindings/${TARGET}"
    if [ -d "$TARGET_HEADERS_DIR" ]; then
        echo "Cleaning up target headers directory: $TARGET_HEADERS_DIR"
        rm -rf "$TARGET_HEADERS_DIR"
    fi
done

echo "✅ Successfully built CoveCore.xcframework with combined Swift bindings"
echo "📦 Framework located at: $(pwd)/ios/CoveCore.xcframework"
echo "📄 Swift file created at: $(realpath ../ios/Cove/Cove.swift)"
echo ""
echo "The Swift file contains combined bindings from the following modules:"
for MODULE in $(find ./bindings -name "*.swift" | sort); do
    MODULE_NAME=$(basename "$MODULE" .swift)
    echo "  - $MODULE_NAME"
done

# Integrate with Xcode if requested
if [ "$INTEGRATE" == "--integrate" ] || [ "$INTEGRATE" == "true" ]; then
    echo ""
    echo "Integrating CoveCore.xcframework with Xcode project..."
    cd ..
    ./scripts/add-framework-to-xcode.sh
fi

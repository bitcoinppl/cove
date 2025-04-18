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

# Combine all Swift files into a single file as required by UniFfi
echo "Combining Swift bindings into a single file..."
echo "// Combined Swift bindings for all modules" > ../ios/Cove/Cove.swift

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

# Copy all header files and modulemaps to the bindings directory
echo "Ensuring all header files are in place..."
for HEADER in ./bindings/*.h; do
    echo "Found header: $HEADER"
done

for MODULEMAP in ./bindings/*.modulemap; do
    echo "Found modulemap: $MODULEMAP"
done

# Create a combined module.modulemap if it doesn't exist
if [ ! -f "./bindings/module.modulemap" ]; then
    echo "Creating combined module.modulemap..."
    echo "framework module Cove {" > ./bindings/module.modulemap
    
    # Add all umbrella headers
    for HEADER in ./bindings/*FFI.h; do
        BASE_NAME=$(basename "$HEADER" FFI.h)
        echo "  umbrella header \"${BASE_NAME}FFI.h\"" >> ./bindings/module.modulemap
    done
    
    echo "  export *" >> ./bindings/module.modulemap
    echo "}" >> ./bindings/module.modulemap
fi
 
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

echo "✅ Successfully built Cove.xcframework with combined Swift bindings for cove and tap_card"
echo "📦 Framework located at: $(pwd)/ios/Cove.xcframework"
echo "📄 Swift file created at: $(realpath ../ios/Cove/Cove.swift)"

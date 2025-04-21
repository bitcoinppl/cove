#!/bin/bash
set -e
set -o pipefail

cd rust

BUILD_TYPE=$1
DEVICE=$2
SIGN=$3

# Determine build flags
if [[ "$BUILD_TYPE" == "release" || "$BUILD_TYPE" == "--release" ]]; then
    BUILD_FLAG="--release"
    BUILD_TYPE="release"
elif [[ "$BUILD_TYPE" == "debug" || "$BUILD_TYPE" == "--debug" ]]; then
    BUILD_FLAG=""
    BUILD_TYPE="debug"
else
    BUILD_FLAG="--profile $BUILD_TYPE"
fi

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


## 1. headers, modulemap, and swift sources
OUTPUT_DIR="./bindings"
DYLIB_PATH="./target/debug/libcove.dylib"
mkdir -p "$OUTPUT_DIR" 

rustup target add aarch64-apple-ios-sim
cargo build --target=aarch64-apple-ios-sim 

rm -rf $OUTPUT_DIR || true
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" \
    --swift-sources --headers \
    --modulemap --module-name cove_core_ffi \
    --modulemap-filename module.modulemap


## 2. static library for iOS and iOS simulator
LIBRARY_FLAGS=""
echo "Build for targets: ${TARGETS[@]}"
for TARGET in ${TARGETS[@]}; do
    echo "Building for target: ${TARGET} with build type: ${BUILD_TYPE}"
    LIBRARY_FLAGS="$LIBRARY_FLAGS -library ./target/$TARGET/$BUILD_TYPE/libcove.a -headers ./bindings"

    rustup target add $TARGET
    cargo build --target=$TARGET $BUILD_FLAG
done

# 3. create XCFramework
SPM_PACKAGE="../ios/CoveCore/"
XCFRAMEWORK_OUTPUT="$SPM_PACKAGE/Sources/cove_core_ffi.xcframework"

rm -rf "$XCFRAMEWORK_OUTPUT" || true
xcodebuild -create-xcframework \
        $LIBRARY_FLAGS \
        -output "$XCFRAMEWORK_OUTPUT"

# 4. copy swift sources
cp -r bindings/*.swift $SPM_PACKAGE/Sources/CoveCore/
rm -rf $SPM_PACKAGE/Sources/CoveCore/Package.swift

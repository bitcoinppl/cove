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

# Generate bindings
cargo run --bin uniffi-bindgen generate --library ./target/debug/libcove.dylib --language swift --out-dir ./bindings

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

# Rename *.modulemap to module.modulemap
mv ./bindings/coveFFI.modulemap ./bindings/module.modulemap
 
# Move the Swift file to the project
rm ../ios/Cove/Cove.swift || true
mv ./bindings/cove.swift ../ios/Cove/Cove.swift
 
# Recreate XCFramework
rm -rf "ios/Cove.xcframework" || true
xcodebuild -create-xcframework \
        $LIBRARY_FLAGS \
        -output "ios/Cove.xcframework"
 
# Cleanup
rm -rf bindings

# if [ ! -z $SIGN ] && [ ! -z $SIGNING_IDENTITY ] || [ $SIGN == "--sign" ]; then
#     echo "Signing for distribution: identity: $SIGNING_IDENTITY"
#     codesign --timestamp -v --sign "$SIGNING_IDENTITY" "ios/Cove.xcframework"
# fi

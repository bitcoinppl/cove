#!/bin/bash
set -e
set -o pipefail

cd rust

BUILD_TYPE=$1
DEVICE=$2
SIGN=$3
INTEGRATE=$4

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

# output and temp dirs
SWIFT_SRC_DIR="../ios/CoveCore/Sources/CoveCore"
SWIFT_FRAMEWORK_BUILD_DIR="../build"
XCFRAMEWORK_OUTPUT="../ios/CoveCore.xcframework"

mkdir -p "$OUTPUT_DIR" "$SWIFT_SRC_DIR" "$SWIFT_FRAMEWORK_BUILD_DIR"

# 1. Build Rust library for all targets
TARGETS=(aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim)
for TARGET in "${TARGETS[@]}"; do
    rustup target add $TARGET
    cargo build --target=$TARGET $BUILD_FLAG
done


OUTPUT_DIR="./bindings"
DYLIB_PATH="./target/aarch64-apple-ios-sim/debug/libcove.a"

rustup target add aarch64-apple-ios-sim
cargo build --target=aarch64-apple-ios-sim 

cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" \
    --swift-sources --headers \
    --modulemap --module-name CoveCore \
    --modulemap-filename module.modulemap

cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" \
    --xcframework \
    --modulemap --module-name CoveCore \
    --modulemap-filename xcframework.modulemap

swiftc \
  -emit-module \
  -module-name CoveCore \
  -emit-library -static -o ./bindings/framework/iossim-arm64/libCoveCore.a \
  -emit-module-path ./bindings/framework/iossim-arm64/CoveCore.swiftmodule \
  -emit-objc-header-path ./bindings/framework/iossim-arm64/CoveCore-Swift.h \
  -parse-as-library \
  -I ./bindings \
  -Xcc -fmodule-map-file=./bindings/module.modulemap \
  -L ./target/aarch64-apple-ios-sim/debug \
  -lcove \
  -target arm64-apple-ios18.0-simulator \
  -sdk $(xcrun --sdk iphonesimulator --show-sdk-path) \
  ./bindings/cove.swift ./bindings/rust_cktap.swift ./bindings/tap_card.swift ./bindings/util.swift


xcodebuild -create-xcframework \
  -library ./bindings/framework/iossim-arm64/libCoveCore.a \
  -headers ./bindings/framework/iossim-arm64 \
  -headers ./bindings \
  -output ./bindings/framework/CoveCore.xcframework


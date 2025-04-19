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
OUTPUT_DIR="./bindings"
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

# 2. Generate UniFFI bindings (Swift, headers, modulemap)
DYLIB_PATH="./target/${TARGETS[0]}/$BUILD_TYPE/libcove.dylib"
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --swift-sources
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --headers
cargo run --bin uniffi-bindgen -- "$DYLIB_PATH" "$OUTPUT_DIR" --xcframework --modulemap 

# copy generated Swift files into Swift framework source dir
echo "Copying Swift bindings into Swift framework source directory..."
cp "$OUTPUT_DIR"/*.swift "$SWIFT_SRC_DIR/"

# copy headers and modulemap for C FFI
for TARGET in "${TARGETS[@]}"; do
    TARGET_HEADERS_DIR="$OUTPUT_DIR/$TARGET"
    mkdir -p "$TARGET_HEADERS_DIR"
    cp "$OUTPUT_DIR"/*.h "$TARGET_HEADERS_DIR/"
    cp "$OUTPUT_DIR"/*.modulemap "$TARGET_HEADERS_DIR/" 2>/dev/null || true
done

# 5. Build the Swift framework for all Apple targets
echo "building Swift framework for device and simulator..."
cd ../ios
xcodebuild archive \
    -scheme CoveCore \
    -destination 'generic/platform=iOS' \
    -archivePath "$SWIFT_FRAMEWORK_BUILD_DIR/CoveCore-iOS.xcarchive" \
    SKIP_INSTALL=NO \
    BUILD_LIBRARY_FOR_DISTRIBUTION=YES

xcodebuild archive \
    -scheme CoveCore \
    -destination 'generic/platform=iOS Simulator' \
    -archivePath "$SWIFT_FRAMEWORK_BUILD_DIR/CoveCore-iOSSim.xcarchive" \
    SKIP_INSTALL=NO \
    BUILD_LIBRARY_FOR_DISTRIBUTION=YES

cd ../rust

# create the XCFramework: combine Swift framework and Rust static lib+headers
echo "Creating XCFramework..."
rm -rf "$XCFRAMEWORK_OUTPUT" || true
xcodebuild -create-xcframework \
    -framework "$SWIFT_FRAMEWORK_BUILD_DIR/CoveCore-iOS.xcarchive/Products/Library/Frameworks/CoveCore.framework" \
    -framework "$SWIFT_FRAMEWORK_BUILD_DIR/CoveCore-iOSSim.xcarchive/Products/Library/Frameworks/CoveCore.framework" \
    -library "./target/aarch64-apple-ios/$BUILD_TYPE/libcove.a" -headers "$OUTPUT_DIR/aarch64-apple-ios" \
    -library "./target/aarch64-apple-ios-sim/$BUILD_TYPE/libcove.a" -headers "$OUTPUT_DIR/aarch64-apple-ios-sim" \
    -output "$XCFRAMEWORK_OUTPUT"

# optional: Sign the framework
if [[ ! -z "$SIGN" && "$SIGN" == "--sign" ]]; then
    SIGNING_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | awk '{print $2}')
    if [[ ! -z "$SIGNING_IDENTITY" ]]; then
        echo "Signing for distribution with identity: $SIGNING_IDENTITY"
        codesign --timestamp -v --sign "$SIGNING_IDENTITY" "$XCFRAMEWORK_OUTPUT"
    else
        echo "Warning: No signing identity found. Framework will not be signed."
    fi
fi

echo "✅ Successfully built CoveCore.xcframework"
echo "📦 Framework located at: $(realpath $XCFRAMEWORK_OUTPUT)"

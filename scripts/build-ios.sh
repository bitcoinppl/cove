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

# Compiling Swift modules for Swift bindings
MIN_IOS="12.0"
SWIFT_MODULE_BASE="./swift_modules"
echo "Compiling Swift modules for Swift bindings..."
rm -rf "$SWIFT_MODULE_BASE"
mkdir -p "$SWIFT_MODULE_BASE"

SWIFTMODULE_FLAGS=""
for TARGET in ${TARGETS[@]}; do
    case "$TARGET" in
        aarch64-apple-ios)
            SDK_NAME="iphoneos"
            SWIFT_TRIPLE="arm64-apple-ios${MIN_IOS}"
            ;;
        aarch64-apple-ios-sim)
            SDK_NAME="iphonesimulator"
            SWIFT_TRIPLE="arm64-apple-ios${MIN_IOS}-simulator"
            ;;
        x86_64-apple-ios-sim)
            SDK_NAME="iphonesimulator"
            SWIFT_TRIPLE="x86_64-apple-ios${MIN_IOS}-simulator"
            ;;
        *)
            echo "Error: Unsupported target $TARGET for Swift module compilation"
            exit 1
            ;;
    esac

    SDK_PATH=$(xcrun --sdk "$SDK_NAME" --show-sdk-path)
    MODULE_OUT="$SWIFT_MODULE_BASE/$TARGET"
    mkdir -p "$MODULE_OUT"
    echo "swiftc -sdk $SDK_PATH -target $SWIFT_TRIPLE -emit-module -emit-module-path $MODULE_OUT/CoveBindings.swiftmodule -module-name CoveBindings -I ./bindings ./bindings/*.swift"
    swiftc \
        -sdk "$SDK_PATH" \
        -target "$SWIFT_TRIPLE" \
        -emit-module \
        -emit-module-path "$MODULE_OUT/CoveBindings.swiftmodule" \
        -module-name CoveBindings \
        -I "./bindings" \
        ./bindings/*.swift

    SWIFTMODULE_FLAGS="$SWIFTMODULE_FLAGS -swiftmodule $MODULE_OUT/CoveBindings.swiftmodule"
done

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

  # No longer copying temporary module files; Swift modules are compiled instead
 
# Recreate XCFramework
rm -rf "ios/Cove.xcframework" || true
xcodebuild -create-xcframework \
        $LIBRARY_FLAGS \
        $SWIFTMODULE_FLAGS \
        -output "ios/Cove.xcframework"
 
# Cleanup
# rm -rf bindings

# if [ ! -z $SIGN ] && [ ! -z $SIGNING_IDENTITY ] || [ $SIGN == "--sign" ]; then
#     echo "Signing for distribution: identity: $SIGNING_IDENTITY"
#     codesign --timestamp -v --sign "$SIGNING_IDENTITY" "ios/Cove.xcframework"
# fi

  # Clean up compiled Swift modules directory
rm -rf "$SWIFT_MODULE_BASE"

echo "✅ Successfully built Cove.xcframework with compiled Swift modules"
echo "📦 Framework located at: $(pwd)/ios/Cove.xcframework"
echo ""
echo "✅ Swift modules compiled and embedded in the XCFramework."

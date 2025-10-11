#!/bin/bash
set -e
set -o pipefail

cd rust

################################################################################
############################### ARG PARSING ####################################
################################################################################

BUILD_TYPE=${1:-release}

if [[ "$BUILD_TYPE" == "release" || "$BUILD_TYPE" == "--release-smaller" ]]; then
    BUILD_FLAG="--release"
    BUILD_TYPE="release"
elif [[ "$BUILD_TYPE" == "debug" || "$BUILD_TYPE" == "--debug" ]]; then
    BUILD_FLAG=""
    BUILD_TYPE="debug"
else
    BUILD_FLAG="--profile $BUILD_TYPE"
fi

################################################################################
############################### PREP WORK ######################################
################################################################################

if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "cargo-ndk not found, installing..."
    cargo install cargo-ndk
fi

TARGETS=(
    aarch64-linux-android
    x86_64-linux-android
)

declare -A ABI_DIRS=(
    [aarch64-linux-android]=arm64-v8a
    [x86_64-linux-android]=x86_64
)

JNI_LIBS_DIR="../android/app/src/main/jniLibs"
ANDROID_KOTLIN_DIR="../android/app/src/main/java"
BINDINGS_DIR="./bindings/kotlin"

mkdir -p "$JNI_LIBS_DIR"
mkdir -p "$ANDROID_KOTLIN_DIR"
rm -rf "$BINDINGS_DIR" || true
mkdir -p "$BINDINGS_DIR"

################################################################################
############################### BUILDING #######################################
################################################################################

export CFLAGS="-D__ANDROID_MIN_SDK_VERSION__=21"
for TARGET in "${TARGETS[@]}"; do
    echo "Building for target: ${TARGET} with build type: ${BUILD_TYPE}"
    rustup target add "$TARGET"
    cargo ndk --target "$TARGET" build $BUILD_FLAG

    TARGET_DIR="./target/$TARGET/$BUILD_TYPE"
    DYNAMIC_LIB_PATH="$TARGET_DIR/libcove.so"
    if [[ ! -f "$DYNAMIC_LIB_PATH" ]]; then
        echo "Missing dynamic library at $DYNAMIC_LIB_PATH" >&2
        exit 1
    fi

    ABI="${ABI_DIRS[$TARGET]}"
    if [[ -z "$ABI" ]]; then
        echo "Unable to map target $TARGET to an Android ABI directory" >&2
        exit 1
    fi

    mkdir -p "$JNI_LIBS_DIR/$ABI"
    cp "$DYNAMIC_LIB_PATH" "$JNI_LIBS_DIR/$ABI/libcoveffi.so"
done

################################################################################
############################### BINDINGS #######################################
################################################################################

DYNAMIC_LIB_PATH="./target/${TARGETS[0]}/$BUILD_TYPE/libcove.so"
if [[ ! -f "$DYNAMIC_LIB_PATH" ]]; then
    echo "Missing dynamic library at $DYNAMIC_LIB_PATH" >&2
    exit 1
fi

echo "Generating Kotlin bindings into $BINDINGS_DIR"
cargo run -p uniffi_cli \
    -- generate "$DYNAMIC_LIB_PATH" \
    --library \
    --language kotlin \
    --no-format \
    --out-dir "$BINDINGS_DIR"

echo "Copying Kotlin bindings into Android project at $ANDROID_KOTLIN_DIR"
# remove only generated binding files, not user code
rm -rf "$ANDROID_KOTLIN_DIR/org/bitcoinppl/cove_core"
cp -R "$BINDINGS_DIR"/. "$ANDROID_KOTLIN_DIR"/

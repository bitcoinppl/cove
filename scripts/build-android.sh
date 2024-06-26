#!/bin/bash
set -ex
cd rust
 
# Set up cargo-ndk and add the Android targets
cargo install cargo-ndk
rustup target add aarch64-linux-android \
    armv7-linux-androideabi \
    i686-linux-android \
    x86_64-linux-android
 
# Build the dylib
cargo build

# Build the Android libraries in jniLibs for arm64
export CFLAGS="-D__ANDROID_MIN_SDK_VERSION__=21"
cargo ndk --target aarch64-linux-android build --release


# Build the Android libraries in jniLibs for x86_64
export CFLAGS="-D__ANDROID_MIN_SDK_VERSION__=21"
cargo ndk --target x86_64-linux-android build --release 

# Copy jnilibs to expected location
mkdir -p ../android/app/src/main/jniLibs/arm64-v8a
mkdir -p ../android/app/src/main/jniLibs/x86_64

cp target/aarch64-linux-android/release/libcove.so ../android/app/src/main/jniLibs/arm64-v8a/libcoveffi.so
cp target/x86_64-linux-android/release/libcove.so ../android/app/src/main/jniLibs/x86_64/libcoveffi.so

# Create Kotlin bindings
cargo run --bin uniffi-bindgen generate \
    --library target/aarch64-linux-android/release/libcove.so \
    --language kotlin \
    --out-dir ../android/app/src/main/java \

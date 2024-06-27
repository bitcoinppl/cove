default:
    just --list

bacon:
    cd rust && bacon clippy

clean:
    cargo clean
    rm -rf ios/Cove.xcframework
    rm -rf ios/Cove
    rf -rf rust/target

fmt:
    cd rust && cargo fmt

build-android:
    bash scripts/build-android.sh

run-android: build-android
    bash scripts/run-android.sh

build-ios profile="debug":
    bash scripts/build-ios.sh {{profile}}

run-ios: build-ios
    bash scripts/run-ios.sh

watch: 
    watchexec --exts rs just build-ios

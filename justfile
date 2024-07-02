default:
    just --list

bacon:
    cd rust && bacon clippy

clean:
    cd rust && cargo clean && \
    rm -rf ios/Cove.xcframework && \
    rm -rf ios/Cove && \
    rm -rf rust/target

fmt:
    cd rust && cargo fmt

build-android:
    bash scripts/build-android.sh

run-android: build-android
    bash scripts/run-android.sh

build-ios profile="debug" device="false":
    bash scripts/build-ios.sh {{profile}} {{device}}

run-ios: build-ios
    bash scripts/run-ios.sh

watch profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

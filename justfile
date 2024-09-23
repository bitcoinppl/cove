alias rx := reset-xcode
alias cx := clean-xcode

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

clippy:
    cd rust && cargo clippy

update pkg="":
    cd rust && cargo update {{pkg}}

clean-xcode:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    cd ios && xcodebuild clean

reset-xcode:
    killAll Xcode || true
    rm -rf ios/Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    cd ios && xcodebuild clean
    open ios/Cove.xcodeproj

build-android:
    bash scripts/build-android.sh

run-android: build-android
    bash scripts/run-android.sh

build-ios-device:
    bash scripts/build-ios.sh release-smaller --device false

build-ios profile="debug" device="false" sign="false":
    bash scripts/build-ios.sh {{profile}} {{device}} {{sign}}

run-ios: build-ios
    bash scripts/run-ios.sh

watch profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

ci:
    cd rust && cargo fmt --check
    cd rust && cargo clippy --all-targets --all-features -- -D warnings


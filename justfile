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

clean-xcode:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    cd ios && xcodebuild clean

reset-xcode:
    killAll Xcode
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

build-ios profile="debug" device="false":
    bash scripts/build-ios.sh {{profile}} {{device}}

run-ios: build-ios
    bash scripts/run-ios.sh

watch profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

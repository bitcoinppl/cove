# xcode aliases
alias xc := xcode-clean
alias xr := xcode-reset

# watch aliases
alias wt := watch-test
alias wb := watch-build

# build aliases
alias bi := build-ios
alias bir := build-ios-release
alias bidd := build-ios-debug-device

default:
    just --list

bacon:
    cd rust && bacon clippy

bcheck:
    cd rust && bacon check

check:
    cd rust && cargo check

clean:
    cd rust && cargo clean && \
    rm -rf ios/Cove.xcframework && \
    rm -rf ios/Cove && \
    rm -rf rust/target

fmt:
    cd rust && cargo fmt --all

clippy:
    cd rust && cargo clippy

update pkg="":
    cd rust && cargo update {{pkg}}

xcode-clean:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    cd ios && xcodebuild clean

ci: 
    just fmt
    just clippy
    just test
    cd rust && cargo clippy --all-targets --all-features -- -D warnings
    cd rust && cargo fmt --check

xcode-reset:
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

build-ios profile="debug" device="false" sign="false":
    bash scripts/build-ios.sh {{profile}} {{device}} {{sign}}

build-ios-release:
    just build-ios release-smaller --device

build-ios-debug-device:
    just build-ios debug --device

run-ios: build-ios
    bash scripts/run-ios.sh

watch-build profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

test test="":
    cd rust && cargo nextest run {{test}}

ctest test="":
    cd rust && cargo test {{test}}

btest test="":
    cd rust && bacon nextest {{test}}

watch-test test="":
    watchexec --exts rs just test {{test}}

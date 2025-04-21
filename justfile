export TMPDIR := "/tmp"

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
    swiftformat . --swiftversion 6 

clippy *flags="":
    cd rust && cargo clippy {{flags}}

update pkg="":
    cd rust && cargo update {{pkg}}

xcode-clean:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    cd ios && xcodebuild clean

ci: 
    just fmt
    cd rust && cargo clippy --all-targets --all-features
    just test
    cd rust && cargo clippy --all-targets --all-features -- -D warnings
    cd rust && cargo fmt --check
    swiftformat --lint . --swiftversion 6 

xcode-reset:
    killAll Xcode || true
    rm -rf ios/Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    cd ios && xcodebuild clean
    cd ios && xcode-build-server config -project *.xcodeproj -scheme Cove
    open ios/Cove.xcodeproj

build-android:
    bash scripts/build-android.sh

run-android: build-android
    bash scripts/run-android.sh

build-ios profile="debug" device="false" sign="false":
    #!/usr/bin/env bash
    if bash scripts/build-ios.sh {{profile}} {{device}} {{sign}}; then
        say "done"
    else
        say "error"
    fi

build-ios-release:
    just build-ios release-smaller --device

build-ios-debug-device:
    just build-ios debug --device

run-ios: build-ios
    bash scripts/run-ios.sh

watch-build profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

test test="" flags="":
    cd rust && cargo nextest run {{test}} -- {{flags}}

ctest test="" flags="":
    cd rust && cargo test {{test}} -- {{flags}}

btest test="":
    cd rust && bacon nextest -- {{test}}

watch-test test="" flags="":
    watchexec --exts rs just test {{test}} {{flags}}

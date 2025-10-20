export TMPDIR := "/tmp"

# xcode aliases
alias xc := xcode-clean
alias xr := xcode-reset

# watch aliases
alias wt := watch-test
alias wtest := watch-test
alias wb := watch-build

# build aliases ios
alias bi := build-ios
alias bir := build-ios-release
alias bidd := build-ios-debug-device

# build aliases android
alias ba := build-android
alias bar := build-android-release

default:
    just --list

bacon:
    cd rust && bacon clippy

bcheck:
    cd rust && bacon check

check *flags="--workspace --all-targets --all-features":
    cd rust && cargo check {{flags}}

clean:
    cd rust && cargo clean && \
    rm -rf ios/Cove.xcframework && \
    rm -rf ios/Cove && \
    rm -rf rust/target

fmt:
    just fmt-rust && just fmt-swift && just fmt-android

fmt-rust:
    cd rust && cargo fmt --all

fmt-swift:
    swiftformat . --swiftversion 6

fmt-android:
    cd android && ./gradlew ktlintFormat 

fix *flags="":
    cd rust && cargo fix --workspace {{flags}}

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
    cd android && ./gradlew ktlintCheck

xcode-reset:
    killAll Xcode || true
    rm -rf ios/Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    cd ios && xcodebuild clean
    cd ios && xcode-build-server config -project *.xcodeproj -scheme Cove
    open ios/Cove.xcodeproj

watch-build profile="debug" device="false":
    watchexec --exts rs just build-ios {{profile}} {{device}}

test test="" flags="":
    cd rust && cargo nextest run {{test}} --workspace {{flags}}

ctest test="" flags="":
    cd rust && cargo test {{test}} --workspace -- {{flags}} 

btest test="":
    cd rust && bacon nextest -- {{test}} --workspace

watch-test test="" flags="":
    watchexec --exts rs just test {{test}} {{flags}}

# both
compile:
    just compile-ios && just compile-android

# build android
build-android:
    bash scripts/build-android.sh debug

build-android-release:
    bash scripts/build-android.sh release

run-android: build-android
    bash scripts/run-android.sh

compile-android:
    cd android && ./gradlew assembleDebug

# build ios
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

compile-ios:
    cd ios && xcodebuild -scheme Cove -sdk iphonesimulator -arch arm64 build


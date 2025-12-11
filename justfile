# Commonly used commands:
#   just ba         - build android debug
#   just bar        - build android release
#   just bi         - build ios debug simulator
#   just bir        - build ios release
#   just bidd       - build ios debug device
#   just bb [ios|android] - bump build numbers only (default: both)
#   just bump <major|minor|patch> [targets] - bump version (default: all targets)
#   just f          - full build and verification (all platforms)
#   just ci         - run all CI checks

default:
    just --list

# full build and verification for all platforms
alias f := full
full:
    just bidd && just ba && just ci && just compile

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
    swiftformat ios --swiftversion 6

fmt-android:
    cd android && ./gradlew ktlintFormat 

fix *flags="":
    cd rust && cargo fix --workspace {{flags}}

clippy *flags="":
    cd rust && cargo clippy {{flags}}

# lint all platforms

lint *flags="":
    just lint-rust {{flags}} && just lint-swift {{flags}} && just lint-android {{flags}}

lint-rust *flags="":
    cd rust && cargo clippy --all-targets --all-features -- -D warnings {{flags}}

lint-android *flags="":
    cd android && ./gradlew ktlintCheck {{flags}}

lint-swift *flags="":
    swiftformat --lint ios --swiftversion 6 {{flags}}

update pkg="":
    cd rust && cargo update {{pkg}}

bump type targets="rust,ios,android":
    cd rust && cargo xtask bump-version {{type}} --targets {{targets}}

# bump build numbers only (ios, android, or both)
alias bb := build-bump
build-bump targets="ios,android":
    cd rust && cargo xtask build-bump {{targets}}


alias xc := xcode-clean
xcode-clean:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    cd ios && xcodebuild clean

ci:
    just fmt && \
    cd rust && cargo fmt --check && \
    just lint && \
    just test && \
    just compile

alias xr := xcode-reset
xcode-reset:
    killAll Xcode || true
    rm -rf ios/Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    cd ios && xcodebuild clean
    cd ios && xcode-build-server config -project *.xcodeproj -scheme Cove
    open ios/Cove.xcodeproj

alias wb := watch-build
watch-build profile="debug" *flags="":
    watchexec --exts rs just build-ios {{profile}} {{flags}}

test test="" flags="":
    cd rust && cargo nextest run {{test}} --workspace {{flags}}

ctest test="" flags="":
    cd rust && cargo test {{test}} --workspace -- {{flags}} 

btest test="":
    cd rust && bacon nextest -- {{test}} --workspace

alias wt := watch-test
alias wtest := watch-test
watch-test test="" flags="":
    watchexec --exts rs just test {{test}} {{flags}}

# both
compile:
    just compile-ios && just compile-android

# build android
alias ba := build-android
build-android:
    cd rust && cargo xtask build-android debug && just say "done android"

alias bar := build-android-release
build-android-release:
    cd rust && cargo xtask build-android release-speed && just say "done android release"

alias ra := run-android
run-android profile="debug":
    cd rust && cargo xtask run-android {{profile}}

compile-android:
    cd android && ./gradlew assembleDebug

# build ios
alias bi := build-ios
build-ios profile="debug" *flags="":
    cd rust && cargo xtask build-ios {{profile}} {{flags}} && just say "done ios"

alias bir := build-ios-release
build-ios-release:
    cd rust && cargo xtask build-ios release-speed --device && just say "done ios release"

alias bidd := build-ios-debug-device
build-ios-debug-device:
    cd rust && cargo xtask build-ios debug --device && just say "done ios device"

run-ios:
    cd rust && cargo xtask run-ios

compile-ios:
    cd ios && xcodebuild -scheme Cove -sdk iphonesimulator -arch arm64 build

xtask *args:
    cd rust && cargo xtask {{args}}


# helpers
say *args:
    @say args || @echo {{args}}


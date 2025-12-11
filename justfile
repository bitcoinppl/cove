# List available recipes
[default]
list:
    @just --list

# ------------------------------------------------------------------------------
# ci
# ------------------------------------------------------------------------------

# Full build and verification for all platforms
[group('ci')]
full:
    just bidd && just ba && just ci && just compile

[private]
alias f := full

# Run all CI checks
[group('ci')]
[script('bash')]
[working-directory: 'rust']
ci:
    set -e
    just fmt
    cargo fmt --check
    just lint
    just test
    just compile

# ------------------------------------------------------------------------------
# build
# ------------------------------------------------------------------------------

# Build Android debug APK
[group('build')]
[working-directory: 'rust']
build-android:
    cargo xtask build-android debug && just say "done android"

[private]
alias ba := build-android

# Build Android release APK
[group('build')]
[working-directory: 'rust']
build-android-release:
    cargo xtask build-android release-speed && just say "done android release"

[private]
alias bar := build-android-release

# Build iOS debug for simulator
[group('build')]
[working-directory: 'rust']
build-ios profile="debug" *flags="":
    cargo xtask build-ios {{profile}} {{flags}} && just say "done ios"

[private]
alias bi := build-ios

# Build iOS release for device
[group('build')]
[working-directory: 'rust']
build-ios-release:
    cargo xtask build-ios release-speed --device && just say "done ios release"

[private]
alias bir := build-ios-release

# Build iOS debug for device
[group('build')]
[working-directory: 'rust']
build-ios-debug-device:
    cargo xtask build-ios debug --device && just say "done ios device"

[private]
alias bidd := build-ios-debug-device

# Compile both iOS and Android
[group('build')]
@compile:
    just compile-ios && just compile-android

# Compile iOS for simulator
[group('build')]
[working-directory: 'ios']
compile-ios:
    xcodebuild -scheme Cove -sdk iphonesimulator -arch arm64 build

# Compile Android debug
[group('build')]
[working-directory: 'android']
compile-android:
    ./gradlew assembleDebug

# ------------------------------------------------------------------------------
# test
# ------------------------------------------------------------------------------

# Run all tests
[group('test')]
[working-directory: 'rust']
test test="" flags="":
    cargo nextest run {{test}} --workspace {{flags}}

# Run tests with cargo test
[group('test')]
[working-directory: 'rust']
ctest test="" flags="":
    cargo test {{test}} --workspace -- {{flags}}

# Run tests with bacon
[group('test')]
[working-directory: 'rust']
btest test="":
    bacon nextest -- {{test}} --workspace

# Watch and re-run tests on file changes
[group('test')]
watch-test test="" flags="":
    watchexec --exts rs just test {{test}} {{flags}}

[private]
alias wt := watch-test
[private]
alias wtest := watch-test

# ------------------------------------------------------------------------------
# lint
# ------------------------------------------------------------------------------

# Lint all platforms
[group('lint')]
@lint *flags="":
    just lint-rust {{flags}} && just lint-swift {{flags}} && just lint-android {{flags}}

# Lint Rust code
[group('lint')]
[working-directory: 'rust']
lint-rust *flags="":
    cargo clippy --all-targets --all-features -- -D warnings {{flags}}

# Lint Android code
[group('lint')]
[working-directory: 'android']
lint-android *flags="":
    ./gradlew ktlintCheck {{flags}}

# Lint Swift code
[group('lint')]
lint-swift *flags="":
    swiftformat --lint ios --swiftversion 6 {{flags}}

# Run clippy
[group('lint')]
[working-directory: 'rust']
clippy *flags="":
    cargo clippy {{flags}}

# ------------------------------------------------------------------------------
# format
# ------------------------------------------------------------------------------

# Format all platforms
[group('format')]
@fmt:
    just fmt-rust && just fmt-swift && just fmt-android

# Format Rust code
[group('format'), private]
[working-directory: 'rust']
fmt-rust:
    cargo fmt --all

# Format Swift code
[group('format'), private]
fmt-swift:
    swiftformat ios --swiftversion 6

# Format Android code
[group('format'), private]
[working-directory: 'android']
fmt-android:
    ./gradlew ktlintFormat

# ------------------------------------------------------------------------------
# dev
# ------------------------------------------------------------------------------

# Run bacon clippy watcher
[group('dev')]
[working-directory: 'rust']
bacon:
    bacon clippy

# Run bacon check watcher
[group('dev')]
[working-directory: 'rust']
bcheck:
    bacon check

# Run cargo check
[group('dev')]
[working-directory: 'rust']
check *flags="--workspace --all-targets --all-features":
    cargo check {{flags}}

# Watch and rebuild iOS on file changes
[group('dev')]
watch-build profile="debug" *flags="":
    watchexec --exts rs just build-ios {{profile}} {{flags}}

[private]
alias wb := watch-build

# Apply cargo fix
[group('dev')]
[working-directory: 'rust']
fix *flags="":
    cargo fix --workspace {{flags}}

# ------------------------------------------------------------------------------
# release
# ------------------------------------------------------------------------------

# Bump version (type: major, minor, patch)
[group('release')]
[working-directory: 'rust']
bump type targets="rust,ios,android":
    cargo xtask bump-version {{type}} --targets {{targets}}

# Bump build numbers only
[group('release')]
[working-directory: 'rust']
build-bump targets="ios,android":
    cargo xtask build-bump {{targets}}

[private]
alias bb := build-bump

# ------------------------------------------------------------------------------
# xcode
# ------------------------------------------------------------------------------

# Clean Xcode caches
[group('xcode')]
[working-directory: 'ios']
xcode-clean:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    xcodebuild clean

[private]
alias xc := xcode-clean

# Reset Xcode completely
[group('xcode')]
[confirm("This will kill Xcode and delete caches. Continue?")]
[script('bash')]
[working-directory: 'ios']
xcode-reset:
    killAll Xcode || true
    rm -rf Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    xcodebuild clean
    xcode-build-server config -project *.xcodeproj -scheme Cove
    open Cove.xcodeproj

[private]
alias xr := xcode-reset

# ------------------------------------------------------------------------------
# util
# ------------------------------------------------------------------------------

# Clean all build artifacts
[group('util')]
[confirm("Delete all build artifacts?")]
[script('bash')]
[working-directory: 'rust']
clean:
    cargo clean
    rm -rf ../ios/Cove.xcframework
    rm -rf ../ios/Cove
    rm -rf target

# Update cargo dependencies
[group('util')]
[working-directory: 'rust']
update pkg="":
    cargo update {{pkg}}

# Run Android app
[group('util')]
[working-directory: 'rust']
run-android profile="debug":
    cargo xtask run-android {{profile}}

[private]
alias ra := run-android

# Run iOS app
[group('util')]
[working-directory: 'rust']
run-ios:
    cargo xtask run-ios

# Run xtask commands
[group('util')]
[working-directory: 'rust']
xtask *args:
    cargo xtask {{args}}

# ------------------------------------------------------------------------------
# helpers
# ------------------------------------------------------------------------------

# text-to-speech helper
[private]
say *args:
    @say args || @echo {{args}} || echo {{args}} || true

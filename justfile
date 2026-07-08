# List available recipes
[default]
list:
    @just --list

# ------------------------------------------------------------------------------
# utilities
# ------------------------------------------------------------------------------

# Run an xtask command
[group('utils')]
xtask *args:
    cd rust && cargo xtask {{ args }}

# Rebase current branch onto new-base after choosing the old squash-merged base
[group('utils')]
rebase new_base="master":
    just xtask rebase "{{ new_base }}"

# Sign a PSBT and output all formats (base64, hex, binary, bbqr-gif, ur-gif)
# Requires MNEMONIC env var (set in .envrc or pass directly)
[group('utils')]
[script('bash')]
sign-psbt psbt:
    set -e
    if [ -z "$MNEMONIC" ]; then
        echo "Error: MNEMONIC env var not set. Set it in .envrc or export it."
        exit 1
    fi
    OUTPUT_DIR="$HOME/Downloads/signed-psbt-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$OUTPUT_DIR"
    echo "Signing PSBT and outputting to: $OUTPUT_DIR"
    cd rust
    cargo xtask util sign-psbt --psbt "{{ psbt }}" -f base64 -O "$OUTPUT_DIR/signed.base64.txt"
    cargo xtask util sign-psbt --psbt "{{ psbt }}" -f hex -O "$OUTPUT_DIR/signed.hex.txt"
    cargo xtask util sign-psbt --psbt "{{ psbt }}" -f binary -O "$OUTPUT_DIR/signed.psbt"
    cargo xtask util sign-psbt --psbt "{{ psbt }}" -f bbqr-gif -O "$OUTPUT_DIR/signed-bbqr.gif"
    cargo xtask util sign-psbt --psbt "{{ psbt }}" -f ur-gif -O "$OUTPUT_DIR/signed-ur.gif"
    echo ""
    echo "All formats saved to: $OUTPUT_DIR"
    ls -la "$OUTPUT_DIR"
    open "$OUTPUT_DIR"

alias sp := sign-psbt

# ------------------------------------------------------------------------------
# ci
# ------------------------------------------------------------------------------

# Full build and verification for all platforms
[group('ci')]
full:
    just bidd && just ba && just ci && just compile

alias f := full

# Run all CI checks
[group('ci')]
[script('bash')]
[working-directory('rust')]
ci:
    set -e
    just fmt
    cargo fmt --check
    just lint
    just compile
    just test

# Regenerate UniFFI bindings in GitHub Actions for committed branch changes
[group('ci')]
regenerate-bindings:
    just xtask regenerate-bindings

alias rb := regenerate-bindings

# Run mobile artifact producer or consumer commands
[group('ci')]
mobile-artifact *args:
    just xtask mobile-artifact {{ args }}

# Trigger Android and iOS mobile artifacts for a pushed ref
[group('ci')]
mobile-artifact-trigger ref platform="both":
    just mobile-artifact trigger --platform "{{ platform }}" --ref "{{ ref }}"

# Install latest matching Android mobile artifact
[group('ci')]
mobile-artifact-install-android ref:
    just mobile-artifact install-android --ref "{{ ref }}"

# Fetch latest matching iOS CoveCore mobile artifact
[group('ci')]
mobile-artifact-fetch-ios-core ref:
    just mobile-artifact fetch-ios-core --ref "{{ ref }}"

# ------------------------------------------------------------------------------
# build
# ------------------------------------------------------------------------------

# Build Android debug Rust FFI and Kotlin bindings for all ABIs
[group('build')]
build-android:
    just xtask build-android debug && just say "done android"

alias ba := build-android

# Build Android debug Rust FFI and Kotlin bindings for the connected device ABI
[group('build')]
build-android-connected-device:
    just xtask build-android debug --connected-device && just say "done android connected device"

alias bad := build-android-connected-device

# Build Android release APK
[group('build')]
build-android-release:
    just xtask build-android release-speed && just say "done android release"

alias bar := build-android-release

# Build signed AAB for Google Play, copy to Downloads
[group('build')]
bundle-android: build-android-release
    cd android && ./gradlew --stop
    just xtask bundle-android && just say "done android bundle"

alias bua := bundle-android

# Build iOS debug for simulator
[group('build')]
build-ios profile="debug" *flags="":
    just xtask build-ios {{ profile }} {{ flags }} && just say "done ios"

alias bi := build-ios

# Build iOS release for device
[group('build')]
build-ios-release:
    just xtask build-ios release-speed --device && just say "done ios release"

alias bir := build-ios-release

# Bump iOS build, build release bindings, then upload to TestFlight
# keep this path aligned with Xcode archives; passkeys fail in TestFlight if CLI signing diverges
# xtask verifies Apple's AASA CDN before upload
[group('build')]
testflight:
    just xtask testflight

# Build iOS debug for device
[group('build')]
build-ios-debug-device:
    just xtask build-ios debug --device && just say "done ios device"

alias bidd := build-ios-debug-device

alias gen-swift := build-ios

# Compile both iOS and Android
[group('build')]
@compile:
    just compile-ios && just compile-android

# Compile iOS for simulator
[group('build')]
[working-directory('ios')]
compile-ios:
    xcodebuild -scheme Cove -sdk iphonesimulator -arch arm64 build && just notf "done compile ios"

# Compile Android debug
[group('build')]
[working-directory('android')]
compile-android:
    ./gradlew assembleDevDebug && just notf "done compile android"

# ------------------------------------------------------------------------------
# test
# ------------------------------------------------------------------------------

# run ios and android manual ui tests
[group('test')]
ui-manual:
    just android-ui-manual && just ios-ui-background

# Run an Android device command from android/ with stay-awake enabled.
#
# Use this for ad hoc device UI testing:
#   just android-stay-awake ./gradlew :app:connectedUiTestDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.annotation=org.bitcoinppl.cove.test.LayoutRegressionTest
#
# Focused instrumentation tests should use AndroidDeviceStayAwakeRule when the
# stay-awake behavior belongs in the test itself.
[group('test')]
android-stay-awake *command:
    just xtask android-stay-awake -- {{ command }}

# Run manual Android full-launch onboarding UI tests.
[group('test')]
android-ui-manual:
    just xtask android-ui-manual

alias aum := android-ui-manual

# Update Android Compose preview screenshot references
[group('test')]
[working-directory('android')]
android-preview-screenshots-update:
    ./gradlew :app:updateDevDebugScreenshotTest

# Validate Android Compose preview screenshots
[group('test')]
[working-directory('android')]
android-preview-screenshots-validate:
    ./gradlew :app:validateDevDebugScreenshotTest

# Run manual iOS full-launch UI tests without opening Simulator
[group('test')]
[script('bash')]
ios-ui-background device="iPhone 17" test="CoveUITests/OnboardingFullLaunchUITests":
    cd rust && cargo build --package xtask -q && ./target/debug/xtask ios-ui --device "{{ device }}" --test "{{ test }}"

alias iub := ios-ui-background

# Run manual iOS full-launch UI tests with Simulator visible
[group('test')]
[script('bash')]
ios-ui-foreground device="iPhone 17" test="CoveUITests/OnboardingFullLaunchUITests":
    cd rust && cargo build --package xtask -q && ./target/debug/xtask ios-ui --foreground --device "{{ device }}" --test "{{ test }}"

alias iuf := ios-ui-foreground

# Run all tests
[group('test')]
[working-directory('rust')]
test test="" flags="":
    cargo nextest run {{ test }} --workspace {{ flags }}

# Run tests the same way as GitHub Actions
[group('test')]
[working-directory('rust')]
test-gh test="" flags="":
    cargo test {{ test }} --workspace {{ flags }}

# Run tests with cargo test
[group('test')]
[working-directory('rust')]
ctest test="" flags="":
    cargo test {{ test }} --workspace -- {{ flags }}

# Run tests with bacon
[group('test')]
[working-directory('rust')]
btest test="":
    bacon nextest -- {{ test }} --workspace

# Watch and re-run tests on file changes
[group('test')]
watch-test test="" flags="":
    watchexec --exts rs just test {{ test }} {{ flags }}

alias wt := watch-test
alias wtest := watch-test

# ------------------------------------------------------------------------------
# lint
# ------------------------------------------------------------------------------

# Lint all platforms
[group('lint')]
@lint *flags="":
    just lint-rust {{ flags }} && just lint-swift {{ flags }} && just lint-android {{ flags }}

# Lint Rust code
[group('lint')]
[working-directory('rust')]
lint-rust *flags="":
    cargo clippy --all-targets --all-features -- -D warnings {{ flags }}

# Lint Android code
[group('lint')]
[working-directory('android')]
lint-android *flags="":
    ./gradlew ktlintCheck {{ flags }}
    ./gradlew detekt

# Lint Swift code
[group('lint')]
lint-swift *flags="":
    swiftformat --lint ios --swiftversion 6 {{ flags }}

# Run clippy
[group('lint')]
[working-directory('rust')]
clippy *flags="":
    cargo clippy {{ flags }}

# Run pedantic clippy checks (excluding must_use, truncation, single_match, if_not_else, needless_continue, option_if_let_else)
[group('lint')]
[working-directory('rust')]
pedantic *flags="":
    cargo clippy -- -D clippy::pedantic -D clippy::nursery \
        -A clippy::must_use_candidate \
        -A clippy::cast_possible_truncation \
        -A clippy::single_match \
        -A clippy::if_not_else \
        -A clippy::needless_continue \
        -A clippy::option_if_let_else \
        -A clippy::unused_self \
        -A clippy::unused_async \
        -A clippy::significant_drop_tightening \
        -A clippy::missing_const_for_fn \
        -A clippy::needless_pass_by_value \
        {{ flags }}

# Run full pedantic clippy checks without any allows
[group('lint')]
[working-directory('rust')]
pedantic-all *flags="":
    cargo clippy -- -D clippy::pedantic -D clippy::nursery {{ flags }}

# ------------------------------------------------------------------------------
# format
# ------------------------------------------------------------------------------

# Format all platforms
[group('format')]
@fmt:
    just fmt-rust && just fmt-swift && just fmt-android

# Format Rust code
[group('format')]
[private]
[working-directory('rust')]
fmt-rust:
    cargo fmt --all

# Format Swift code
alias fmt-ios := fmt-swift
alias fi := fmt-swift

[group('format')]
[private]
fmt-swift:
    swiftformat ios --swiftversion 6

# Format Android code
[group('format')]
[private]
[working-directory('android')]
fmt-android:
    ./gradlew ktlintFormat

# ------------------------------------------------------------------------------
# dev
# ------------------------------------------------------------------------------

# Run bacon clippy watcher
[group('dev')]
[working-directory('rust')]
bacon:
    bacon clippy

# Run bacon check watcher
[group('dev')]
[working-directory('rust')]
bcheck:
    bacon check

# Run cargo check
[group('dev')]
[working-directory('rust')]
check *flags="--workspace --all-targets --all-features":
    cargo check {{ flags }}

# Watch and rebuild iOS on file changes
[group('dev')]
watch-build profile="debug" *flags="":
    watchexec --exts rs just build-ios {{ profile }} {{ flags }}

alias wb := watch-build

# Apply cargo fix
[group('dev')]
[working-directory('rust')]
fix *flags="":
    cargo fix --workspace {{ flags }}

# ------------------------------------------------------------------------------
# release
# ------------------------------------------------------------------------------

# Bump version (type: major, minor, patch, build)
[group('release')]
bump type targets="":
    just xtask bump-version {{ type }} {{ if targets != "" { "--targets " + targets } else { "" } }}

# ------------------------------------------------------------------------------
# xcode
# ------------------------------------------------------------------------------

# Clean Xcode caches
[group('xcode')]
[working-directory('ios')]
xcode-clean:
    rm -rf ~/Library/Caches/org.swift.swiftpm
    xcodebuild clean

alias xc := xcode-clean

# Reset Xcode completely
[confirm("This will kill Xcode and delete caches. Continue?")]
[group('xcode')]
[script('bash')]
[working-directory('ios')]
xcode-reset:
    killAll Xcode || true
    rm -rf Cove.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
    xcrun simctl --set previews delete all
    rm -rf ~/Library/Caches/org.swift.swiftpm
    rm -rf ~/Library/Developer/Xcode/DerivedData
    xcodebuild clean
    xcode-build-server config -project *.xcodeproj -scheme Cove
    open Cove.xcodeproj

alias xr := xcode-reset

# ------------------------------------------------------------------------------
# util
# ------------------------------------------------------------------------------

# Clean all build artifacts
[confirm("Delete all build artifacts?")]
[group('util')]
[script('bash')]
[working-directory('rust')]
clean:
    cargo clean
    rm -rf ../ios/Cove.xcframework
    rm -rf ../ios/Cove
    rm -rf target

# Update cargo dependencies
[group('util')]
[working-directory('rust')]
update pkg="":
    cargo update {{ pkg }}

# Run Android app
[group('util')]
run-android profile="debug":
    just xtask run-android {{ profile }} && just notf "done run android"

alias ra := run-android

# Rebuild Android bindings, then install and run the Android app
[group('util')]
build-run-android:
    just ba && just ra

alias bra := build-run-android

# Rebuild, install, and run iOS and Android apps
[group('util')]
build-run-all:
    just bri && just bra

alias brall := build-run-all

# Launch installed Android app
[group('util')]
launch-android:
    adb shell am start -W -n org.bitcoinppl.cove.dev/org.bitcoinppl.cove.MainActivity

alias la := launch-android

# Clear Android app data and launch installed app
[group('util')]
reset-run-android:
    just reset-android
    just launch-android

[group('util')]
reset-android:
    adb shell pm clear org.bitcoinppl.cove.dev

alias rra := reset-run-android
alias rea := reset-android

# Download Android screenshots into _scratch and delete them from the device
[group('util')]
download-android-screenshots:
    just xtask download-android-screenshots

alias das := download-android-screenshots

# Build and clean install Android (rebuilds native libs, clears Gradle cache)
[group('util')]
[working-directory('android')]
install-android-clean:
    just ba && ./gradlew --stop && ./gradlew clean installDebug && just notf "done install android clean"

alias iac := install-android-clean

# Run iOS app with existing generated bindings
[group('util')]
run-ios *args:
    just xtask run-ios {{ args }} && just notf "done run ios"

alias ri := run-ios

# Rebuild iOS bindings, then install and run the iOS app
[group('util')]
build-run-ios *args:
    just xtask build-run-ios {{ args }} && just notf "done build run ios"

alias bri := build-run-ios
alias ib := build-run-ios

# Show logcat for cove process
[group('util')]
logcat:
    #!/usr/bin/env bash
    pid=$(adb shell pidof org.bitcoinppl.cove.dev | tr -d '[:space:]')
    if [ -z "$pid" ]; then
        echo "error: org.bitcoinppl.cove.dev is not running" >&2
        exit 1
    fi
    adb logcat --pid="$pid"

# ------------------------------------------------------------------------------
# helpers
# ------------------------------------------------------------------------------

# text-to-speech helper
[private]
say *args:
    @say {{ args }} 2>/dev/null || echo {{ args }} || true
    @just notf {{ args }} || true

[private]
notf *args:
    @command -v notf >/dev/null && notf "{{ args }}" -t "Cove" -T bell,macos || true

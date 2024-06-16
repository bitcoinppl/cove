default:
    just --list

clean:
    cargo clean
    rm -rf ios/Cove.xcframework
    rm -rf ios/Cove
    rf -rf rust/target

build-android:
    bash scripts/build-android.sh

run-android: build-android
    bash scripts/run-android.sh

build-ios profile="debug":
    bash scripts/build-ios.sh {{profile}}

run-ios: build-ios
    bash scripts/run-ios.sh

#!/bin/bash
set -ex
cd android

# Variables
PACKAGE_NAME="com.example.cove"
ACTIVITY_NAME=".MainActivity"
APK_PATH="app/build/outputs/apk/debug/app-debug.apk"

# Build the debug version of the app
./gradlew assembleDebug

# Check if build was successful
if [ $? -eq 0 ]; then
    echo "Build successful."
else
    echo "Build failed."
    exit 1
fi

# Install the APK on the connected device or running emulator
adb install -r $APK_PATH

# Check if install was successful
if [ $? -eq 0 ]; then
    echo "App installed successfully."
else
    echo "App installation failed."
    exit 1
fi

# Launch the app
adb shell am start -n $PACKAGE_NAME/$ACTIVITY_NAME

# Check if launch was successful
if [ $? -eq 0 ]; then
    echo "App launched successfully."
else
    echo "App launch failed."
    exit 1
fi

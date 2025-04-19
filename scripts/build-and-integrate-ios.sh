#!/bin/bash
set -e
set -o pipefail

# Script to build the CoveCore XCFramework and integrate it with the Xcode project

echo "Building and integrating CoveCore.xcframework..."
echo ""
./scripts/build-ios.sh "$1" "$2" "$3" "--integrate"
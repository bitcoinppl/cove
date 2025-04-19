#!/bin/bash
set -e
set -o pipefail

# Script to set up automatic imports for CoveCore

echo "Setting up automatic imports for CoveCore..."
echo ""

# Ensure the CoveCore XCFramework is built and integrated
./scripts/build-and-integrate-ios.sh "$1"

# Choose which method to use based on argument
METHOD=${2:-"swift"} # Default to Swift method

case "$METHOD" in
  "precompiled")
    echo "Setting up using precompiled Swift header (Method 1)..."
    ./scripts/configure-precompiled-header.sh
    ;;
  "bridging")
    echo "Setting up using Objective-C bridging header (Method 2)..."
    ./scripts/configure-bridging-header.sh
    ;;
  "swift" | *)
    echo "Setting up using Swift exports (Method 3)..."
    ./scripts/add-global-imports.sh
    ;;
esac

echo ""
echo "✅ Setup complete! CoveCore will now be automatically imported in your project."
echo ""
echo "IMPORTANT: You may need to clean your build folder (Cmd+Shift+K) and restart Xcode for changes to take effect."
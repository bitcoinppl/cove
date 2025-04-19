#!/bin/bash
set -e
set -o pipefail

# Script to import the CoveCore XCFramework into the iOS Xcode project

# Move to project root
cd "$(dirname "$0")/.."

# Ensure the XCFramework exists
XCFRAMEWORK_PATH="rust/ios/CoveCore.xcframework"
if [ ! -d "$XCFRAMEWORK_PATH" ]; then
  echo "❌ Error: CoveCore.xcframework not found at $XCFRAMEWORK_PATH"
  echo "   Run ./scripts/build-ios.sh first to build the framework"
  exit 1
fi

# Check if we need to create the Frameworks directory
FRAMEWORKS_DIR="ios/Frameworks"
if [ ! -d "$FRAMEWORKS_DIR" ]; then
  echo "Creating Frameworks directory..."
  mkdir -p "$FRAMEWORKS_DIR"
fi

# Copy the XCFramework to the iOS Frameworks directory
echo "Copying CoveCore.xcframework to $FRAMEWORKS_DIR..."
rm -rf "$FRAMEWORKS_DIR/CoveCore.xcframework" || true
cp -R "$XCFRAMEWORK_PATH" "$FRAMEWORKS_DIR/"

echo "✅ Successfully imported CoveCore.xcframework to $FRAMEWORKS_DIR"
echo ""
echo "To link the framework in Xcode, follow these steps:"
echo ""
echo "1. Open your Xcode project at ios/Cove.xcodeproj"
echo "2. Select your project in the Project Navigator"
echo "3. Select your target and go to 'General' tab"
echo "4. Scroll down to 'Frameworks, Libraries, and Embedded Content'"
echo "5. Click the '+' button"
echo "6. Click 'Add Other...' and select 'Add Files...'"
echo "7. Navigate to $FRAMEWORKS_DIR/CoveCore.xcframework and click 'Open'"
echo "8. Ensure 'Embed & Sign' is selected for the framework"
echo "9. Build and run your project"
echo ""
echo "You can now import CoveCore in your Swift files with: import CoveCore"
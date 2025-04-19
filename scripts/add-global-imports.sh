#!/bin/bash
set -e
set -o pipefail

# Script to add global imports to the main app file

APP_FILE="/Users/praveen/code/bitcoinppl/cove/ios/Cove/CoveApp.swift"

if [ ! -f "$APP_FILE" ]; then
  echo "❌ Error: Main app file not found at $APP_FILE"
  exit 1
fi

# Check if the import is already added
if grep -q "import GlobalImports" "$APP_FILE"; then
  echo "Global imports already added to the main app file"
  exit 0
fi

# Add the import to the main app file, right after any existing imports
echo "Adding global imports to the main app file..."
sed -i '' '/^import / a\
import GlobalImports
' "$APP_FILE"

echo "✅ Successfully added global imports to the main app file"
echo "   CoveCore will now be automatically available in files that import this app file"
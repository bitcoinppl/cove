#!/bin/bash
set -e
set -o pipefail

# Script to add the CoveCore XCFramework to the Xcode project programmatically

# Check if Ruby is installed
if ! command -v ruby &> /dev/null; then
  echo "❌ Error: Ruby is required to run this script"
  exit 1
fi

# Check if the xcodeproj gem is installed
if ! gem list -i xcodeproj &> /dev/null; then
  echo "Installing xcodeproj gem..."
  gem install xcodeproj
fi

# Run the import framework script first to ensure the framework is copied
echo "Importing framework..."
"$(dirname "$0")/import-framework.sh"

# Run the Ruby script to add the framework to the Xcode project
echo "Adding framework to Xcode project..."
ruby "$(dirname "$0")/add-framework-to-xcode.rb"
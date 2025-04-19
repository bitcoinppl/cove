#!/bin/bash
set -e
set -o pipefail

# Script to configure the Objective-C bridging header for the Xcode project

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

# Run the Ruby script to configure the bridging header
echo "Configuring bridging header..."
ruby "$(dirname "$0")/configure-bridging-header.rb"
#!/usr/bin/env ruby
require 'xcodeproj'

# Path to the Xcode project
project_path = File.expand_path('../ios/Cove.xcodeproj', __dir__)

# Check if the project exists
unless File.exist?(project_path)
  puts "❌ Error: Xcode project not found at #{project_path}"
  exit 1
end

# Open the project
project = Xcodeproj::Project.open(project_path)

# Find the main target
target = project.targets.find { |t| t.product_type == "com.apple.product-type.application" }

unless target
  puts "❌ Error: Could not find an application target in the project"
  exit 1
end

puts "Found target: #{target.name}"

# Path to the bridging header file (relative to the project)
bridging_header_path = 'Cove/Cove-Bridging-Header.h'

# Update build settings to use the bridging header
target.build_configurations.each do |config|
  puts "Updating build settings for configuration: #{config.name}"
  
  # Set the Swift Objective-C bridging header
  current_header = config.build_settings['SWIFT_OBJC_BRIDGING_HEADER']
  if current_header != bridging_header_path
    config.build_settings['SWIFT_OBJC_BRIDGING_HEADER'] = bridging_header_path
    puts "  - Set bridging header path"
  else
    puts "  - Bridging header path already set"
  end
  
  # Enable Objective-C modules
  if config.build_settings['CLANG_ENABLE_MODULES'] != 'YES'
    config.build_settings['CLANG_ENABLE_MODULES'] = 'YES'
    puts "  - Enabled Clang modules"
  else
    puts "  - Clang modules already enabled"
  end
  
  # Precompile the bridging header
  if config.build_settings['SWIFT_PRECOMPILE_BRIDGING_HEADER'] != 'YES'
    config.build_settings['SWIFT_PRECOMPILE_BRIDGING_HEADER'] = 'YES'
    puts "  - Enabled precompiled bridging header"
  else
    puts "  - Precompiled bridging header already enabled"
  end
end

# Save the project
project.save
puts "✅ Successfully updated the Xcode project to use Cove-Bridging-Header.h"
puts "   This bridging header will now be automatically imported by all Swift files in the project"
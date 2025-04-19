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

# Path to the precompiled header file (relative to the project)
pch_path = 'Cove/CoveImports.swift'

# Update build settings to use the precompiled header
target.build_configurations.each do |config|
  puts "Updating build settings for configuration: #{config.name}"
  
  # Add the Swift compile flags
  current_flags = config.build_settings['OTHER_SWIFT_FLAGS'] || ''
  unless current_flags.include?('-import-objc-header')
    config.build_settings['OTHER_SWIFT_FLAGS'] = "#{current_flags} -Xfrontend -enable-implicit-module-import-synthesis"
    puts "  - Added Swift compiler flags"
  end
  
  # Set the precompiled module for Swift
  if !config.build_settings['SWIFT_PRECOMPILE_BRIDGING_HEADER'] || config.build_settings['SWIFT_PRECOMPILE_BRIDGING_HEADER'] != 'YES'
    config.build_settings['SWIFT_PRECOMPILE_BRIDGING_HEADER'] = 'YES'
    puts "  - Enabled precompiled bridging header"
  end
  
  # Set the implicit Swift module import name
  unless config.build_settings['SWIFT_IMPLICIT_MODULE_IMPORT_NAME']
    config.build_settings['SWIFT_IMPLICIT_MODULE_IMPORT_NAME'] = 'CoveImports'
    puts "  - Set implicit module import name"
  end
end

# Save the project
project.save
puts "✅ Successfully updated the Xcode project to use CoveImports.swift"
puts "   This file will now be automatically imported by all Swift files in the project"
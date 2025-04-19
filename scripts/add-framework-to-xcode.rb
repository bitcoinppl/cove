#!/usr/bin/env ruby
require 'xcodeproj'

# Path to the Xcode project
project_path = File.expand_path('../ios/Cove.xcodeproj', __dir__)
xcframework_path = File.expand_path('../ios/Frameworks/CoveCore.xcframework', __dir__)

# Check if the project exists
unless File.exist?(project_path)
  puts "❌ Error: Xcode project not found at #{project_path}"
  exit 1
end

# Check if the XCFramework exists
unless File.exist?(xcframework_path)
  puts "❌ Error: CoveCore.xcframework not found at #{xcframework_path}"
  puts "   Run ./scripts/import-framework.sh first to import the framework"
  exit 1
end

# Open the project
project = Xcodeproj::Project.open(project_path)

# Find the main target (usually the first application target)
target = project.targets.find { |t| t.product_type == "com.apple.product-type.application" }

unless target
  puts "❌ Error: Could not find an application target in the project"
  exit 1
end

puts "Found target: #{target.name}"

# Get the frameworks group, create it if it doesn't exist
frameworks_group = project.main_group.find_subpath('Frameworks', true)
puts "Using frameworks group: #{frameworks_group.display_name}"

# Check if the framework is already added
existing_file_ref = frameworks_group.find_file_by_path(xcframework_path)
if existing_file_ref
  puts "CoveCore.xcframework is already added to the project"
else
  # Add the XCFramework to the project
  file_ref = frameworks_group.new_file(xcframework_path)
  puts "Added CoveCore.xcframework to the project"

  # Add the framework to the target
  target.frameworks_build_phase.add_file_reference(file_ref)
  puts "Added CoveCore.xcframework to target's Frameworks Build Phase"
end

# Make sure the framework is embedded
embed_phase = target.build_phases.find { |bp| bp.display_name == 'Embed Frameworks' }
unless embed_phase
  embed_phase = project.new(Xcodeproj::Project::Object::PBXCopyFilesBuildPhase)
  embed_phase.name = 'Embed Frameworks'
  embed_phase.dstPath = '$(FRAMEWORKS_FOLDER_PATH)'
  embed_phase.dstSubfolderSpec = 10 # Frameworks
  target.build_phases << embed_phase
  puts "Created Embed Frameworks build phase"
end

# Find the file reference again (it might have changed)
file_ref = frameworks_group.find_file_by_path(xcframework_path)
if file_ref
  # Check if the framework is already in the embed phase
  existing_build_file = embed_phase.files.find { |bf| bf.file_ref == file_ref }
  unless existing_build_file
    build_file = embed_phase.add_file_reference(file_ref)
    build_file.settings = { 'ATTRIBUTES' => ['CodeSignOnCopy', 'RemoveHeadersOnCopy'] }
    puts "Added CoveCore.xcframework to Embed Frameworks phase"
  else
    puts "CoveCore.xcframework is already in the Embed Frameworks phase"
  end
else
  puts "❌ Error: Could not find CoveCore.xcframework reference in project"
  exit 1
end

# Save the project
project.save
puts "✅ Successfully updated the Xcode project"
puts "   The CoveCore.xcframework is now linked and will be embedded in your app"
puts "   You can now import CoveCore in your Swift files with: import CoveCore"
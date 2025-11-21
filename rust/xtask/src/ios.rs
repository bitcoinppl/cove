use crate::common::{command_exists, print_error, print_info, print_success};
use color_eyre::{eyre::Context, Result};
use colored::Colorize;
use xshell::{cmd, Shell};

#[derive(Debug, Clone, Copy)]
pub enum IosBuildType {
    Debug,
    Release,
    Custom(&'static str),
}

impl IosBuildType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "debug" | "--debug" => Self::Debug,
            "release" | "--release" => Self::Release,
            "release-smaller" | "--release-smaller" => Self::Custom("release-smaller"),
            profile => Self::Custom(Box::leak(profile.to_string().into_boxed_str())),
        }
    }

    pub fn cargo_flag(&self) -> String {
        match self {
            Self::Debug => String::new(),
            Self::Release => "--release".to_string(),
            Self::Custom(profile) => format!("--profile {}", profile),
        }
    }

    pub fn target_dir_name(&self) -> &str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
            Self::Custom(profile) => profile,
        }
    }
}

pub fn build_ios(build_type: IosBuildType, device: bool, _sign: bool, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for xcodebuild
    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    // determine targets based on build type and device flag
    let targets = match build_type {
        IosBuildType::Release | IosBuildType::Custom(_) => {
            // release builds only for actual device
            vec!["aarch64-apple-ios"]
        }
        IosBuildType::Debug => {
            if device {
                // debug on device and simulator
                vec!["aarch64-apple-ios", "aarch64-apple-ios-sim"]
            } else {
                // debug on simulator only
                vec!["aarch64-apple-ios-sim"]
            }
        }
    };

    let build_flag = build_type.cargo_flag();
    let build_dir = build_type.target_dir_name();

    println!("{}", format!("Building for targets: {:?}", targets).blue().bold());

    // build static libraries for each target
    let mut library_flags = Vec::new();
    for target in &targets {
        println!(
            "{}",
            format!("Building for target: {} with build type: {}", target, build_dir).blue().bold()
        );

        // add target
        cmd!(sh, "rustup target add {target}")
            .run()
            .wrap_err_with(|| format!("Failed to add target {}", target))?;

        // build with cargo
        let build_cmd = if build_flag.is_empty() {
            cmd!(sh, "cargo build --target {target}")
        } else {
            let flags: Vec<&str> = build_flag.split_whitespace().collect();
            match flags.as_slice() {
                ["--release"] => cmd!(sh, "cargo build --target {target} --release"),
                ["--profile", profile_name] => {
                    cmd!(sh, "cargo build --target {target} --profile {profile_name}")
                }
                _ => cmd!(sh, "cargo build --target {target}"),
            }
        };

        if verbose {
            build_cmd.run().wrap_err_with(|| format!("Failed to build for target {}", target))?;
        } else {
            build_cmd
                .quiet()
                .run()
                .wrap_err_with(|| format!("Failed to build for target {}", target))?;
        }

        let lib_path = format!("./target/{}/{}/libcove.a", target, build_dir);
        if !sh.path_exists(&lib_path) {
            print_error(&format!("Missing static library at {}", lib_path));
            color_eyre::eyre::bail!("Build failed: missing library at {}", lib_path);
        }

        library_flags.push(format!("-library {} -headers ./bindings", lib_path));
        print_success(&format!("Built library for {}", target));
    }

    // generate headers, modulemap, and swift sources using UniFFI
    println!("{}", "Generating Swift bindings...".blue().bold());
    let output_dir = "./bindings";
    let static_lib_path = format!("./target/{}/{}/libcove.a", targets[0], build_dir);

    sh.create_dir(output_dir).wrap_err("Failed to create bindings directory")?;

    print_info(&format!("Running uniffi-bindgen for {}, outputting to {}", targets[0], output_dir));

    let _ = sh.remove_path(output_dir);
    cmd!(
        sh,
        "cargo run -p uniffi_cli -- {static_lib_path} {output_dir} --swift-sources --headers --modulemap --module-name cove_core_ffi --modulemap-filename module.modulemap"
    )
    .run()
    .wrap_err("Failed to generate Swift bindings")?;

    // create XCFramework
    println!("{}", "Creating XCFramework...".blue().bold());
    let spm_package = "../ios/CoveCore/";
    let xcframework_output = format!("{}Sources/cove_core_ffi.xcframework", spm_package);
    let generated_swift_sources = format!("{}Sources/CoveCore/generated", spm_package);

    let _ = sh.remove_path(&xcframework_output);

    // build xcodebuild command with library flags
    let library_flags_str = library_flags.join(" ");
    let xcodebuild_cmd = format!(
        "xcodebuild -create-xcframework {} -output {}",
        library_flags_str, xcframework_output
    );

    // run xcodebuild command
    sh.cmd("sh").arg("-c").arg(&xcodebuild_cmd).run().wrap_err("Failed to create XCFramework")?;

    print_success("Created XCFramework");

    // copy Swift sources to SPM package
    print_info("Copying Swift sources to SPM package...");
    let _ = sh.remove_path(&generated_swift_sources);
    sh.create_dir(&generated_swift_sources)
        .wrap_err("Failed to create generated sources directory")?;

    // use sh -c to expand the glob properly
    let copy_cmd = format!("cp -r {}/*.swift {}", output_dir, generated_swift_sources);
    sh.cmd("sh")
        .arg("-c")
        .arg(&copy_cmd)
        .run()
        .wrap_err("Failed to copy Swift sources")?;

    // remove uniffi generated Package.swift file if it exists
    let package_swift = format!("{}Sources/CoveCore/Package.swift", spm_package);
    let _ = sh.remove_path(&package_swift);

    print_success("iOS build completed successfully!");
    Ok(())
}

pub fn run_ios(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for xcodebuild
    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    // check for xcrun
    if !command_exists("xcrun") {
        print_error("xcrun not found. Please install Xcode command line tools");
        color_eyre::eyre::bail!("xcrun command not found");
    }

    let scheme = "Cove";
    let app_name = "Cove";
    let bundle_id = "org.bitcoinppl.Cove";
    let destination = "platform=iOS Simulator,name=iPhone 15 Pro,OS=latest";

    // change to ios directory
    sh.change_dir("../ios");

    // build the app
    print_info("Building iOS app...");
    if verbose {
        cmd!(sh, "xcodebuild -scheme {scheme} -destination {destination} build")
            .run()
            .wrap_err("Failed to build iOS app")?;
    } else {
        cmd!(sh, "xcodebuild -scheme {scheme} -destination {destination} build")
            .quiet()
            .run()
            .wrap_err("Failed to build iOS app")?;
    }
    print_success("Build successful");

    // find the built app
    print_info("Finding built app...");
    let home_dir = std::env::var("HOME").wrap_err("Failed to get HOME environment variable")?;
    let derived_data = format!("{}/Library/Developer/Xcode/DerivedData", home_dir);

    let find_output = cmd!(sh, "find {derived_data} -name {app_name}.app")
        .read()
        .wrap_err("Failed to find built app")?;

    let app_path = find_output
        .lines()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("App not found in DerivedData"))?;

    if app_path.is_empty() {
        print_error("App not found!");
        color_eyre::eyre::bail!("Could not locate built app");
    }

    print_success(&format!("Found app at: {}", app_path));

    // install the app on the simulator
    print_info("Installing app on simulator...");
    cmd!(sh, "xcrun simctl install booted {app_path}")
        .run()
        .wrap_err("Failed to install app on simulator")?;
    print_success("App installed successfully");

    // launch the app
    print_info("Launching app...");
    cmd!(sh, "xcrun simctl launch booted {bundle_id}").run().wrap_err("Failed to launch app")?;
    print_success("App launched successfully");

    Ok(())
}

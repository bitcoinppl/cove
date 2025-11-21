use crate::common::{command_exists, print_error, print_info, print_success, print_warning};
use color_eyre::{eyre::Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use xshell::{cmd, Shell};

#[derive(Debug, Clone, Copy)]
pub enum BuildProfile {
    Debug,
    Release,
    Custom(&'static str),
}

impl BuildProfile {
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

const ANDROID_TARGETS: &[&str] = &["aarch64-linux-android", "x86_64-linux-android"];

fn get_abi_mapping() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("aarch64-linux-android", "arm64-v8a");
    map.insert("x86_64-linux-android", "x86_64");
    map
}

pub fn build_android(profile: BuildProfile, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for cargo-ndk
    if !command_exists("cargo-ndk") {
        print_warning("cargo-ndk not found, installing...");
        cmd!(sh, "cargo install cargo-ndk").run().wrap_err("Failed to install cargo-ndk")?;
        print_success("Installed cargo-ndk");
    }

    let jni_libs_dir = "../android/app/src/main/jniLibs";
    let android_kotlin_dir = "../android/app/src/main/java";
    let bindings_dir = "./bindings/kotlin";

    // prepare directories
    sh.create_dir(jni_libs_dir).wrap_err("Failed to create jniLibs directory")?;
    sh.create_dir(android_kotlin_dir).wrap_err("Failed to create kotlin directory")?;
    let _ = sh.remove_path(bindings_dir);
    sh.create_dir(bindings_dir).wrap_err("Failed to create bindings directory")?;

    let abi_mapping = get_abi_mapping();
    let build_flag = profile.cargo_flag();
    let build_type = profile.target_dir_name();

    // set Android min SDK version
    sh.set_var("CFLAGS", "-D__ANDROID_MIN_SDK_VERSION__=21");

    // build for each target
    for target in ANDROID_TARGETS {
        println!(
            "{}",
            format!("Building for target: {} with build type: {}", target, build_type)
                .blue()
                .bold()
        );

        // add target
        cmd!(sh, "rustup target add {target}")
            .run()
            .wrap_err_with(|| format!("Failed to add target {}", target))?;

        // build with cargo-ndk
        let build_cmd = if build_flag.is_empty() {
            cmd!(sh, "cargo ndk --target {target} build")
        } else {
            let flags: Vec<&str> = build_flag.split_whitespace().collect();
            match flags.as_slice() {
                ["--release"] => cmd!(sh, "cargo ndk --target {target} build --release"),
                ["--profile", profile_name] => {
                    cmd!(sh, "cargo ndk --target {target} build --profile {profile_name}")
                }
                _ => cmd!(sh, "cargo ndk --target {target} build"),
            }
        };

        if verbose {
            build_cmd.run().wrap_err_with(|| {
                format!("Failed to build for target {} with profile {:?}", target, profile)
            })?;
        } else {
            build_cmd.quiet().run().wrap_err_with(|| {
                format!("Failed to build for target {} with profile {:?}", target, profile)
            })?;
        }

        // verify the library was built
        let dynamic_lib_path = format!("./target/{}/{}/libcove.so", target, build_type);
        if !sh.path_exists(&dynamic_lib_path) {
            print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
            color_eyre::eyre::bail!("Build failed: missing library at {}", dynamic_lib_path);
        }

        // copy to jniLibs
        let abi = abi_mapping.get(target).ok_or_else(|| {
            color_eyre::eyre::eyre!("Unable to map target {} to an Android ABI directory", target)
        })?;

        let abi_dir = format!("{}/{}", jni_libs_dir, abi);
        sh.create_dir(&abi_dir)
            .wrap_err_with(|| format!("Failed to create ABI directory {}", abi_dir))?;

        let dest_path = format!("{}/libcoveffi.so", abi_dir);
        sh.copy_file(&dynamic_lib_path, &dest_path).wrap_err_with(|| {
            format!("Failed to copy library from {} to {}", dynamic_lib_path, dest_path)
        })?;

        print_success(&format!("Built and copied library for {}", target));
    }

    // generate UniFFI bindings
    println!("{}", "Generating Kotlin bindings...".blue().bold());
    let first_target = ANDROID_TARGETS[0];
    let dynamic_lib_path = format!("./target/{}/{}/libcove.so", first_target, build_type);

    if !sh.path_exists(&dynamic_lib_path) {
        print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
        color_eyre::eyre::bail!(
            "Cannot generate bindings: missing library at {}",
            dynamic_lib_path
        );
    }

    print_info(&format!("Generating Kotlin bindings into {}", bindings_dir));
    cmd!(
        sh,
        "cargo run -p uniffi_cli -- generate {dynamic_lib_path} --library --language kotlin --no-format --out-dir {bindings_dir}"
    )
    .run()
    .wrap_err("Failed to generate Kotlin bindings")?;

    print_info(&format!("Copying Kotlin bindings into Android project at {}", android_kotlin_dir));

    // remove only generated binding files, not user code
    let cove_core_dir = format!("{}/org/bitcoinppl/cove_core", android_kotlin_dir);
    let _ = sh.remove_path(&cove_core_dir);

    // copy bindings
    cmd!(sh, "cp -R {bindings_dir}/. {android_kotlin_dir}/")
        .run()
        .wrap_err("Failed to copy Kotlin bindings to Android project")?;

    print_success("Android build completed successfully!");
    Ok(())
}

pub fn run_android(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for adb
    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        color_eyre::eyre::bail!("adb command not found");
    }

    let package_name = "org.bitcoinppl.cove";
    let activity_name = ".MainActivity";
    let apk_path = "app/build/outputs/apk/debug/app-debug.apk";

    // change to android directory
    sh.change_dir("../android");

    // build the debug version
    print_info("Building debug APK...");
    if verbose {
        cmd!(sh, "./gradlew assembleDebug").run().wrap_err("Failed to build APK")?;
    } else {
        cmd!(sh, "./gradlew assembleDebug").quiet().run().wrap_err("Failed to build APK")?;
    }
    print_success("Build successful");

    // install the APK
    print_info("Installing APK on device/emulator...");
    cmd!(sh, "adb install -r {apk_path}").run().wrap_err("Failed to install APK")?;
    print_success("App installed successfully");

    // launch the app
    print_info("Launching app...");
    let full_activity = format!("{}{}", package_name, activity_name);
    cmd!(sh, "adb shell am start -n {full_activity}").run().wrap_err("Failed to launch app")?;
    print_success("App launched successfully");

    Ok(())
}

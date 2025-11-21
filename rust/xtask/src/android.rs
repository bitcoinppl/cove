use crate::common::{command_exists, print_error, print_info, print_success};
use color_eyre::{eyre::Context, Result};
use colored::Colorize;
use std::collections::HashMap;
use xshell::{cmd, Shell};

// Android build constants
const ANDROID_TARGETS: &[&str] = &["aarch64-linux-android", "x86_64-linux-android"];
const JNI_LIBS_DIR: &str = "../android/app/src/main/jniLibs";
const ANDROID_KOTLIN_DIR: &str = "../android/app/src/main/java";
const BINDINGS_DIR: &str = "./bindings/kotlin";
const CFLAGS_VALUE: &str = "-D__ANDROID_MIN_SDK_VERSION__=21";
const LIB_NAME: &str = "libcove.so";
const OUTPUT_LIB_NAME: &str = "libcoveffi.so";
const COVE_CORE_PACKAGE_PATH: &str = "org/bitcoinppl/cove_core";

// Android run constants
const ANDROID_PACKAGE_NAME: &str = "org.bitcoinppl.cove";
const ANDROID_ACTIVITY_NAME: &str = ".MainActivity";
const APK_PATH: &str = "app/build/outputs/apk/debug/app-debug.apk";

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
        print_error("cargo-ndk not found. Please run: cargo xtask install-deps");
        color_eyre::eyre::bail!("cargo-ndk is required for Android builds");
    }

    // prepare directories
    sh.create_dir(JNI_LIBS_DIR).wrap_err("Failed to create jniLibs directory")?;
    sh.create_dir(ANDROID_KOTLIN_DIR).wrap_err("Failed to create kotlin directory")?;
    let _ = sh.remove_path(BINDINGS_DIR);
    sh.create_dir(BINDINGS_DIR).wrap_err("Failed to create bindings directory")?;

    let abi_mapping = get_abi_mapping();
    let build_flag = profile.cargo_flag();
    let build_type = profile.target_dir_name();

    // set Android min SDK version
    sh.set_var("CFLAGS", CFLAGS_VALUE);

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
        let flags = crate::common::parse_build_flags(&build_flag);
        let build_result = if flags.is_empty() {
            let cmd = cmd!(sh, "cargo ndk --target {target} build");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 1 && flags[0] == "--release" {
            let cmd = cmd!(sh, "cargo ndk --target {target} build --release");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 2 && flags[0] == "--profile" {
            let profile_name = &flags[1];
            let cmd = cmd!(sh, "cargo ndk --target {target} build --profile {profile_name}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else {
            let cmd = cmd!(sh, "cargo ndk --target {target} build");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        };

        build_result.wrap_err_with(|| {
            format!("Failed to build for target {} with profile {:?}", target, profile)
        })?;

        // verify the library was built
        let dynamic_lib_path = format!("./target/{}/{}/{}", target, build_type, LIB_NAME);
        if !sh.path_exists(&dynamic_lib_path) {
            print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
            color_eyre::eyre::bail!("Build failed: missing library at {}", dynamic_lib_path);
        }

        // copy to jniLibs
        let abi = abi_mapping.get(target).ok_or_else(|| {
            color_eyre::eyre::eyre!("Unable to map target {} to an Android ABI directory", target)
        })?;

        let abi_dir = format!("{}/{}", JNI_LIBS_DIR, abi);
        sh.create_dir(&abi_dir)
            .wrap_err_with(|| format!("Failed to create ABI directory {}", abi_dir))?;

        let dest_path = format!("{}/{}", abi_dir, OUTPUT_LIB_NAME);
        sh.copy_file(&dynamic_lib_path, &dest_path).wrap_err_with(|| {
            format!("Failed to copy library from {} to {}", dynamic_lib_path, dest_path)
        })?;

        print_success(&format!("Built and copied library for {}", target));
    }

    // generate UniFFI bindings
    println!("{}", "Generating Kotlin bindings...".blue().bold());
    let first_target = ANDROID_TARGETS[0];
    let dynamic_lib_path = format!("./target/{}/{}/{}", first_target, build_type, LIB_NAME);

    if !sh.path_exists(&dynamic_lib_path) {
        print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
        color_eyre::eyre::bail!(
            "Cannot generate bindings: missing library at {}",
            dynamic_lib_path
        );
    }

    print_info(&format!("Generating Kotlin bindings into {}", BINDINGS_DIR));
    cmd!(
        sh,
        "cargo run -p uniffi_cli -- generate {dynamic_lib_path} --library --language kotlin --no-format --out-dir {BINDINGS_DIR}"
    )
    .run()
    .wrap_err("Failed to generate Kotlin bindings")?;

    print_info(&format!("Copying Kotlin bindings into Android project at {}", ANDROID_KOTLIN_DIR));

    // remove only generated binding files, not user code
    let cove_core_dir = format!("{}/{}", ANDROID_KOTLIN_DIR, COVE_CORE_PACKAGE_PATH);
    let _ = sh.remove_path(&cove_core_dir);

    // copy bindings
    cmd!(sh, "cp -R {BINDINGS_DIR}/. {ANDROID_KOTLIN_DIR}/")
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
    cmd!(sh, "adb install -r {APK_PATH}").run().wrap_err("Failed to install APK")?;
    print_success("App installed successfully");

    // launch the app
    print_info("Launching app...");
    let full_activity = format!("{}{}", ANDROID_PACKAGE_NAME, ANDROID_ACTIVITY_NAME);
    cmd!(sh, "adb shell am start -n {full_activity}").run().wrap_err("Failed to launch app")?;
    print_success("App launched successfully");

    Ok(())
}

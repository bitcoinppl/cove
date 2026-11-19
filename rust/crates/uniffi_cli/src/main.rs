mod error_text;

use std::env;
use std::path::PathBuf;
use std::process::exit;

/// Swift CLI flags that take a value, so the value is not mistaken for a positional argument
const SWIFT_VALUE_FLAGS: &[&str] =
    &["--module-name", "--modulemap-filename", "--link-frameworks", "--config", "-c"];

fn main() {
    // check if any argument contains "kotlin"
    let args: Vec<String> = env::args().collect();
    let is_kotlin = args.iter().any(|arg| arg.to_lowercase().contains("kotlin"));

    if is_kotlin {
        uniffi::uniffi_bindgen_main();
    } else {
        uniffi::uniffi_bindgen_swift();
    }

    // uniffi exits the process on failure, so reaching here means its bindings were written
    let result = if is_kotlin {
        kotlin_out_dir(&args).map(|dir| error_text::write_kotlin(&dir)).transpose()
    } else {
        swift_out_dir(&args).map(|dir| error_text::write_swift(&dir)).transpose()
    };

    if let Err(error) = result {
        eprintln!("{error}");
        exit(1);
    }
}

/// The `--out-dir` of a Kotlin `generate` call
fn kotlin_out_dir(args: &[String]) -> Option<PathBuf> {
    let position = args.iter().position(|arg| arg == "--out-dir" || arg == "-o")?;
    args.get(position + 1).map(PathBuf::from)
}

/// The output directory of a Swift call that writes Swift sources
///
/// The Swift CLI takes the library path and then the output directory as positionals
fn swift_out_dir(args: &[String]) -> Option<PathBuf> {
    if !args.iter().any(|arg| arg == "--swift-sources") {
        return None;
    }

    let mut positionals = Vec::new();
    let mut skip_value = false;
    for arg in args.iter().skip(1) {
        if skip_value {
            skip_value = false;
            continue;
        }

        if arg.starts_with('-') {
            skip_value = SWIFT_VALUE_FLAGS.contains(&arg.as_str());
            continue;
        }

        positionals.push(arg);
    }

    positionals.get(1).map(PathBuf::from)
}

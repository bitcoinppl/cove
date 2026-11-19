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

/// The `--out-dir` of a Kotlin `generate` call, in any form clap accepts
fn kotlin_out_dir(args: &[String]) -> Option<PathBuf> {
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out-dir" {
            return args.next().map(PathBuf::from);
        }

        // clap keeps everything after the first `=` of a long flag, so `--out-dir==out` is `=out`
        if let Some(dir) = arg.strip_prefix("--out-dir=") {
            return Some(PathBuf::from(dir));
        }

        let Some(cluster) = arg.strip_prefix('-').filter(|rest| !rest.starts_with('-')) else {
            continue;
        };

        match short_out_dir(cluster) {
            ShortOutDir::Inline(dir) => return Some(PathBuf::from(dir)),
            ShortOutDir::NextArg => return args.next().map(PathBuf::from),
            ShortOutDir::Absent => {}
        }
    }

    None
}

enum ShortOutDir<'a> {
    Inline(&'a str),
    NextArg,
    Absent,
}

/// Reads `-o` from a cluster of short flags such as `-noout`
///
/// Of `generate`'s short flags only `-n` takes no value, so a cluster ends at the first other
/// flag, and an `o` after `-l` or `-c` is part of their value
fn short_out_dir(cluster: &str) -> ShortOutDir<'_> {
    for (index, flag) in cluster.char_indices() {
        match flag {
            'n' => continue,
            'o' => {
                let rest = &cluster[index + 1..];
                if rest.is_empty() {
                    return ShortOutDir::NextArg;
                }

                return ShortOutDir::Inline(rest.strip_prefix('=').unwrap_or(rest));
            }
            _ => return ShortOutDir::Absent,
        }
    }

    ShortOutDir::Absent
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(String::from).collect()
    }

    #[test]
    fn kotlin_out_dir_reads_every_clap_form() {
        for line in [
            "uniffi_cli generate lib.so --language kotlin --out-dir out",
            "uniffi_cli generate lib.so --language kotlin --out-dir=out",
            "uniffi_cli generate lib.so --language kotlin -o out",
            "uniffi_cli generate lib.so --language kotlin -o=out",
            "uniffi_cli generate lib.so --language kotlin -oout",
            "uniffi_cli generate lib.so -lkotlin -noout",
            "uniffi_cli generate lib.so -lkotlin -no out",
        ] {
            assert_eq!(kotlin_out_dir(&args(line)), Some(PathBuf::from("out")), "{line}");
        }

        let line = "uniffi_cli generate lib.so --language kotlin --out-dir==out";
        assert_eq!(kotlin_out_dir(&args(line)), Some(PathBuf::from("=out")));

        // the `o` in `-lkotlin` belongs to the language value
        assert_eq!(kotlin_out_dir(&args("uniffi_cli generate lib.so -lkotlin")), None);
    }

    #[test]
    fn swift_out_dir_skips_flag_values() {
        let line = "uniffi_cli lib.a --module-name cove_core_ffi out --swift-sources";
        assert_eq!(swift_out_dir(&args(line)), Some(PathBuf::from("out")));
        assert_eq!(swift_out_dir(&args("uniffi_cli lib.a out --headers")), None);
    }
}

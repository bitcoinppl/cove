use std::process::Command;

fn main() {
    let output = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().unwrap();
    let git_hash = String::from_utf8(output.stdout).unwrap();
    println!("cargo:rustc-env=GIT_SHORT_HASH={}", git_hash.trim());

    // Determine build profile from OUT_DIR
    // outdir: /Users/praveen/code/bitcoinppl/cove/rust/target/aarch64-apple-ios/release-smaller/...
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    let profile =
        out_dir.split("/target/").nth(1).unwrap_or_default().split('/').nth(1).unwrap_or("unknown");

    println!("cargo:rustc-env=BUILD_PROFILE={profile}");

    // Rebuild when Git changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=crates/*");
}

//! Release preflight that proves codesign can use the keychain's Apple signing keys

use crate::common::{print_info, print_success, print_warning};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use xshell::{cmd, Shell};

const APPLE_SIGNING_PREFIXES: [&str; 2] = ["Apple Development:", "Apple Distribution:"];
const LOGIN_KEYCHAIN: &str = "~/Library/Keychains/login.keychain-db";

/// A valid code signing identity from `security find-identity`
#[derive(Debug, PartialEq, Eq)]
struct SigningIdentity {
    hash: String,
    name: String,
}

/// Sign a scratch binary with every Apple Development and Apple Distribution identity
///
/// xcodebuild signs the archive only after the release Rust and Swift builds, so a locked
/// keychain or a key that needs an access prompt otherwise fails with errSecInternalComponent
/// many minutes in. A locked keychain in a GUI session shows its password prompt here instead,
/// while the person who started the release is still watching
pub fn ensure_keychain_can_sign(sh: &Shell) -> Result<()> {
    let output = cmd!(sh, "security find-identity -v -p codesigning")
        .quiet()
        .read()
        .wrap_err("Failed to list code signing identities")?;

    let identities = apple_signing_identities(&output);
    if identities.is_empty() {
        // automatic signing with -allowProvisioningUpdates can still create one during archive
        print_warning("No Apple Development or Distribution signing identity found; skipping keychain signing check");
        return Ok(());
    }

    print_info("Checking that codesign can use the keychain's signing keys...");
    let scratch = std::env::temp_dir().join(format!("cove-codesign-check-{}", std::process::id()));
    sh.copy_file("/usr/bin/true", &scratch)
        .wrap_err("Failed to create the codesign scratch binary")?;

    let result = identities.iter().try_for_each(|identity| sign_scratch(sh, identity, &scratch));
    let _ = sh.remove_path(&scratch);
    result?;

    print_success("codesign can use the keychain's signing keys");
    Ok(())
}

fn sign_scratch(sh: &Shell, identity: &SigningIdentity, scratch: &std::path::Path) -> Result<()> {
    let SigningIdentity { hash, name } = identity;
    let output = cmd!(sh, "codesign --force --sign {hash} {scratch}")
        .quiet()
        .ignore_status()
        .output()
        .wrap_err("Failed to run codesign")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(eyre!(
        "codesign could not sign with \"{name}\": {}\n\
         Nothing was bumped or built. Unlock the login keychain and retry:\n  \
         security unlock-keychain {LOGIN_KEYCHAIN}\n\
         If it still fails, codesign needs permission to use the key without a prompt:\n  \
         security set-key-partition-list -S apple-tool:,apple:,codesign: -s {LOGIN_KEYCHAIN}",
        stderr.trim()
    ))
}

/// Parse `security find-identity -v` output, keeping Apple Development and Distribution identities
///
/// The same identity is listed once per keychain search list match, so duplicates are dropped
fn apple_signing_identities(output: &str) -> Vec<SigningIdentity> {
    let mut identities: Vec<SigningIdentity> = Vec::new();

    for line in output.lines() {
        let Some((_, entry)) = line.trim().split_once(") ") else { continue };
        let Some((hash, name)) = entry.split_once(' ') else { continue };
        let name = name.trim().trim_matches('"');

        let is_apple_signing = APPLE_SIGNING_PREFIXES.iter().any(|prefix| name.starts_with(prefix));
        if !is_apple_signing || identities.iter().any(|identity| identity.hash == hash) {
            continue;
        }

        identities.push(SigningIdentity { hash: hash.to_string(), name: name.to_string() });
    }

    identities
}

#[cfg(test)]
mod tests {
    use super::{apple_signing_identities, SigningIdentity};

    #[test]
    fn keeps_unique_apple_development_and_distribution_identities() {
        let output = r#"  1) 0D43AA0F6277F4F1662140743349F510329BA6A7 "Developer ID Application: Example Inc. (WP9G5PVTDD)"
  2) B1B5AB1EF93AEB72FDD8E8289631A3AC6E4A7D57 "Apple Development: Jane Doe (DLB353KZ35)"
  3) 20311B31FC424F44392095DFE2FB87DAF4F10A3D "Apple Distribution: Example Inc. (Q8UP8C53Y8)"
  4) B1B5AB1EF93AEB72FDD8E8289631A3AC6E4A7D57 "Apple Development: Jane Doe (DLB353KZ35)"
     4 valid identities found"#;

        assert_eq!(
            apple_signing_identities(output),
            vec![
                SigningIdentity {
                    hash: "B1B5AB1EF93AEB72FDD8E8289631A3AC6E4A7D57".into(),
                    name: "Apple Development: Jane Doe (DLB353KZ35)".into(),
                },
                SigningIdentity {
                    hash: "20311B31FC424F44392095DFE2FB87DAF4F10A3D".into(),
                    name: "Apple Distribution: Example Inc. (Q8UP8C53Y8)".into(),
                },
            ]
        );
    }
}

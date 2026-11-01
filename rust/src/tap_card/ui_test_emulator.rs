//! Debug-only trust in Coinkite's card emulator, for the TAPSIGNER UI tests
//!
//! The UI tests sign through `ecard.py` over the app's debug TAPSIGNER bridge,
//! and the emulator's certificates chain to its own root key rather than
//! Coinkite's factory root. The emulator's root private key is public, so this
//! is compiled only with debug assertions, which no release profile enables,
//! and applies only while the app is configured to use the bridge

/// The root key `ecard.py` certifies its cards with
const EMULATOR_ROOT_KEY: &str =
    "0312d005ca1501b1603c3b00412eefe27c6b20a74c29377263b357b3aff12de6fa";

/// Set by the UI tests to point the app's TAPSIGNER transport at the bridge
const BRIDGE_ENV: &str = "COVE_UI_TEST_TAPSIGNER_BRIDGE";

/// Whether a certificate chain ending at `root`, a hex public key, is the emulator's and
/// the app is talking to it through the UI test bridge
pub fn trusts_root(root: &str) -> bool {
    root == EMULATOR_ROOT_KEY && std::env::var_os(BRIDGE_ENV).is_some()
}

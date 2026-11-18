//! Debug-only failure of opening Receive, for the UI test of the Receive sheet's failure alert
//!
//! Opening Receive fails only when reading or saving the wallet's address data fails, which a UI
//! test can't make happen. While the test sets the variable, every open fails with the error the
//! real failure reports, so the sheet closes and the app shows its alert. It makes Receive
//! unusable, so it is compiled only with debug assertions, which no release profile enables, and
//! applies only while the variable is set

/// Set by the UI test to make opening Receive fail
const OPEN_FAILS_ENV: &str = "COVE_UI_TEST_RECEIVE_OPEN_FAILS";

/// Whether opening Receive must fail
pub fn open_fails() -> bool {
    std::env::var_os(OPEN_FAILS_ENV).is_some()
}

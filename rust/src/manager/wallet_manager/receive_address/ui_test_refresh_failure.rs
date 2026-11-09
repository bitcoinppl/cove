//! Debug-only failure of the expired receive address refresh, for the Receive sheet UI test
//!
//! Receive replaces a shown address once it is five minutes old, and shows its refresh failure
//! only when saving the replacement fails, which a UI test can't make happen. While the test
//! sets the variable, a shown address expires after a few seconds and the save that the expired
//! address refresh makes fails, so the real error path shows the failure. Nothing else changes:
//! every other save, and which address is shown, behave as without it. It changes how long an
//! address stays and makes a save fail, so it is compiled only with debug assertions, which no
//! release profile enables, and applies only while the variable is set

use std::{cell::Cell, time::Duration};

/// Set by the Receive sheet UI test to make the expired address refresh fail
const REFRESH_FAILS_ENV: &str = "COVE_UI_TEST_RECEIVE_REFRESH_FAILS";

/// How long a shown address stays while the variable is set
const CACHE_WINDOW: Duration = Duration::from_secs(5);

thread_local! {
    // the refresh saves from a synchronous call on the actor's thread, so a flag on that thread
    // marks the saves it makes without passing the seam through the wallet and database code
    static IN_REFRESH: Cell<bool> = const { Cell::new(false) };
}

/// How long a shown address stays, or `None` for the real window while the variable is unset
pub fn cache_window() -> Option<Duration> {
    is_active().then_some(CACHE_WINDOW)
}

/// Whether a receive address save must fail: only one the expired address refresh makes, and
/// only while the variable is set
pub fn save_fails() -> bool {
    is_active() && IN_REFRESH.with(Cell::get)
}

/// Marks the saves made on this thread as the expired address refresh's until it drops
pub struct RefreshScope(());

impl RefreshScope {
    pub fn enter() -> Self {
        IN_REFRESH.with(|flag| flag.set(true));
        Self(())
    }
}

impl Drop for RefreshScope {
    fn drop(&mut self) {
        IN_REFRESH.with(|flag| flag.set(false));
    }
}

fn is_active() -> bool {
    std::env::var_os(REFRESH_FAILS_ENV).is_some()
}

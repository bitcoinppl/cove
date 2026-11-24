//! Debug-only OHTTP relay for the UI tests that pay a pj URI
//!
//! The UI test host runs a BIP 77 directory and an OHTTP relay on loopback ports, with a receiver
//! whose pj URI names that directory. The built-in relays are public `https` services that can't
//! reach it, so while the test sets the variable to the host's relay, a wallet's Payjoin sends go
//! through that relay instead. Only a plain `http` relay on a loopback host is taken, and the seam
//! is compiled only with debug assertions, which no release profile enables

use tracing::{error, info};
use url::Url;

use super::config::OhttpRelays;

/// Set by the UI test to the host relay's URL, such as `http://127.0.0.1:39012`
const RELAY_ENV: &str = "COVE_UI_TEST_PAYJOIN_RELAY";

/// The host's relay when the variable names a valid loopback relay
pub(crate) fn relays() -> Option<OhttpRelays> {
    let relay = std::env::var(RELAY_ENV).ok()?;

    // a typo here would otherwise send the test's payment through the public relays
    let parsed = Url::parse(&relay)
        .map_err(|error| error.to_string())
        .and_then(|url| OhttpRelays::loopback_for_test([url]).map_err(|error| error.to_string()));

    match parsed {
        Ok(relays) => {
            info!("Payjoin sends use the UI test relay {relay}");
            Some(relays)
        }

        Err(error) => {
            error!("Ignoring {RELAY_ENV}={relay}: {error}");
            None
        }
    }
}

//! Where a Payjoin session sends its requests and how long it waits for the receiver

use std::time::Duration;

use rand::seq::SliceRandom as _;
use url::Url;

/// How long a sender waits for a receiver proposal before broadcasting the fallback transaction
///
/// The receiver's own pj expiry can be much later, so this bounds how long a payment waits. A
/// poll past the receiver's expiry is rejected by the payjoin crate and falls back at once
const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Public OHTTP relays Cove uses when nothing else is configured
const DEFAULT_OHTTP_RELAYS: [&str; 3] =
    ["https://relay.payjoin.org", "https://ohttp.achow101.com", "https://pj.bobspacebkk.com"];

/// Settings one Payjoin sender session runs with, owned by the wallet actor
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PayjoinConfig {
    relays: OhttpRelays,
    session_timeout: Duration,
}

impl PayjoinConfig {
    pub(crate) fn new(relays: OhttpRelays, session_timeout: Duration) -> Self {
        Self { relays, session_timeout }
    }

    pub(crate) fn relays(&self) -> &OhttpRelays {
        &self.relays
    }

    pub(crate) fn session_timeout(&self) -> Duration {
        self.session_timeout
    }
}

impl Default for PayjoinConfig {
    fn default() -> Self {
        Self { relays: OhttpRelays::default(), session_timeout: DEFAULT_SESSION_TIMEOUT }
    }
}

/// A non-empty list of OHTTP relays that a Payjoin session's requests go through
///
/// Every relay is an `https` URL, so the relay operator learns only the sender's IP and the
/// directory sees only the relay. Plain `http` loopback relays exist only in tests, through
/// `test_support`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OhttpRelays(Vec<Url>);

/// Why a list of OHTTP relays was rejected
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OhttpRelaysError {
    #[error("no OHTTP relays were given")]
    Empty,

    #[error("OHTTP relay {0} is not a valid URL")]
    InvalidUrl(String),

    #[error("OHTTP relay {0} must use https")]
    NotHttps(Url),
}

impl OhttpRelays {
    /// Validates a relay list, requiring at least one relay and https for each
    pub(crate) fn new(relays: impl IntoIterator<Item = Url>) -> Result<Self, OhttpRelaysError> {
        let relays = relays.into_iter().collect::<Vec<_>>();
        if relays.is_empty() {
            return Err(OhttpRelaysError::Empty);
        }

        if let Some(relay) = relays.iter().find(|relay| relay.scheme() != "https") {
            return Err(OhttpRelaysError::NotHttps(relay.clone()));
        }

        Ok(Self(relays))
    }

    /// Parses and validates relay URLs, as `new` does
    pub(crate) fn parse<S: AsRef<str>>(
        relays: impl IntoIterator<Item = S>,
    ) -> Result<Self, OhttpRelaysError> {
        let relays = relays
            .into_iter()
            .map(|relay| {
                let relay = relay.as_ref();
                Url::parse(relay).map_err(|_| OhttpRelaysError::InvalidUrl(relay.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(relays)
    }

    /// The relays in a fresh random order, so retries spread across relays and no relay sees
    /// every session first
    pub(crate) fn shuffled(&self) -> Vec<Url> {
        let mut relays = self.0.clone();
        relays.shuffle(&mut rand::rng());
        relays
    }
}

impl Default for OhttpRelays {
    fn default() -> Self {
        Self::parse(DEFAULT_OHTTP_RELAYS).expect("the built-in OHTTP relays are valid https URLs")
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{OhttpRelays, OhttpRelaysError, Url};

    impl OhttpRelays {
        /// Relays for tests that run their own relay, which may be plain `http` only when it
        /// listens on a loopback host
        pub(crate) fn loopback_for_test(
            relays: impl IntoIterator<Item = Url>,
        ) -> Result<Self, OhttpRelaysError> {
            let relays = relays.into_iter().collect::<Vec<_>>();
            if relays.is_empty() {
                return Err(OhttpRelaysError::Empty);
            }

            let not_allowed = relays.iter().find(|relay| match relay.scheme() {
                "https" => false,
                "http" => !is_loopback(relay),
                _ => true,
            });

            if let Some(relay) = not_allowed {
                return Err(OhttpRelaysError::NotHttps(relay.clone()));
            }

            Ok(Self(relays))
        }
    }

    fn is_loopback(url: &Url) -> bool {
        match url.host() {
            Some(url::Host::Domain(domain)) => domain == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_the_built_in_relays_and_ten_minutes() {
        let config = PayjoinConfig::default();

        let mut relays =
            config.relays().shuffled().into_iter().map(|url| url.to_string()).collect::<Vec<_>>();
        relays.sort();

        assert_eq!(
            relays,
            [
                "https://ohttp.achow101.com/",
                "https://pj.bobspacebkk.com/",
                "https://relay.payjoin.org/"
            ]
        );
        assert_eq!(config.session_timeout(), Duration::from_secs(600));
    }

    #[test]
    fn relays_must_be_non_empty_https() {
        assert_eq!(OhttpRelays::parse::<&str>([]), Err(OhttpRelaysError::Empty));
        assert!(matches!(
            OhttpRelays::parse(["http://127.0.0.1:3000"]),
            Err(OhttpRelaysError::NotHttps(_))
        ));
        assert!(matches!(OhttpRelays::parse(["not a url"]), Err(OhttpRelaysError::InvalidUrl(_))));
    }

    #[test]
    fn test_relays_allow_http_only_on_loopback() {
        let url = |s: &str| Url::parse(s).unwrap();

        assert!(OhttpRelays::loopback_for_test([url("http://localhost:3000")]).is_ok());
        assert!(OhttpRelays::loopback_for_test([url("http://127.0.0.1:3000")]).is_ok());
        assert!(OhttpRelays::loopback_for_test([url("http://[::1]:3000")]).is_ok());
        assert!(matches!(
            OhttpRelays::loopback_for_test([url("http://relay.example.com")]),
            Err(OhttpRelaysError::NotHttps(_))
        ));
    }
}

//! The saved Tor setting

use cove_http::{ProxyAddress, ProxyAddressError};
use cove_util::json::JsonDecodeError;
use serde::{Deserialize, Serialize};

/// How the app reaches the network
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TorSettings {
    /// Connections go straight to their destination
    Off,
    /// Connections go through the Tor client inside the app
    BuiltIn,
    /// Connections go through a Tor SOCKS5 proxy the user runs, such as Orbot
    Proxy(ProxyAddress),
}

/// [`TorSettings`] as the apps see it, with the proxy as text they can show and edit
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TorMode {
    Off,
    BuiltIn,
    Proxy { address: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum TorSettingsError {
    #[error(transparent)]
    InvalidProxy(#[from] ProxyAddressError),

    #[error("the saved Tor setting is unreadable: {0}")]
    Decode(String),
}

impl From<&TorSettings> for TorMode {
    fn from(settings: &TorSettings) -> Self {
        match settings {
            TorSettings::Off => Self::Off,
            TorSettings::BuiltIn => Self::BuiltIn,
            TorSettings::Proxy(address) => Self::Proxy { address: address.to_string() },
        }
    }
}

impl TryFrom<TorMode> for TorSettings {
    type Error = TorSettingsError;

    fn try_from(mode: TorMode) -> Result<Self, Self::Error> {
        let settings = match mode {
            TorMode::Off => Self::Off,
            TorMode::BuiltIn => Self::BuiltIn,
            TorMode::Proxy { address } => Self::Proxy(ProxyAddress::parse(&address)?),
        };

        Ok(settings)
    }
}

/// The JSON stored under `tor_settings`, with its names spelled out so renaming a Rust type never
/// changes what is on disk
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum StoredTorSettings {
    Off,
    BuiltIn,
    Proxy { address: String },
}

impl TorSettings {
    pub(crate) fn to_json(&self) -> String {
        let stored = match self {
            Self::Off => StoredTorSettings::Off,
            Self::BuiltIn => StoredTorSettings::BuiltIn,
            Self::Proxy(address) => StoredTorSettings::Proxy { address: address.to_string() },
        };

        serde_json::to_string(&stored).expect("Tor settings always serialize")
    }

    pub(crate) fn from_json(json: &str) -> Result<Self, TorSettingsError> {
        // serde's text quotes the input, which holds the proxy address, so only its category stays
        let stored: StoredTorSettings = serde_json::from_str(json)
            .map_err(|error| TorSettingsError::Decode(JsonDecodeError::from(error).to_string()))?;

        let settings = match stored {
            StoredTorSettings::Off => Self::Off,
            StoredTorSettings::BuiltIn => Self::BuiltIn,
            StoredTorSettings::Proxy { address } => Self::Proxy(ProxyAddress::parse(&address)?),
        };

        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_json_is_stable() {
        let proxy = TorSettings::Proxy(ProxyAddress::parse("[::1]:9050").unwrap());

        assert_eq!(TorSettings::Off.to_json(), r#"{"mode":"off"}"#);
        assert_eq!(TorSettings::BuiltIn.to_json(), r#"{"mode":"built_in"}"#);
        assert_eq!(proxy.to_json(), r#"{"mode":"proxy","address":"[::1]:9050"}"#);

        for settings in [TorSettings::Off, TorSettings::BuiltIn, proxy] {
            assert_eq!(TorSettings::from_json(&settings.to_json()).unwrap(), settings);
        }
    }

    #[test]
    fn unreadable_json_is_an_error() {
        assert!(TorSettings::from_json(r#"{"mode":"bridge"}"#).is_err());
        assert!(TorSettings::from_json(r#"{"mode":"proxy","address":"nope"}"#).is_err());
    }
}

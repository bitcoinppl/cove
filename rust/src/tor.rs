use serde::{Deserialize, Serialize};

/// Persisted Tor routing preference
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum TorConfig {
    /// Connect without Tor
    #[default]
    Off,
    /// Connect through Cove's built-in Tor runtime
    BuiltIn,
    /// Connect through an external SOCKS5 proxy
    External { host: String, port: u16 },
}

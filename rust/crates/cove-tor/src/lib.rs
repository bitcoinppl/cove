//! Tor for the app: the built-in Arti client and the local SOCKS5 proxy in front of it

pub mod bridge;
pub mod built_in;

pub use built_in::{BootstrapState, BuiltInTor, StartError};
pub use tor_socksproto::{SocksAddr, SocksStatus};

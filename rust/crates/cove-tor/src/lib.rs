//! Tor for the app: the built-in Arti client and the local SOCKS5 proxy in front of it

pub mod bridge;
pub mod built_in;
pub mod socks;

pub use built_in::{BootstrapState, BuiltInTor, BuiltInTorConfig, StartError};

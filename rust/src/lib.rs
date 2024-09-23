pub mod database;

pub(crate) mod app;
pub(crate) mod logging;
pub(crate) mod macros;
pub(crate) mod router;

pub mod autocomplete;
pub mod bip39;
pub mod color;
pub mod color_scheme;
pub mod consts;
pub mod encryption;
pub mod fiat;
pub mod format;
pub mod header_icon_presenter;
pub mod keychain;
pub mod keys;
pub mod mnemonic;
pub mod network;
pub mod node;
pub mod node_connect;
pub mod pending_wallet;
pub mod qr;
pub mod redb;
pub mod task;
pub mod transaction;
pub mod unblock;
pub mod util;
pub mod view_model;
pub mod wallet;
pub mod word_validator;
pub mod xpub;

uniffi::setup_scaffolding!();

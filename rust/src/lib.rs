pub(crate) mod database;

pub(crate) mod app;
pub(crate) mod logging;
pub(crate) mod router;

pub(crate) mod autocomplete;
pub(crate) mod bip39;
pub(crate) mod color;
pub(crate) mod color_scheme;
pub(crate) mod consts;
pub(crate) mod cove_nfc;
pub(crate) mod encryption;
pub(crate) mod ffi;
pub(crate) mod fiat;
pub(crate) mod format;
pub(crate) mod header_icon_presenter;
pub(crate) mod keychain;
pub(crate) mod keys;
pub(crate) mod mnemonic;
pub(crate) mod network;
pub(crate) mod node;
pub(crate) mod node_connect;
pub(crate) mod pending_wallet;
pub(crate) mod qr;
pub(crate) mod redb;
pub(crate) mod seed_qr;
pub(crate) mod task;
pub(crate) mod transaction;
pub(crate) mod unblock;
pub(crate) mod util;
pub(crate) mod view_model;
pub(crate) mod wallet;
pub(crate) mod wallet_scanner;
pub(crate) mod word_validator;
pub(crate) mod xpub;

uniffi::setup_scaffolding!();
pubport::uniffi_reexport_scaffolding!();

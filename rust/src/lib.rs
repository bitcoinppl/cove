mod database;

mod app;
mod logging;
mod router;

mod auth;
mod autocomplete;
mod bdk_store;
mod bip39;
mod build;
mod color;
mod color_scheme;
mod consts;
mod converter;
// cove_nfc is now an external crate
mod cove_nfc;
mod device;
mod encryption;
mod fiat;
mod file_handler;
mod format;
mod hardware_export;
mod keychain;
mod keys;
mod label_manager;
mod manager;
mod mnemonic;
mod multi_format;
mod multi_qr;
mod network;
mod node;
mod node_connect;
mod pending_wallet;
mod psbt;
mod push_tx;
mod redb;
mod seed_qr;
mod send_flow;
mod tap_card;
mod task;
mod transaction;
mod transaction_watcher;
mod unblock;
mod wallet;
mod wallet_scanner;
mod word_validator;
mod xpub;

uniffi::setup_scaffolding!();

mod database;

mod app;
mod router;

mod auth;
mod autocomplete;
mod bdk_store;
mod build;
mod converter;
mod fee_client;
mod fiat;
mod file_handler;
mod hardware_export;
mod historical_price_service;
mod keys;
mod label_manager;
mod manager;
mod mnemonic;
mod multi_format;
mod node;
mod node_connect;
mod pending_wallet;
mod push_tx;
mod qr_scanner;
mod reporting;
mod seed_qr;
mod send_flow;
mod tap_card;
mod task;
mod transaction;
mod transaction_watcher;
mod unblock;
mod ur;
mod wallet;
mod wallet_scanner;
mod word_validator;
mod xpub;

::cove_tap_card::uniffi_reexport_scaffolding!();
::cove_util::uniffi_reexport_scaffolding!();
::cove_nfc::uniffi_reexport_scaffolding!();
::cove_types::uniffi_reexport_scaffolding!();
::cove_device::uniffi_reexport_scaffolding!();

uniffi::setup_scaffolding!();

// re-export types from crates that are are used
use cove_common::logging;
use cove_device::device;
use cove_device::keychain;
use cove_types::color;
use cove_types::color_scheme;
use cove_types::network;
use cove_types::psbt;

use std::path::PathBuf;

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi(flat_error)]
pub enum InitError {
    #[error("Failed to set root data directory: {0}")]
    RootDataDirAlreadySet(String),
}

/// set root data directory before any database access
/// required for Android to specify app-specific storage path
#[uniffi::export]
fn set_root_data_dir(path: String) -> Result<(), InitError> {
    cove_common::consts::set_root_data_dir(PathBuf::from(path))
        .map_err(InitError::RootDataDirAlreadySet)
}

mod database;

mod app;
mod router;

mod auth;
mod autocomplete;
mod bdk_store;
mod build;
mod converter;
mod fiat;
mod file_handler;
mod hardware_export;
mod historical_price_service;
mod keys;
mod label_manager;
mod manager;
mod mnemonic;
mod multi_format;
mod multi_qr;
mod node;
mod node_connect;
mod pending_wallet;
mod psbt;
mod push_tx;
mod reporting;
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

::cove_tap_card::uniffi_reexport_scaffolding!();
::cove_util::uniffi_reexport_scaffolding!();
::rust_cktap::uniffi_reexport_scaffolding!();
::cove_nfc::uniffi_reexport_scaffolding!();
::cove_types::uniffi_reexport_scaffolding!();
::cove_common::uniffi_reexport_scaffolding!();

uniffi::setup_scaffolding!();

// re-export types from crates that are are used
use cove_common::logging;
use cove_device::device;
use cove_device::keychain;
use cove_types::color;
use cove_types::color_scheme;
use cove_types::network;

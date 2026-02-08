// Allow lints that are problematic due to uniffi requirements or too invasive
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::missing_fields_in_debug)]
#![allow(clippy::inline_always)]
#![allow(clippy::option_option)]
#![allow(clippy::trivially_copy_pass_by_ref)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::implicit_clone)]
#![allow(clippy::unchecked_time_subtraction)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::default_trait_access)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::needless_collect)]
#![allow(clippy::redundant_closure_for_method_calls)]

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
mod loading_popup;
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
mod signed_import;
mod tap_card;
mod transaction;
mod transaction_watcher;
mod unblock;
mod ur;
mod wallet;
mod wallet_scanner;
mod word_validator;
mod word_verify_state_machine;
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

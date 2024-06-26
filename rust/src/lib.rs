pub mod database;

pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod logging;
pub(crate) mod macros;
pub(crate) mod router;
pub(crate) mod update;

pub mod keychain;
pub mod keys;
pub mod view_model;
pub mod wallet;
pub mod autocomplete;

uniffi::setup_scaffolding!();

pub mod database;

pub(crate) mod app;
pub(crate) mod logging;
pub(crate) mod macros;
pub(crate) mod router;

pub mod autocomplete;
pub mod keychain;
pub mod keys;
pub mod redb;
pub mod view_model;
pub mod wallet;
pub mod word_validator;

uniffi::setup_scaffolding!();

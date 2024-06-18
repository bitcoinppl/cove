pub mod database;

pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod logging;
pub(crate) mod macros;
pub(crate) mod router;
pub(crate) mod update;

pub mod view_model;
pub mod wallet;

uniffi::setup_scaffolding!();

pub mod database;

pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod logging;
pub(crate) mod macros;
pub(crate) mod router;
pub(crate) mod update;

uniffi::setup_scaffolding!();

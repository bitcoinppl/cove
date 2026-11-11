//! Owners of Cloud Backup facts
//!
//! Each owner is a sibling module here, never a child of a worker, so a worker cannot reach
//! owner state through parent privacy. Producers hand their results to an owner as typed
//! inputs; only the owner decides what is persisted

pub(crate) mod lifecycle;

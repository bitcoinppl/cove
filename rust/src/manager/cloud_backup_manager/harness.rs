//! Real-manager sequence-test harness for Cloud Backup
//!
//! Each harness test body runs in its own child process (see [`process`]), so the case owns the
//! Tokio runtime every `spawn_actor` lands on, the provider globals and the database file.
//! Phase 0 decisions behind this design live in the PR 1 description

mod app;
mod clock;
mod crash_tests;
mod deadline;
mod fakes;
mod fixtures;
mod incident_tests;
mod invariants;
mod model_tests;
mod process;
mod reset_tests;
mod runtime;
mod tests;
mod write_policy_tests;

pub(crate) use self::app::{NextAction, TestApp, open_durable_state};
pub(crate) use self::fakes::{
    CallId, CloudCall, CloudFault, HarnessCloud, HarnessPasskey, Hold, PasskeyCall, PasskeyMode,
};
pub(crate) use self::fixtures::{
    FakeAccount, change_a_wallet, keychain_namespace, load_detail, persisted,
    seed_legacy_stuck_disabling,
};
pub(crate) use self::process::{PhaseEnd, run_case};

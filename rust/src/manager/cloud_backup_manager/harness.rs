//! Real-manager sequence-test harness for Cloud Backup
//!
//! Each harness test body runs in its own child process (see [`process`]), so the case owns the
//! Tokio runtime every `spawn_actor` lands on, the provider globals and the database file

mod account_switch_tests;
mod app;
mod batch_delete_tests;
mod clock;
mod continuation_tests;
mod crash_tests;
mod deadline;
mod enable_entry_tests;
mod fakes;
mod finishing_tests;
mod fixtures;
mod incident_tests;
mod invariants;
mod model_tests;
mod owner_tests;
mod passkey_tests;
mod process;
mod recovered_enable_tests;
mod reset_tests;
mod row_delete_tests;
mod runtime;
mod tests;
mod write_policy_tests;

pub(crate) use self::app::{NextAction, TestApp, launch_keychain, open_durable_state};
pub(crate) use self::fakes::{
    CallId, CloudCall, CloudFault, HarnessCloud, HarnessPasskey, Hold, PasskeyCall, PasskeyMode,
};
pub(crate) use self::fixtures::{
    FakeAccount, SeededRecord, assert_uploads_confirmed_in, change_a_wallet, finish_wipe,
    keychain_namespace, load_detail, persisted, seed_an_uploaded_wallet, seed_corrupted_record,
    seed_legacy_stuck_disabling, seed_other_namespace, seed_verifiable_master_key,
    seed_wallet_backup_elsewhere, verifiable_wrapper,
};
pub(crate) use self::process::{PhaseEnd, run_case};

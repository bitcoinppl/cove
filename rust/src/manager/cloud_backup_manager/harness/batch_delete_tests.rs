//! Delete Unreadable Backups: what the screen says when a batch stops before its last record
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use cove_device::cloud_storage::CloudStorageError;

use super::fixtures::seed_undecryptable_records;
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PhaseEnd, TestApp, load_detail,
    run_case,
};
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupOutcome, CloudBackupOutcomeTarget, CloudBackupScreenAction,
};

/// A batch the provider stops after it deleted a record says how many it deleted and why it
/// stopped, next to Delete Unreadable Backups
#[test]
fn a_batch_a_provider_failure_stops_partway_says_what_it_deleted() {
    let test =
        concat!(module_path!(), "::a_batch_a_provider_failure_stops_partway_says_what_it_deleted");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let (mut app, namespace, records, first_delete) = start_held_batch();

        // the provider goes offline before the lane reads the second record
        app.event("the provider goes offline after the first delete", |app| {
            app.cloud.script_next(
                CloudCall::DownloadWallet,
                CloudFault::Fail(CloudStorageError::Offline("offline".into())),
            );
            first_delete.release();
        })
        .then_cannot(&[N::Enable]);

        assert!(!app.cloud.inner.has_wallet_backup(&namespace, &records[0]));
        assert!(app.cloud.inner.has_wallet_backup(&namespace, &records[1]));
        assert_eq!(
            batch_outcome(&app),
            Some(CloudBackupOutcome::Failed {
                message: "Deleted 1 inaccessible wallet backup, then stopped. Reconnect to the \
                          internet, then try deleting the inaccessible wallet backups again"
                    .into(),
            })
        );
    });
}

/// A batch whose inventory changed after it deleted a record says how many it deleted and that
/// the rest wait for a fresh check, next to Delete Unreadable Backups
#[test]
fn a_stale_batch_stopped_partway_says_what_it_deleted() {
    let test = concat!(module_path!(), "::a_stale_batch_stopped_partway_says_what_it_deleted");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let (mut app, namespace, records, first_delete) = start_held_batch();

        // the local wallet set moves the inventory the batch was authorized against
        app.event("the local wallet set changes while the first delete runs", |app| {
            app.manager().handle_wallet_set_change();
            first_delete.release();
        })
        .then_cannot(&[N::Enable]);

        assert!(!app.cloud.inner.has_wallet_backup(&namespace, &records[0]));
        assert!(app.cloud.inner.has_wallet_backup(&namespace, &records[1]));
        assert_eq!(
            batch_outcome(&app),
            Some(CloudBackupOutcome::Failed {
                message: "Deleted 1 inaccessible wallet backup, then stopped because your \
                          backups changed. Check again before deleting the rest."
                    .into(),
            })
        );
    });
}

/// Two undecryptable records on the loaded detail screen, with Delete Unreadable Backups tapped
/// and the provider holding its first delete
fn start_held_batch() -> (TestApp, String, Vec<String>, Hold) {
    let mut app = TestApp::launch();
    let account = FakeAccount::new(1, 0).seed_records(&app);
    let records = seed_undecryptable_records(&app, &account, 2);
    load_detail(&mut app);
    let consent = app.undecryptable_delete().expect("the screen offers Delete Unreadable Backups");

    let first_delete = Hold::default();
    app.cloud.script_next(CloudCall::DeleteWallet, CloudFault::Hold(first_delete.clone()));
    app.dispatch(CloudBackupManagerAction::DeleteUndecryptableWalletBackups(consent))
        .then_cannot(&[N::Enable]);
    app.block_on("the first delete to reach the provider", first_delete.entered());

    (app, account.namespace, records, first_delete)
}

/// The outcome shown next to Delete Unreadable Backups
fn batch_outcome(app: &TestApp) -> Option<CloudBackupOutcome> {
    let target = CloudBackupOutcomeTarget::Action(CloudBackupScreenAction::DeleteUnreadableBackups);

    app.manager()
        .published_screen()
        .last_outcome
        .filter(|outcome| outcome.target == target)
        .map(|outcome| outcome.outcome)
}

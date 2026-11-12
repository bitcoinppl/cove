//! Delete on a Not on This Device row: what the screen says when the delete ends without
//! deleting, while the rows it left are read again
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_device::cloud_storage::CloudStorageError;

use super::{
    CallId, CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd,
    TestApp, change_a_wallet, load_detail, run_case,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupOutcome, CloudBackupOutcomeTarget, CloudBackupRowControl, CloudBackupScreenBody,
    CloudBackupSection,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupManagerAction, CloudOnlyOperation, RecordId,
};

/// A confirmed row delete that went stale in the write queue deleted nothing, and says so next
/// to the row while the rows it left are read again, however long that read takes
#[test]
fn a_stale_row_delete_says_so_while_its_rows_are_read_again() {
    let test =
        concat!(module_path!(), "::a_stale_row_delete_says_so_while_its_rows_are_read_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let (mut app, namespace, elsewhere) = launch_with_a_row_not_on_device();

        // an upload holds the write queue, so the confirmed delete waits behind it
        let upload = Hold::default();
        app.cloud.script_next(CloudCall::UploadWallet, CloudFault::Hold(upload.clone()));
        app.event("a wallet's backed-up metadata changes", |app| {
            change_a_wallet(app).expect("the device has a wallet");
        })
        .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);
        app.block_on("the upload to reach the provider", upload.entered());

        let consent = app.row_delete(&elsewhere).expect("the row's Delete");
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(consent))
            .then_cannot(&[N::Enable]);

        // the wallet set changes before the delete reaches the front of the queue, and the
        // provider never answers the listings that read the rows again
        let listings = hold_every_listing(&app);
        app.event("the local wallet set changes, then the upload finishes", |app| {
            app.manager().handle_wallet_set_change();
            upload.release();
        })
        .then_cannot(&[N::Enable]);

        assert!(app.cloud.inner.has_wallet_backup(&namespace, &elsewhere));
        assert_eq!(app.cloud.count(CloudCall::DeleteWallet), 0);
        assert!(app.row_delete(&elsewhere).is_none(), "the rows are still being read");
        assert_eq!(
            shown_failures(&app, &elsewhere),
            vec![
                "This backup changed since you checked, so nothing was deleted. Check again."
                    .to_owned()
            ],
            "{}",
            app.describe()
        );

        listings.iter().for_each(Hold::release);
    });
}

/// A row delete the provider refused deleted nothing, and says why next to the row even after a
/// wallet change starts reading the rows again
#[test]
fn a_failed_row_delete_says_why_while_its_rows_are_read_again() {
    let test =
        concat!(module_path!(), "::a_failed_row_delete_says_why_while_its_rows_are_read_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let (mut app, namespace, elsewhere) = launch_with_a_row_not_on_device();

        app.cloud.script_next(
            CloudCall::DeleteWallet,
            CloudFault::Fail(CloudStorageError::Offline("offline".into())),
        );
        let consent = app.row_delete(&elsewhere).expect("the row's Delete");
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(consent))
            .then_cannot(&[N::Enable]);
        let failed = shown_failures(&app, &elsewhere);
        assert_eq!(failed.len(), 1, "{failed:?}: {}", app.describe());

        let listings = hold_every_listing(&app);
        app.event("the local wallet set changes", |app| app.manager().handle_wallet_set_change())
            .then_cannot(&[N::Enable]);

        assert!(app.cloud.inner.has_wallet_backup(&namespace, &elsewhere));
        assert!(app.row_delete(&elsewhere).is_none(), "the rows are still being read");
        assert_eq!(shown_failures(&app, &elsewhere), failed, "{}", app.describe());

        listings.iter().for_each(Hold::release);
    });
}

/// A configured backup of one local wallet and one wallet not on this device, with the detail
/// screen loaded; returns the namespace and the record not on this device
fn launch_with_a_row_not_on_device() -> (TestApp, String, String) {
    let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
    let account = FakeAccount::new(1, 1).seed_records(&app);
    let [elsewhere] = account.not_on_device.as_slice() else {
        // nothing read from the account: a panic message reaches the test log
        panic!("expected exactly one record not on this device");
    };
    let elsewhere = elsewhere.clone();
    load_detail(&mut app);

    (app, account.namespace, elsewhere)
}

/// Holds every listing from here on, so no fetch that reads the rows again answers until the
/// test releases them all
///
/// A fetch under a newer listing replaces a held one rather than waiting for it, so holding
/// only the next listing would let the fetch that matters answer
fn hold_every_listing(app: &TestApp) -> Vec<Hold> {
    // more listings than any one of these events starts
    const LISTINGS: usize = 10;

    let next = app.cloud.count(CloudCall::ListWalletFiles) + 1;
    (next..next + LISTINGS)
        .map(|nth| {
            let hold = Hold::default();
            app.cloud.script(
                CallId::new(CloudCall::ListWalletFiles, nth),
                CloudFault::Hold(hold.clone()),
            );
            hold
        })
        .collect()
}

/// Every failure message the screen shows for a delete of `record_id`: the outcome next to its
/// row's Delete, and the failure of the section's row operation
///
/// One delete that failed shows exactly one message, so the row and its section don't both say
/// it, and the message stays while the rows are read again
fn shown_failures(app: &TestApp, record_id: &str) -> Vec<String> {
    let at_row = row_outcome(app, record_id).map(|outcome| match outcome {
        CloudBackupOutcome::Failed { message } => message,
        other => format!("{other:?}"),
    });

    let screen = app.manager().published_screen();
    let CloudBackupScreenBody::Configured(configured) = &screen.body else {
        return at_row.into_iter().collect();
    };

    let in_section = configured.sections.iter().find_map(|section| match section {
        CloudBackupSection::NotOnDevice(section) => match &section.operation {
            CloudOnlyOperation::Failed { error } => Some(error.clone()),
            _ => None,
        },
        _ => None,
    });

    at_row.into_iter().chain(in_section).collect()
}

/// The outcome shown next to the Delete of `record_id`'s row
fn row_outcome(app: &TestApp, record_id: &str) -> Option<CloudBackupOutcome> {
    let record_id = RecordId::from(record_id.to_owned());
    let at_delete = |target: &CloudBackupOutcomeTarget| match target {
        CloudBackupOutcomeTarget::Row { target } => {
            target.record_id == record_id && target.control == CloudBackupRowControl::Delete
        }
        CloudBackupOutcomeTarget::Action(_) => false,
    };

    app.manager()
        .published_screen()
        .last_outcome
        .filter(|outcome| at_delete(&outcome.target))
        .map(|outcome| outcome.outcome)
}

//! The incident replay: the reporter's lived-in account with a Turn Off stuck before its delete,
//! walked through what the reporter's device did, with the next actions asserted at every step
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use super::{
    CloudCall, FakeAccount, NextAction as N, PasskeyMode, PhaseEnd, TestApp, change_a_wallet,
    keychain_namespace, load_detail, persisted, run_case, seed_legacy_stuck_disabling,
};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableConfirmation, CloudBackupScreenBody, CloudBackupSection,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

/// What the platform sends when the app comes back to the foreground with a connection
fn foreground(app: &TestApp) {
    let manager = app.manager();
    manager.handle_connectivity_change(ConnectivityStatus::Connected);
    manager.cloud_storage_did_change();
    manager.resume_pending_cloud_upload_verification();
}

/// How many rows the published detail lists as own backups, not on this device, and extra copies
fn listed_counts(app: &TestApp) -> (usize, usize, usize) {
    let CloudBackupScreenBody::Configured(configured) = app.manager().published_screen().body
    else {
        panic!("the detail is not configured:\n{}", app.describe());
    };

    configured.sections.iter().fold((0, 0, 0), |(own, elsewhere, extra), section| match section {
        CloudBackupSection::Wallets(wallets) => {
            (own + wallets.up_to_date.len() + wallets.needs_sync.len(), elsewhere, extra)
        }
        CloudBackupSection::NotOnDevice(section) => (own, elsewhere + section.rows.len(), extra),
        CloudBackupSection::ExtraCopies(section) => (own, elsewhere, extra + section.rows.len()),
        _ => (own, elsewhere, extra),
    })
}

#[test]
fn incident_replay_on_the_lived_in_account_always_has_a_way_out() {
    let test =
        concat!(module_path!(), "::incident_replay_on_the_lived_in_account_always_has_a_way_out");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let app = TestApp::launch();
            let namespace = FakeAccount::lived_in().seed(&app);
            seed_legacy_stuck_disabling(&namespace, app.clock.now_secs());
            return;
        }

        // the reporter opens the app after the stuck Turn Off: the record heals at load, and
        // nothing reaches the passkey provider on any step that follows without a tap
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current()
            .then_can(&[N::Verify, N::SyncUnsynced])
            .then_cannot(&[N::Enable, N::RestoreFromCloudBackup]);
        let namespace = keychain_namespace().expect("the heal keeps the namespace");
        assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)));

        // the device came back online and iCloud changed, again and again
        for _ in 0..3 {
            app.event("foreground with a connection", foreground)
                .then_can(&[N::Verify, N::SyncUnsynced])
                .then_cannot(&[N::Enable]);
        }
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0, "a resume deleted the backup");

        // the detail screen loads its inventory silently and offers Turn Off, listing each local
        // wallet's own backup and each backup of a wallet not on this device
        load_detail(&mut app);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
        assert_eq!(listed_counts(&app), (31, 4, 0), "own, not on this device, extra copies");

        // a wallet edited meanwhile uploads again, since nothing holds writes any more
        let uploads = app.cloud.count(CloudCall::UploadWallet);
        app.event("a local wallet changes", |app| {
            change_a_wallet(app).expect("a lived-in wallet");
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads + 1);

        // the app is closed and opened again: still configured, with the notice still there
        app.relaunch().then_can(&[N::Verify]).then_cannot(&[N::Enable]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the relaunch lost the healed configuration:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some(), "the heal notice is persisted");

        // Turn Off now runs instead of hanging: it shows what would be lost while records that are
        // not on this device remain, asks twice, and deletes nothing without consent. The review
        // is presentation, so it holds nothing else, and Cancel leaves every way out
        load_detail(&mut app);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable, N::CancelDisable, N::Verify])
            .then_cannot(&[N::Enable]);
        let review = app.turn_off_review().expect("the Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two);
        assert!(
            review.impact.not_on_device.names.len() + review.impact.unchecked as usize > 0,
            "the review hides the records not on this device: {review:?}"
        );

        app.dispatch(CloudBackupManagerAction::CancelDisable)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::ConfirmDisable, N::Enable]);
        assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)));
        assert!(app.cloud.inner.has_namespace(&namespace), "a review deleted the backup");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);

        // and coming back to the app keeps every way out, still without a passkey prompt
        app.event("foreground with a connection", foreground)
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

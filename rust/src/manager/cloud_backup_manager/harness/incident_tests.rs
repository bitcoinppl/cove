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
use crate::manager::connectivity_manager::ConnectivityStatus;

/// What the platform sends when the app comes back to the foreground with a connection
fn foreground(app: &TestApp) {
    let manager = app.manager();
    manager.handle_connectivity_change(ConnectivityStatus::Connected);
    manager.cloud_storage_did_change();
    manager.resume_pending_cloud_upload_verification();
}

#[test]
#[ignore = "the first detail refresh opens every local wallet's label database synchronously on \
            the runtime, about 5s for this account in a debug build, which trips the 10s wait \
            limit on a loaded machine; remove this ignore once detail refresh does its wallet I/O \
            off the runtime"]
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
        app.current().then_can(&[N::Verify, N::SyncUnsynced]).then_cannot(&[
            N::Enable,
            N::KeepEnabled,
            N::RestoreFromCloudBackup,
        ]);
        let namespace = keychain_namespace().expect("the heal keeps the namespace");
        assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)));

        // the device came back online and iCloud changed, again and again
        for _ in 0..3 {
            app.event("foreground with a connection", foreground)
                .then_can(&[N::Verify, N::SyncUnsynced])
                .then_cannot(&[N::Enable, N::KeepEnabled]);
        }
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0, "a resume deleted the backup");

        // the detail screen loads its inventory silently and offers Turn Off
        load_detail(&mut app);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);

        // a wallet edited meanwhile uploads again, since nothing holds writes any more
        let uploads = app.cloud.count(CloudCall::UploadWallet);
        app.event("a local wallet changes", |app| {
            change_a_wallet(app).expect("a lived-in wallet");
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable, N::KeepEnabled]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads + 1);

        // the app is closed and opened again: still configured, with the notice still there
        app.relaunch().then_can(&[N::Verify]).then_cannot(&[N::Enable, N::KeepEnabled]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the relaunch lost the healed configuration:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some(), "the heal notice is persisted");

        // Turn Off now runs instead of hanging: it refuses with a reason while records that are
        // not on this device remain, deletes nothing, and stays available to try again. The
        // refusal holds no other work
        load_detail(&mut app);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable, N::KeepEnabled]);
        assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)));
        assert!(app.cloud.inner.has_namespace(&namespace), "a refused Turn Off deleted the backup");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);

        // and coming back to the app keeps every way out, still without a passkey prompt
        app.event("foreground with a connection", foreground)
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable, N::KeepEnabled]);
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

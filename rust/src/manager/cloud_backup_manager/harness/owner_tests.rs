//! The lifecycle owner's own facts: how an operation finishes, the record it rebases on, and
//! what a failed local reset leaves
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use act_zero::call;
use cove_device::cloud_storage::CloudAccessPolicy;

use super::invariants::Invariant;
use super::process::checkpoint;
use super::{CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd};
use super::{
    SeededRecord, TestApp, change_a_wallet, keychain_namespace, load_detail, open_durable_state,
    persisted, run_case, seed_verifiable_master_key, verifiable_wrapper,
};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::cleanup::CloudBackupCleanupJob;
use crate::manager::cloud_backup_manager::actors::supervisor::Finish;
use crate::manager::cloud_backup_manager::actors::write::{CommandClass, Decision, LaneQueue};
use crate::manager::cloud_backup_manager::actors::{
    CleanupExpectedWalletRecord, CleanupSourceNamespace,
};
use crate::manager::cloud_backup_manager::admission::Admission;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::ops::test_support::tap;
use crate::manager::cloud_backup_manager::owner::lifecycle::{LifecycleLoad, LifecycleWriter};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupInventoryControl, CloudBackupProgressSection, CloudBackupScreenBody,
    CloudBackupSection,
};
use crate::manager::cloud_backup_manager::wallets::PermitIntent;
use crate::manager::cloud_backup_manager::{
    CloudBackupInventoryIncompleteReason, CloudBackupManagerAction, CloudBackupResetPermit,
    CloudBackupResetRecovery, CloudBackupRuntimeOwnership, CloudBackupStatus,
    CloudBackupVerificationSource,
};

fn running_claim(app: &TestApp) -> Option<CloudBackupExclusiveOperationClaim> {
    let supervisor = app.manager().supervisor.clone();
    app.block_on(
        "the running claim",
        async move { call!(supervisor.harness_running_claim()).await },
    )
    .expect("the supervisor answers")
}

/// What the write lane decides now for an ordinary write
fn ordinary_decision(app: &TestApp) -> Decision {
    let writes = app.writes();
    app.block_on("the write lane's ticket", async move { call!(writes.ticket()).await })
        .expect("the write lane answers")
        .ordinary
}

/// Starts Turn Off and holds it at its namespace delete; returns the hold and its claim
fn held_turn_off(app: &mut TestApp) -> (Hold, CloudBackupExclusiveOperationClaim) {
    FakeAccount::new(1, 0).seed(app);
    load_detail(app);
    let hold = Hold::default();
    app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(hold.clone()));

    app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
    app.confirm_turn_off().then_cannot(&[N::TurnOff, N::Enable]);
    app.block_on("the held delete to start", hold.entered());

    let claim = running_claim(app).expect("Turn Off runs");
    (hold, claim)
}

/// A finish names the operation it ends: a second finish of an operation that already ended,
/// arriving while another runs, ends nothing, and invariant 5 reports the second finish
#[test]
fn a_second_finish_of_an_ended_operation_ends_nothing_and_breaks_invariant_5() {
    let test = concat!(
        module_path!(),
        "::a_second_finish_of_an_ended_operation_ends_nothing_and_breaks_invariant_5"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (hold, first) = held_turn_off(&mut app);
        app.event("release the first Turn Off", |_| hold.release())
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(running_claim(&app), None);

        let (hold, second) = held_turn_off(&mut app);
        assert_ne!(first, second);

        let supervisor = app.manager().supervisor.clone();
        let finish = app
            .block_on("a second finish of the first Turn Off", async move {
                call!(supervisor.harness_finish(first)).await
            })
            .expect("the supervisor answers");

        assert_eq!(finish, Finish::NotCurrent { running: Some(second) });
        assert_eq!(running_claim(&app), Some(second), "the second finish ended another operation");

        let violation = app.settled_violation().expect("invariant 5 sees the second finish");
        assert_eq!(violation.invariant, Invariant::OneFinishOneAdmission, "{}", violation.detail);

        hold.release();
        app.settle();
    });
}

/// A record read error is not another record: an operation that starts and finishes while the
/// record can't be read leaves the configured body, its admission and its write policy
#[test]
fn an_operation_finishing_while_the_record_is_unreadable_keeps_the_configured_body() {
    let test = concat!(
        module_path!(),
        "::an_operation_finishing_while_the_record_is_unreadable_keeps_the_configured_body"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let fault = Database::global().cloud_backup_state.fail_reads();
        app.dispatch_unchecked(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable, N::RestoreFromCloudBackup]);

        let outcome = app.last_outcome().expect("the verification's admission is recorded");
        assert_eq!(outcome.admission, Admission::Accepted);
        assert_eq!(running_claim(&app), None, "the verification did not finish");
        assert_eq!(ordinary_decision(&app), Decision::Run, "the owner published another policy");
        drop(fault);

        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
    });
}

/// Bootstrap, then fail the next record read, which is the new owner's first
fn bootstrap_then_fail_the_next_read(
    ownership: &CloudBackupRuntimeOwnership,
) -> (LifecycleWriter, LifecycleLoad) {
    let bootstrap = LifecycleWriter::bootstrap(ownership);
    Database::global().cloud_backup_state.fail_next_reads(1);
    bootstrap
}

/// The owner starts from the record bootstrap read: a read error right after a successful
/// bootstrap leaves the configured body, not an off one
#[test]
fn a_read_error_after_bootstrap_keeps_the_configured_body() {
    let test = concat!(module_path!(), "::a_read_error_after_bootstrap_keeps_the_configured_body");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            return;
        }

        let mut app = TestApp::launch_with_bootstrap(
            PasskeyMode::Succeeds,
            bootstrap_then_fail_the_next_read,
        );
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
        assert_eq!(ordinary_decision(&app), Decision::Run, "the owner published another policy");
    });
}

/// Seeds a configured backup and prepares a local reset of it, as a full wipe does
fn configured_reset(app: &mut TestApp) -> CloudBackupResetPermit {
    FakeAccount::new(1, 0).seed(app);
    let manager = app.manager().clone();
    app.block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit")
}

/// Clears the local keys through the owner, as the wipe does before its database reset
fn clear_keys(app: &TestApp, permit: &CloudBackupResetPermit) {
    app.block_on("the owner to clear local state", permit.clear_local_state())
        .expect("clear local state");
}

/// Changes a wallet while the reset holds the lane, so a write would be queued if any could run
fn change_a_wallet_during_the_reset(app: &mut TestApp) {
    let mut changed = None;
    app.event("a local wallet changes during the reset", |app| {
        changed = change_a_wallet(app);
    })
    .then_cannot(&[N::TurnOff, N::Enable]);
    assert!(changed.is_some(), "no seeded wallet changed");
}

/// Ends the reset as a wipe that failed before its database reset
fn abandon(
    app: &mut TestApp,
    permit: CloudBackupResetPermit,
    name: &str,
) -> CloudBackupResetRecovery {
    let mut recovery = None;
    app.event(name, |app| {
        let ended = app.block_on("the reset to end", permit.resume_after_failed_reset());
        recovery = Some(ended.expect("abandon the reset"));
    })
    .then_cannot(&[N::ResumeDisable]);
    recovery.expect("the reset ended")
}

/// A wipe that fails before the owner clears the local keys leaves the backup as it was
#[test]
fn a_reset_abandoned_before_its_key_clear_resumes_the_configured_backup() {
    let test = concat!(
        module_path!(),
        "::a_reset_abandoned_before_its_key_clear_resumes_the_configured_backup"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        change_a_wallet_during_the_reset(&mut app);
        let uploads = app.cloud.count(CloudCall::UploadWallet);

        let recovery = abandon(&mut app, permit, "the wipe fails before the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);

        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(ordinary_decision(&app), Decision::Run);
        assert_eq!(
            app.cloud.count(CloudCall::UploadWallet),
            uploads + 1,
            "the wake did not upload"
        );
    });
}

/// A wipe that fails after the owner began clearing the local keys can't take the clear back:
/// the reset ends with Cloud Backup off, and no write runs for the backup it had
#[test]
fn a_reset_abandoned_after_its_key_clear_ends_with_backup_off() {
    let test =
        concat!(module_path!(), "::a_reset_abandoned_after_its_key_clear_ends_with_backup_off");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        clear_keys(&app, &permit);
        change_a_wallet_during_the_reset(&mut app);
        let uploads = app.cloud.count(CloudCall::UploadWallet);

        let recovery = abandon(&mut app, permit, "the wipe fails after the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::SafelyDisabled);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::Verify, N::SyncUnsynced]);

        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_ne!(ordinary_decision(&app), Decision::Run);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);

        app.advance_clock(Duration::from_secs(600)).then_can(&[N::Enable]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads, "an upload ran without keys");
    });
}

/// When Cloud Backup can't be turned off before the key clear, the owner refuses the clear: the
/// keys stay with the configured backup, which the abandoned reset and the next launch resume
#[test]
fn a_reset_that_cannot_turn_off_keeps_its_keys_and_resumes_the_configured_backup() {
    let test = concat!(
        module_path!(),
        "::a_reset_that_cannot_turn_off_keeps_its_keys_and_resumes_the_configured_backup"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 1 {
            let mut app = TestApp::launch();
            app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
            assert!(keychain_namespace().is_some(), "the relaunch found no keys");
            return;
        }

        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        let namespace = keychain_namespace();
        let configured = persisted();

        Database::global()
            .cloud_backup_state
            .fail_next_write_where(|state| matches!(state, PersistedCloudBackupState::Disabled));
        let cleared = app.block_on("the owner to refuse the key clear", permit.clear_local_state());
        assert!(cleared.is_err(), "the keys were cleared while the record says configured");
        assert_eq!(keychain_namespace(), namespace);
        assert_eq!(persisted(), configured);

        let recovery = abandon(&mut app, permit, "the wipe fails at its key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
        assert_eq!(ordinary_decision(&app), Decision::Run);
    });
}

/// The process dies after the reset cleared the local keys and before its database reset: the
/// next launch reads a record that already says Cloud Backup is off
#[test]
fn a_reset_killed_after_its_key_clear_relaunches_with_backup_off() {
    let test =
        concat!(module_path!(), "::a_reset_killed_after_its_key_clear_relaunches_with_backup_off");
    run_case(test, &[PhaseEnd::KillAt("reset-keys-cleared"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            let permit = configured_reset(&mut app);
            clear_keys(&app, &permit);

            checkpoint("reset-keys-cleared");
            unreachable!("the parent kills this process after the key clear");
        }

        open_durable_state();
        assert_eq!(keychain_namespace(), None, "the clear kept the namespace");
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);

        let mut app = TestApp::launch();
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::Verify, N::SyncUnsynced]);
        assert_ne!(ordinary_decision(&app), Decision::Run);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);
    });
}

/// A namespace a merge left behind: its wrapper opens with the same passkey and it holds the
/// same wallet as the active namespace, so a cleanup proves it redundant and a reinitialize
/// with no local master key would recover it
fn seed_merged_source(app: &TestApp, record: &SeededRecord) -> String {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    app.cloud.inner.set_master_key_backup(namespace.clone(), verifiable_wrapper(&master_key));
    app.cloud.inner.set_wallet_files(
        namespace.clone(),
        vec![cove_cspp::backup_data::wallet_filename_from_record_id(&record.record_id)],
    );
    app.cloud.persist();
    namespace
}

/// Queues the cleanup a merge into `active` schedules for `source`, under the current
/// configuration
fn schedule_merge_cleanup(app: &TestApp, active: &str, source: &str, record: &SeededRecord) {
    let master_key = cove_cspp::Cspp::new(cove_device::keychain::Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .expect("active master key");
    let writes = app.writes();
    let ticket =
        app.block_on("the write lane's ticket", async move { call!(writes.ticket()).await });
    let job = CloudBackupCleanupJob {
        generation: ticket.expect("the write lane answers").generation,
        cloud: cove_device::cloud_storage::CloudStorage::global_explicit_client(),
        active_namespace_id: active.to_owned(),
        active_critical_key: master_key.critical_data_key(),
        sources: vec![CleanupSourceNamespace {
            namespace_id: source.to_owned(),
            expected_wallets: vec![CleanupExpectedWalletRecord {
                record_id: record.record_id.clone(),
                content_revision_hash: Some(record.revision.clone()),
            }],
        }],
    };

    let supervisor = app.manager().supervisor.clone();
    app.block_on("the cleanup to queue", async move {
        call!(supervisor.enqueue_cleanup_for_test(job)).await
    })
    .expect("queue the cleanup");
}

fn lane_queue(app: &TestApp) -> LaneQueue {
    let writes = app.writes();
    app.block_on("the write lane's queue", async move { call!(writes.harness_queue()).await })
        .expect("the write lane answers")
}

fn master_key_reads_of(app: &TestApp, namespace: &str) -> usize {
    app.cloud
        .calls()
        .iter()
        .filter(|(id, target)| {
            id.kind == CloudCall::DownloadMasterKey && target.as_deref() == Some(namespace)
        })
        .count()
}

/// A Reinitialize owns the namespaces from its start: a cleanup delete already running when it
/// starts finishes before the reinitialize reads what exists, so the reinitialize never recovers
/// the namespace that delete removes
///
/// Settling, not a timer, shows the reinitialize has gone as far as it can while the delete is
/// held: without the drain it lists both namespaces and reads the source's wrapper here
#[test]
fn a_reinitialize_waits_for_a_running_cleanup_delete_before_it_reads_the_namespaces() {
    let test = concat!(
        module_path!(),
        "::a_reinitialize_waits_for_a_running_cleanup_delete_before_it_reads_the_namespaces"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let seeded = FakeAccount::new(1, 0).seed_records(&app);
        let active = seeded.namespace.clone();
        let record = seeded.local[0].clone();
        seed_verifiable_master_key(&app, &active);
        let source = seed_merged_source(&app, &record);
        load_detail(&mut app);

        let delete = Hold::default();
        app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(delete.clone()));
        app.event("a merge cleanup proves its source and starts deleting it", |app| {
            schedule_merge_cleanup(app, &active, &source, &record);
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);
        app.block_on("the source delete to start", delete.entered());

        // with no local master key the reinitialize lists the namespaces and recovers one
        crate::manager::cloud_backup_manager::CloudBackupKeychain::global()
            .delete_master_key()
            .expect("delete the local master key");
        let source_reads = master_key_reads_of(&app, &source);
        app.event("Reinitialize starts while the delete runs", |app| {
            let supervisor = app.manager().supervisor.clone();
            app.block_on("the reinitialize to start", async move {
                call!(supervisor.start_recovery_operation(
                    crate::manager::cloud_backup_manager::RecoveryAction::ReinitializeBackup,
                    Some(tap(PermitIntent::Reinitialize))
                ))
                .await
            })
            .expect("start the reinitialize");
        })
        .then_cannot(&[N::Enable, N::TurnOff]);

        assert_eq!(
            master_key_reads_of(&app, &source),
            source_reads,
            "the reinitialize read a namespace the cleanup is deleting"
        );
        let claim = running_claim(&app).expect("the reinitialize runs");
        let lane = lane_queue(&app);
        assert!(
            matches!(lane.in_flight, Some(CommandClass::SourceCleanup { .. })),
            "the held delete runs: {lane:?}"
        );
        assert_eq!(
            lane.pending,
            vec![CommandClass::Operation(claim)],
            "the reinitialize waits behind the delete"
        );

        app.event("the source delete finishes", |_| delete.release()).then_cannot(&[N::Enable]);
        assert!(!app.cloud.inner.has_namespace(&source), "the cleanup deleted its source");
        assert_eq!(master_key_reads_of(&app, &source), source_reads);
        assert!(!app.runs_operation(), "the reinitialize finished");
        assert_eq!(
            keychain_namespace(),
            Some(active),
            "the reinitialize recovered the active backup"
        );
    });
}

/// The sections the published detail draws, empty when the backup is not configured
fn configured_sections(app: &TestApp) -> Vec<CloudBackupSection> {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::Configured(configured) => configured.sections,
        _ => Vec::new(),
    }
}

/// How many of this device's wallets the published detail lists
fn own_rows(app: &TestApp) -> usize {
    configured_sections(app)
        .iter()
        .map(|section| match section {
            CloudBackupSection::Wallets(wallets) => {
                wallets.up_to_date.len() + wallets.needs_sync.len()
            }
            _ => 0,
        })
        .sum()
}

/// The policy of each inventory listing after the first `skip` calls
fn listing_policies(app: &TestApp, skip: usize) -> Vec<Option<CloudAccessPolicy>> {
    app.cloud
        .calls()
        .into_iter()
        .skip(skip)
        .filter(|(id, _)| id.kind == CloudCall::ListWalletFilesSnapshot)
        .map(|(id, _)| app.cloud.policy_of(id))
        .collect()
}

/// A local reset owns the detail refresh queued before it: one still waiting on its rate limit
/// when the reset begins never lists the cloud, so it can't ask the user for provider consent.
/// When the reset fails and the backup resumes, the cycle opens again with nobody tapping
#[test]
fn a_local_reset_cancels_the_detail_refresh_waiting_on_its_rate_limit() {
    let test = concat!(
        module_path!(),
        "::a_local_reset_cancels_the_detail_refresh_waiting_on_its_rate_limit"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        // the first refresh runs now, so the second one waits on the rate limit
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_can(&[N::TurnOff, N::Verify]);
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_can(&[N::TurnOff, N::Verify]);
        let listings = app.cloud.count(CloudCall::ListWalletFilesSnapshot);
        app.cloud.revoke_authorization();

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::TurnOff, N::Enable]);

        assert_eq!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot),
            listings,
            "the waiting refresh listed the cloud after the reset began\n{}",
            app.describe()
        );
        assert_eq!(app.cloud.consent_sheets(), 0);

        // the resumed cycle lists on its own, without the taps from before the reset
        let before = app.cloud.calls().len();
        let recovery = abandon(&mut app, permit, "the wipe fails before the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let resumed = listing_policies(&app, before);
        assert!(
            !resumed.is_empty() && resumed.iter().all(|p| *p == Some(CloudAccessPolicy::Silent)),
            "the resumed backup did not list its inventory silently: {resumed:?}\n{}",
            app.describe()
        );
        assert_eq!(app.cloud.consent_sheets(), 0);

        let progress = configured_sections(&app).into_iter().find_map(|section| match section {
            CloudBackupSection::Progress(progress) => Some(progress),
            _ => None,
        });
        assert!(
            matches!(
                &progress,
                Some(CloudBackupProgressSection::InventoryIncomplete {
                    reason: CloudBackupInventoryIncompleteReason::AuthorizationRequired,
                    controls,
                }) if controls.contains(&CloudBackupInventoryControl::ReconnectDrive)
            ),
            "the resumed listing that needs consent did not offer Reconnect: {progress:?}\n{}",
            app.describe()
        );

        // Check Again is a new tap, so its listing may ask, and the rows load
        load_detail(&mut app);
        assert_eq!(app.cloud.consent_sheets(), 1);
        assert_eq!(own_rows(&app), 1, "{}", app.describe());
    });
}

/// Launch discovery that arrives while a local reset holds the runtime doesn't reopen the detail
/// refresh the reset suspended, so it lists nothing for the configuration being cleared; when the
/// reset fails and the backup resumes, the cycle opens and the inventory loads
#[test]
fn launch_discovery_during_a_local_reset_lists_nothing() {
    let test = concat!(module_path!(), "::launch_discovery_during_a_local_reset_lists_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        let listings = app.cloud.count(CloudCall::ListWalletFilesSnapshot);

        app.event("launch discovery during the reset", |app| {
            app.manager().start_background_inventory_discovery();
        })
        .then_cannot(&[N::TurnOff, N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::TurnOff, N::Enable]);

        assert_eq!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot),
            listings,
            "discovery reopened the detail refresh during the reset\n{}",
            app.describe()
        );

        let recovery = abandon(&mut app, permit, "the wipe fails before the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);

        assert!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot) > listings,
            "the resumed backup never listed its inventory\n{}",
            app.describe()
        );
        assert_eq!(own_rows(&app), 1, "{}", app.describe());
    });
}

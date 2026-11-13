//! An Enable or reinitialize that recovers an existing backup instead of creating one, and each
//! way its local promotion can fail
//!
//! A recovered backup has no saved-passkey pause to go back to: a failure either rolls the device
//! all the way back to before the recovery began, or leaves the journal for the journal recovery,
//! which the screen's Try Again runs. Each test checks the retry the screen then offers works
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use cove_cspp::master_key::MasterKey;
use cove_cspp::{Cspp, MasterKeyPromotionStatus};
use cove_device::keychain::Keychain;
use cove_device::passkey::DiscoveredPasskeyResult;

use super::fixtures::VERIFIABLE_PRF_KEY;
use super::{
    FakeAccount, NextAction as N, PhaseEnd, TestApp, keychain_namespace, load_detail, persisted,
    run_case, seed_verifiable_master_key, verifiable_wrapper,
};
use crate::database::Database;
use crate::manager::cloud_backup_manager::keychain::CSPP_PENDING_ENABLE_JOURNAL_KEY;
use crate::manager::cloud_backup_manager::pending_enable::{
    PendingEnableJournal, PendingEnableJournalPhase, PendingEnableNamespaceOwnership,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupOutcome, CloudBackupOutcomeTarget, CloudBackupScreenAction as A,
    CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupKeychain, CloudBackupManagerAction,
    PendingEnableLocalMetadataSnapshot,
};

/// The keychain item holding CSPP's active master-key encryption key, which every promotion and
/// rollback of the active key rewrites
const ACTIVE_ENCRYPTION_KEY_ITEM: &str = "cspp::v1::master_key_encryption_key_and_nonce";

/// The device before the recovery began, which a full rollback puts back
struct Prior {
    namespace: String,
    metadata: PendingEnableLocalMetadataSnapshot,
    /// The key the backup's cloud copy holds, which a finished recovery makes active
    recovered_key: MasterKey,
}

fn active_master_key() -> Option<MasterKey> {
    Cspp::new(Keychain::global().clone()).load_master_key_from_store().expect("read active key")
}

fn promotion_status() -> MasterKeyPromotionStatus {
    Cspp::new(Keychain::global().clone()).master_key_promotion_status().expect("read promotion")
}

fn metadata() -> PendingEnableLocalMetadataSnapshot {
    CloudBackupKeychain::global().snapshot_passkey_metadata().expect("read passkey metadata")
}

fn journal() -> Option<PendingEnableJournal> {
    CloudBackupKeychain::global().load_pending_enable_journal().expect("read the journal")
}

fn body(app: &TestApp) -> CloudBackupScreenBody {
    app.manager().published_screen().body
}

/// A configured backup whose local key is gone, so its reinitialize recovers the backup from the
/// cloud copy the user's passkey opens rather than creating another
fn configured_backup_without_its_key(app: &mut TestApp) -> Prior {
    let namespace = FakeAccount::new(1, 0).seed(app);
    seed_verifiable_master_key(app, &namespace);
    load_detail(app);

    let recovered_key = active_master_key().expect("the seeded key");
    CloudBackupKeychain::global().delete_master_key().expect("delete the local master key");

    Prior { namespace, metadata: metadata(), recovered_key }
}

/// Fails the finalization's lifecycle write, the one that saves the recovery's durable
/// completion, and runs `also` when it does, so a fault can follow it in the same step
fn fail_the_durable_completion(also: impl Fn() + Send + 'static) {
    Database::global().cloud_backup_state.fail_next_write_where(move |state| {
        let saves_completion = state.pending_verification_completion().is_some();
        if saves_completion {
            also();
        }

        saves_completion
    });
}

/// Nothing of the recovery is left: the device is as it was before it began
fn assert_rolled_all_the_way_back(app: &TestApp, prior: &Prior) {
    assert!(active_master_key().is_none(), "{}", app.describe());
    assert_eq!(metadata(), prior.metadata);
    assert_eq!(promotion_status(), MasterKeyPromotionStatus::None);
    assert_eq!(journal(), None, "{}", app.describe());
    assert_eq!(app.saved_passkey_confirmation(), None, "a recovered backup has no pause");
}

/// The reinitialize failed, drawn next to the control the user taps to run it again
fn assert_reinitialize_failed(app: &TestApp) {
    let outcome = app
        .manager()
        .published_screen()
        .last_outcome
        .unwrap_or_else(|| panic!("the reinitialize showed no failure\n{}", app.describe()));
    assert_eq!(outcome.target, CloudBackupOutcomeTarget::Action(A::ReinitializeBackup));
    assert!(
        matches!(outcome.outcome, CloudBackupOutcome::Failed { .. }),
        "{outcome:?}\n{}",
        app.describe()
    );
}

/// The recovered key is active under its backup's metadata, and nothing of the recovery is left
fn assert_recovered(app: &TestApp, prior: &Prior) {
    let active = active_master_key().expect("the recovered key is active");
    assert_eq!(active.as_bytes(), prior.recovered_key.as_bytes(), "{}", app.describe());
    assert_eq!(keychain_namespace().as_deref(), Some(prior.namespace.as_str()));
    assert_eq!(credential_id(), Some(vec![1, 2, 3]));
    assert_eq!(promotion_status(), MasterKeyPromotionStatus::None);
    assert_eq!(journal(), None, "{}", app.describe());
    assert!(!app.runs_operation(), "{}", app.describe());
    assert!(matches!(body(app), CloudBackupScreenBody::Configured(_)), "{}", app.describe());
}

fn credential_id() -> Option<Vec<u8>> {
    CloudBackupKeychain::global().load_credential_id().expect("read the credential")
}

/// Model review sequence: a reinitialize recovering its backup fails to save the durable
/// completion, and the promotion rolls back
///
/// A recovered backup has no pause to keep its stage for, so the rollback goes all the way back
/// to before the recovery. The reinitialize the screen offers again recovers the backup from the
/// start instead of meeting a stage it can't use
#[test]
fn a_recovered_finalization_that_fails_rolls_back_for_the_offered_retry() {
    let test = concat!(
        module_path!(),
        "::a_recovered_finalization_that_fails_rolls_back_for_the_offered_retry"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let prior = configured_backup_without_its_key(&mut app);

        fail_the_durable_completion(|| {});
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup).then_cannot(&[N::RetryLoad]);
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_reinitialize_failed(&app);
        assert_rolled_all_the_way_back(&app, &prior);
        assert_eq!(persisted().pending_verification_completion(), None);

        the_passkey_opens_the_backup(&app);
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup).then_cannot(&[N::RetryLoad]);
        assert_recovered(&app, &prior);
        assert_eq!(metadata(), prior.metadata);
        assert_eq!(app.passkey.inner.create_count(), 0, "a recovery registers no passkey");
    });
}

/// A backup only the cloud holds, which an Enable recovers with the passkey that opens it
fn a_backup_only_the_cloud_holds(app: &TestApp) -> Prior {
    let recovered_key = MasterKey::generate();
    let namespace = recovered_key.namespace_id();
    app.cloud.inner.set_master_key_backup(namespace.clone(), verifiable_wrapper(&recovered_key));
    app.cloud.persist();
    the_passkey_opens_the_backup(app);

    Prior { namespace, metadata: metadata(), recovered_key }
}

/// The platform finds the passkey that opens the backup, for the next recovery; each discovery
/// takes the answer it finds
fn the_passkey_opens_the_backup(app: &TestApp) {
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: VERIFIABLE_PRF_KEY.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    app.passkey.inner.set_authenticate_result(Ok(VERIFIABLE_PRF_KEY.to_vec()));
}

/// An Enable recovering a backup fails before its commit, and its rollback gets as far as the
/// journal, whose delete the keychain refuses
///
/// The key and metadata are already back, but the journal stays, so the work stays the
/// recovery's: the screen offers Try Again, which finishes the cleanup, after which Enable
/// recovers the backup again. Without that, the leftover journal would refuse every new
/// recovery
#[test]
fn a_recovered_enable_whose_rollback_keeps_its_journal_cleans_up_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_recovered_enable_whose_rollback_keeps_its_journal_cleans_up_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let prior = a_backup_only_the_cloud_holds(&app);

        // the promotion's active-key write fails and rolls back; the full rollback after it
        // can't delete the journal
        app.keychain.fail_saves_of(ACTIVE_ENCRYPTION_KEY_ITEM);
        app.keychain.fail_next_delete_of(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_can(&[N::RetryLoad])
        .then_cannot(&[N::Enable, N::TurnOff]);
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_eq!(body(&app), CloudBackupScreenBody::LoadFailed, "{}", app.describe());
        let journal = journal().expect("the journal the rollback couldn't delete");
        assert!(
            matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)),
            "{journal:?}"
        );
        assert_eq!(
            journal.namespace_ownership(),
            PendingEnableNamespaceOwnership::RecoveredExisting
        );
        assert!(active_master_key().is_none());
        assert_eq!(metadata(), prior.metadata);

        app.keychain.clear_save_failures();
        app.dispatch(CloudBackupManagerAction::RetryLoad)
            .then_can(&[N::Enable])
            .then_cannot(&[N::RetryLoad]);
        assert_rolled_all_the_way_back(&app, &prior);

        the_passkey_opens_the_backup(&app);
        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable, N::RetryLoad]);
        assert_recovered(&app, &prior);
    });
}

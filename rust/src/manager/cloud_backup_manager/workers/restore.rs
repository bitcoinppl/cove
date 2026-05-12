use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, Produces, WeakAddr, call};
use cove_device::keychain::Keychain;
use cove_util::ResultExt as _;
use cove_util::{GenerationClaim, GenerationToken, GenerationTracker};
use tracing::{error, info, warn};

use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableOutcome, CloudBackupError, CloudBackupRestoreOutcome, CloudBackupStatus,
    RustCloudBackupManager,
};

use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;

#[derive(Debug)]
pub(crate) struct CloudBackupRestoreWorker {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    restore_operations: GenerationTracker,
}

#[async_trait::async_trait]
impl Actor for CloudBackupRestoreWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupRestoreWorker {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self { addr: WeakAddr::default(), manager, restore_operations: GenerationTracker::new() }
    }

    fn manager(&self) -> Option<Arc<RustCloudBackupManager>> {
        self.manager.upgrade()
    }

    fn addr(&self) -> Addr<Self> {
        self.addr.upgrade()
    }

    fn restore_generation_is_current(&self, generation: GenerationToken) -> bool {
        self.restore_operations.is_current(generation)
    }

    pub(crate) async fn ensure_restore_current(
        &mut self,
        generation: GenerationToken,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if self.restore_generation_is_current(generation) {
            Produces::ok(Ok(()))
        } else {
            Produces::ok(Err(CloudBackupError::Cancelled))
        }
    }

    pub(crate) async fn apply_restore_status(
        &mut self,
        generation: GenerationToken,
        status: CloudBackupStatus,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        manager.reconcile_runtime_status(status);
        Produces::ok(Ok(()))
    }

    pub(crate) async fn apply_restore_outcome(
        &mut self,
        generation: GenerationToken,
        outcome: CloudBackupRestoreOutcome,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        manager.apply_restore_outcome(outcome);
        Produces::ok(Ok(()))
    }

    pub(crate) async fn persist_restore_cloud_backup_state(
        &mut self,
        generation: GenerationToken,
        state: PersistedCloudBackupState,
        context: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        };
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = Database::global()
            .cloud_backup_state
            .set(&state)
            .map_err(|error| CloudBackupError::Internal(format!("{context}: {error}")));
        if result.is_ok() {
            manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(&state));
            manager.refresh_persisted_flags();
        }

        Produces::ok(result)
    }

    pub(crate) async fn save_restore_keychain_state(
        &mut self,
        generation: GenerationToken,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if !self.restore_generation_is_current(generation) {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = save_restore_keychain_entries(master_key, passkey, namespace_id);
        Produces::ok(result)
    }

    pub(crate) async fn start_restore_from_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let status = manager.state.read().status().clone();
        if matches!(status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring) {
            warn!("restore_from_cloud_backup called while {status:?}, ignoring");
            return Produces::ok(());
        }

        let operation = RestoreOperation::new(self.restore_operations.clone(), self.addr());
        cove_tokio::task::spawn(async move {
            info!("restore_from_cloud_backup: task started");
            match manager.do_restore_from_cloud_backup(&operation).await {
                Ok(()) => {}
                Err(CloudBackupError::Cancelled) => {
                    info!("restore_from_cloud_backup: task cancelled");
                }
                Err(error) => {
                    error!("restore_from_cloud_backup failed: {error}");
                    manager.finish_background_operation_error(&error);
                }
            }
        });

        Produces::ok(())
    }

    pub(crate) async fn cancel_restore(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let status = manager.state.read().status().clone();
        if !matches!(status, CloudBackupStatus::Restoring) {
            return Produces::ok(());
        }

        self.restore_operations.invalidate();
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ReportCleared);
        manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));
        info!("restore_from_cloud_backup: cancelled active restore");
        Produces::ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RestoreOperation {
    active_restore_generation: GenerationClaim,
    restore_actor: Addr<CloudBackupRestoreWorker>,
}

impl RestoreOperation {
    fn new(tracker: GenerationTracker, restore_actor: Addr<CloudBackupRestoreWorker>) -> Self {
        Self { active_restore_generation: tracker.claim(), restore_actor }
    }

    fn generation(&self) -> GenerationToken {
        self.active_restore_generation.token()
    }

    pub(crate) async fn ensure_current(&self) -> Result<(), CloudBackupError> {
        call!(self.restore_actor.ensure_restore_current(self.generation()))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn apply_status(
        &self,
        status: CloudBackupStatus,
    ) -> Result<(), CloudBackupError> {
        call!(self.restore_actor.apply_restore_status(self.generation(), status))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn apply_outcome(
        &self,
        outcome: CloudBackupRestoreOutcome,
    ) -> Result<(), CloudBackupError> {
        call!(self.restore_actor.apply_restore_outcome(self.generation(), outcome))
            .await
            .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn persist_cloud_backup_state(
        &self,
        state: PersistedCloudBackupState,
        context: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.restore_actor.persist_restore_cloud_backup_state(
            self.generation(),
            state,
            context
        ))
        .await
        .map_err(|_| CloudBackupError::Cancelled)?
    }

    pub(crate) async fn save_keychain_state(
        &self,
        master_key: cove_cspp::master_key::MasterKey,
        passkey: Option<RestoredPasskeyMaterial>,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        call!(self.restore_actor.save_restore_keychain_state(
            self.generation(),
            master_key,
            passkey,
            namespace_id
        ))
        .await
        .map_err(|_| CloudBackupError::Cancelled)?
    }
}

#[derive(Debug)]
pub(crate) struct RestoredPasskeyMaterial {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) prf_salt: [u8; 32],
}

fn save_restore_keychain_entries(
    master_key: cove_cspp::master_key::MasterKey,
    passkey: Option<RestoredPasskeyMaterial>,
    namespace_id: String,
) -> Result<(), CloudBackupError> {
    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
    let cspp = cove_cspp::Cspp::new(keychain.clone());

    if let Some(passkey) = passkey {
        cloud_keychain
            .save_passkey_and_namespace(&passkey.credential_id, passkey.prf_salt, &namespace_id)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?
    } else {
        cloud_keychain
            .save_namespace_id(&namespace_id)
            .map_err_prefix("save namespace_id", CloudBackupError::Internal)?
    };

    if let Err(error) = cspp.save_master_key(&master_key) {
        if let Err(rollback) = cloud_keychain.clear_local_state() {
            return Err(CloudBackupError::Internal(format!(
                "save master key: {error}; rollback failed: {rollback}"
            )));
        }

        return Err(CloudBackupError::Internal(format!("save master key: {error}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::{test_globals, test_lock};

    impl CloudBackupRestoreWorker {
        pub(crate) async fn new_restore_operation(&mut self) -> ActorResult<RestoreOperation> {
            Produces::ok(RestoreOperation::new(self.restore_operations.clone(), self.addr()))
        }

        pub(crate) async fn invalidate_restore_operation(&mut self) -> ActorResult<()> {
            self.restore_operations.invalidate();
            Produces::ok(())
        }
    }

    #[test]
    fn restore_keychain_save_rolls_back_metadata_when_master_key_save_fails() {
        let _guard = test_lock().lock();
        let globals = test_globals();
        globals.reset();
        globals.keychain.fail_save_at(4);

        let result = save_restore_keychain_entries(
            cove_cspp::master_key::MasterKey::generate(),
            Some(RestoredPasskeyMaterial { credential_id: vec![1, 2, 3], prf_salt: [4; 32] }),
            "namespace-id".into(),
        );

        assert!(result.is_err());
        assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
        assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_none());
    }
}

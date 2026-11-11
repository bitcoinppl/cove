//! Typed results that producers off the actor hand to the lifecycle owner
//!
//! Each message carries the producer's result and, where the work belonged to an operation, its
//! claim. The supervisor applies the durable write with its [`LifecycleWriter`] and reconciles,
//! so no producer persists the lifecycle record itself

use super::*;
use crate::manager::cloud_backup_manager::actors::write::CloudBackupUploadedWalletsStateMode;
use crate::manager::cloud_backup_manager::verify::{
    IntegrityDowngrade, PendingVerificationSettlement,
};

impl CloudBackupSupervisor {
    /// The cloud lists `wallet_count` wallet backups for the active namespace
    pub async fn record_cloud_wallet_count(&mut self, wallet_count: u32) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if let Err(error) = manager.reconcile_committed(
            "persist cloud backup wallet count",
            self.lifecycle.record_wallet_count(wallet_count),
        ) {
            warn!("Failed to persist cloud backup wallet count: {error}");
        }

        Produces::ok(())
    }

    /// The write lane uploaded wallets for `origin` and the cloud now lists `wallet_count`
    ///
    /// The write lane waits for this reply before it marks the wallets pending confirmation. A
    /// result whose operation already ended is discarded, and the write lane reports it cancelled
    pub async fn record_uploaded_wallets(
        &mut self,
        origin: Option<CloudBackupExclusiveOperationClaim>,
        wallet_count: u32,
        state_mode: CloudBackupUploadedWalletsStateMode,
    ) -> ActorResult<Result<(), CloudBackupError>> {
        if let Some(origin) = origin
            && !self.active_operation.accepts_completion(origin, "uploaded wallets")
        {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = match state_mode {
            CloudBackupUploadedWalletsStateMode::PreserveVerification => {
                self.lifecycle.persist_enabled(wallet_count)
            }
            CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                completion,
            ) => self.lifecycle.persist_enabled_reset_verification_with_pending_completion(
                wallet_count,
                completion,
            ),
        };

        Produces::ok(result)
    }

    /// The local wallet set changed, so the recorded verification no longer covers it
    ///
    /// A verification that is already running read the old wallet set, so its evidence no
    /// longer matches the current generation and can't undo this reset
    pub async fn require_verification_after_wallet_change(
        &mut self,
        requested_at: u64,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.wallet_set = self.wallet_set.next();

        if let Err(error) = manager.reconcile_committed(
            "mark cloud backup unverified after wallet change",
            self.lifecycle.require_verification_after_wallet_change(requested_at),
        ) {
            error!("Failed to mark cloud backup unverified after wallet change: {error}");
        }

        Produces::ok(())
    }

    /// The integrity check found the backup weaker than recorded
    pub async fn apply_integrity_downgrade(
        &mut self,
        downgrade: IntegrityDowngrade,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if let Err(error) = manager.reconcile_committed(
            "persist backup integrity state",
            self.lifecycle.apply_integrity_downgrade(downgrade),
        ) {
            error!("Failed to persist backup integrity state: {error}");
        }

        Produces::ok(())
    }

    /// A manual verification is starting, so a pending upload confirmation no longer settles it
    pub async fn discard_pending_verification_completion(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let cleared = match manager.reconcile_committed(
            "clear pending verification completion",
            self.lifecycle.discard_pending_verification_completion(),
        ) {
            Ok(cleared) => cleared,
            Err(error) => {
                error!("Failed to clear pending verification completion: {error}");
                return Produces::ok(());
            }
        };

        self.detail_workflow.clear_pending_completion();
        if cleared {
            manager.refresh_pending_upload_verification_state();
        }

        Produces::ok(())
    }

    /// An upload confirmation finished
    ///
    /// The pending upload worker waits for this reply, so its next read of the pending state
    /// already sees the settle
    pub async fn settle_pending_verification(
        &mut self,
        settlement: PendingVerificationSettlement,
    ) -> ActorResult<()> {
        if let Some(manager) = self.manager() {
            self.apply_pending_verification_settlement(&manager, settlement);
        }

        Produces::ok(())
    }

    /// Settles an upload confirmation, accepted only while it is still the stored marker
    pub(crate) fn apply_pending_verification_settlement(
        &mut self,
        manager: &RustCloudBackupManager,
        settlement: PendingVerificationSettlement,
    ) {
        let PendingVerificationSettlement { completion, result } = settlement;
        let verified_at = cove_util::time::unix_timestamp_secs_or_zero();
        let settled = manager.reconcile_committed(
            "settle pending verification completion",
            self.lifecycle
                .settle_pending_verification_completion(&completion, Some((&result, verified_at))),
        );

        match settled {
            Ok(true) => {
                self.detail_workflow.clear_pending_completion();
                manager.apply_settled_verification_result(result);
            }
            Ok(false) => debug!("Dropped an upload confirmation that is no longer pending"),
            Err(error) => error!("Failed to settle pending verification completion: {error}"),
        }
    }

    /// A sync found the Restore All marker for `namespace_id` while this device uses another
    /// namespace
    pub async fn clear_stale_restore_all_marker(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        // the sync that found the marker already projected the reset
        if let Err(error) = self.lifecycle.clear_stale_restore_all_marker(&namespace_id) {
            error!("Failed to clear stale Restore All marker: {error}");
        }

        Produces::ok(())
    }

    /// Records the running verification's evidence, unless the wallet set changed since it read
    /// the wallets
    pub(crate) fn record_verification_evidence(
        &self,
        manager: &RustCloudBackupManager,
        result: &DeepVerificationResult,
    ) {
        if self.verification_basis != self.wallet_set {
            debug!("Cloud Backup discarded verification evidence for an older wallet set");
            return;
        }

        manager.persist_verification_result(&self.lifecycle, result);
    }
}

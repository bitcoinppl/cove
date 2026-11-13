//! Typed results that producers off the actor hand to the lifecycle owner
//!
//! Each message carries the producer's result and, where the work belonged to an operation, its
//! claim. The supervisor applies the durable write with its [`LifecycleWriter`] and reconciles,
//! so no producer persists the lifecycle record itself

use super::*;
use crate::manager::cloud_backup_manager::actors::write::CloudBackupUploadedWalletsStateMode;
use crate::manager::cloud_backup_manager::pending::RemoteRevisionEvidence;
use crate::manager::cloud_backup_manager::verify::{
    IntegrityDowngrade, PendingVerificationSettlement,
};

impl CloudBackupSupervisor {
    /// The cloud lists `wallet_count` wallet backups for the active namespace
    pub async fn record_cloud_wallet_count(&mut self, wallet_count: u32) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if let Err(error) = self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "persist cloud backup wallet count",
                lifecycle.record_wallet_count(wallet_count),
            )
        }) {
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
            && !self.runtime.accepts_completion(origin, "uploaded wallets")
        {
            return Produces::ok(Err(CloudBackupError::Cancelled));
        }

        let result = self.runtime.commit(|lifecycle| match state_mode {
            CloudBackupUploadedWalletsStateMode::PreserveVerification => {
                lifecycle.persist_enabled(wallet_count)
            }
            CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                completion,
            ) => lifecycle.persist_enabled_reset_verification_with_pending_completion(
                wallet_count,
                completion,
            ),
        });

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

        if let Err(error) = self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "mark cloud backup unverified after wallet change",
                lifecycle.require_verification_after_wallet_change(requested_at),
            )
        }) {
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

        if let Err(error) = self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "persist backup integrity state",
                lifecycle.apply_integrity_downgrade(downgrade),
            )
        }) {
            error!("Failed to persist backup integrity state: {error}");
        }

        Produces::ok(())
    }

    /// A manual verification is starting, so a pending upload confirmation no longer settles it
    pub async fn discard_pending_verification_completion(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let cleared = match self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "clear pending verification completion",
                lifecycle.discard_pending_verification_completion(),
            )
        }) {
            Ok(cleared) => cleared,
            Err(error) => {
                error!("Failed to clear pending verification completion: {error}");
                return Produces::ok(());
            }
        };

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

    /// The configuration and local wallet set an upload confirmation reads its evidence under
    ///
    /// The pending upload worker asks before it reads, so a wallet change or a new configuration
    /// after this answer makes its settlement stale
    pub async fn evidence_basis_for_confirmation(&mut self) -> ActorResult<EvidenceBasis> {
        Produces::ok(self.evidence_basis())
    }

    /// Settles an upload confirmation, accepted only while it is still the stored marker and the
    /// configuration and wallet set it read are still current
    pub(crate) fn apply_pending_verification_settlement(
        &mut self,
        manager: &RustCloudBackupManager,
        settlement: PendingVerificationSettlement,
    ) {
        let PendingVerificationSettlement { basis, read_at, completion, result } = settlement;
        if basis != self.evidence_basis() {
            debug!("Dropped an upload confirmation read before a wallet or configuration change");
            return;
        }

        let verified_at = cove_util::time::unix_timestamp_secs_or_zero();
        let settled = self.runtime.commit(|lifecycle| {
            manager.reconcile_committed(
                "settle pending verification completion",
                lifecycle.settle_pending_verification_completion(
                    &completion,
                    Some((&result, verified_at)),
                ),
            )
        });

        match settled {
            Ok(true) => manager.apply_settled_verification_result(result, read_at),
            Ok(false) => debug!("Dropped an upload confirmation that is no longer pending"),
            Err(error) => error!("Failed to settle pending verification completion: {error}"),
        }
    }

    /// Confirms the pending uploads that remote evidence proved
    ///
    /// The evidence names the namespace it was read from and the configuration its producer
    /// started under. It confirms only rows in that namespace, and only while that configuration
    /// is current, so a read of one backup held across a reset and an Enable of another, or across
    /// a Drive account switch, confirms nothing in the backup that replaced it
    pub async fn confirm_remote_revisions(
        &mut self,
        evidence: RemoteRevisionEvidence,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if evidence.config() != self.runtime.generation() {
            debug!("Dropped remote revision evidence read under an older configuration");
            return Produces::ok(());
        }

        if evidence.confirm_pending_uploads() {
            manager.refresh_pending_upload_verification_state();
        }

        Produces::ok(())
    }

    /// A sync found the Restore All marker for `namespace_id` while this device uses another
    /// namespace
    pub async fn clear_stale_restore_all_marker(
        &mut self,
        namespace_id: String,
    ) -> ActorResult<()> {
        // the sync that found the marker already projected the reset
        if let Err(error) =
            self.runtime.commit(|lifecycle| lifecycle.clear_stale_restore_all_marker(&namespace_id))
        {
            error!("Failed to clear stale Restore All marker: {error}");
        }

        Produces::ok(())
    }

    /// Records a verification run's evidence, unless the configuration or wallet set changed
    /// since the run read the wallets at `basis`
    pub(crate) fn record_verification_evidence(
        &mut self,
        manager: &RustCloudBackupManager,
        basis: EvidenceBasis,
        result: &DeepVerificationResult,
    ) {
        if basis != self.evidence_basis() {
            debug!("Cloud Backup discarded verification evidence for an older basis {basis:?}");
            return;
        }

        self.runtime.commit(|lifecycle| manager.persist_verification_result(lifecycle, result));
    }

    /// Records a finished verification run's evidence and shows its result, or, when the
    /// configuration or wallet set changed since the run read the wallets at `basis`, ends the
    /// run without either
    ///
    /// The wallet change already recorded that the backup needs verifying, so showing the older
    /// result would contradict it
    pub(crate) fn settle_verification_evidence(
        &mut self,
        manager: &RustCloudBackupManager,
        basis: EvidenceBasis,
        read_at: InventoryReadAt,
        result: DeepVerificationResult,
    ) {
        if basis != self.evidence_basis() {
            debug!("Cloud Backup discarded verification result for an older basis {basis:?}");
            manager.apply_verification_state(VerificationState::Idle);
            return;
        }

        self.runtime.commit(|lifecycle| manager.persist_verification_result(lifecycle, &result));
        manager.handle_deep_verification_result(result, read_at);
    }
}

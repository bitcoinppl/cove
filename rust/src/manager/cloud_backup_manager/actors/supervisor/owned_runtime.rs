//! The supervisor's runtime with its write policy published on every change
//!
//! [`OwnedRuntime`] is the only way to change the stored [`Runtime`]: it derefs for reads and has
//! no `DerefMut`, so every mutation goes through a method here, which rebases an idle body on the
//! owner's record and pushes the derived [`WritePolicy`] to the write supervisor when it changed.
//! The compiler, not each call site, guarantees the write lane sees every runtime change

use std::ops::Deref;
use std::sync::Weak;

use act_zero::{Addr, send};
use tokio::sync::oneshot;
use tracing::debug;

use super::runtime::{Effect, IdleFacts, ResetStart, RestoreRun, Runtime, Unstartable};
use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::manager::cloud_backup_manager::actors::write::{
    CloudBackupWriteSupervisor, ConfigGeneration, VersionedPolicy, WritePolicy,
};
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::owner::lifecycle::CommittedDisable;

/// The stored runtime and the write policy last published for it
#[derive(Debug)]
pub(crate) struct OwnedRuntime {
    runtime: Runtime,
    write: Addr<CloudBackupWriteSupervisor>,
    manager: Weak<RustCloudBackupManager>,
    /// The configuration generation commands with no operation are matched against
    generation: ConfigGeneration,
    /// The running reinitialize at the last publish, which replaces the configuration when it ends
    replacing: Option<CloudBackupExclusiveOperationClaim>,
    published: VersionedPolicy,
}

impl Deref for OwnedRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Runtime {
        &self.runtime
    }
}

impl OwnedRuntime {
    pub(crate) fn new(
        runtime: Runtime,
        write: Addr<CloudBackupWriteSupervisor>,
        manager: Weak<RustCloudBackupManager>,
    ) -> Self {
        let generation = ConfigGeneration::next();
        let mut owned = Self {
            published: VersionedPolicy { version: 0, policy: WritePolicy::starting() },
            replacing: runtime.replacing_configuration(),
            runtime,
            write,
            manager,
            generation,
        };

        owned.publish_policy();
        owned
    }

    /// The configuration generation of the current body
    pub(crate) fn generation(&self) -> ConfigGeneration {
        self.generation
    }

    /// The policy last pushed to the write supervisor, with its version
    pub(crate) fn published_policy(&self) -> &VersionedPolicy {
        &self.published
    }

    fn mutate<T>(&mut self, change: impl FnOnce(&mut Runtime) -> T) -> T {
        let result = change(&mut self.runtime);
        self.publish();
        result
    }

    /// Rebases an idle body on the record, then publishes the policy if it changed
    fn publish(&mut self) {
        let record = RustCloudBackupManager::load_persisted_state();
        let pending_enable_recovery = self.shows_pending_enable_recovery();
        self.runtime.rebase_idle(IdleFacts { record: &record, pending_enable_recovery });
        self.publish_policy();
    }

    /// The supervisor projected an interrupted Enable it found at restart
    fn shows_pending_enable_recovery(&self) -> bool {
        self.manager
            .upgrade()
            .is_some_and(|manager| manager.state.read().shows_pending_enable_recovery())
    }

    fn publish_policy(&mut self) {
        let replacing = self.runtime.replacing_configuration();
        let mut policy = self.runtime.write_policy(self.generation);
        let body_changed = std::mem::discriminant(&policy.body)
            != std::mem::discriminant(&self.published.policy.body);
        let replaced = self.replacing.is_some() && replacing != self.replacing;
        self.replacing = replacing;

        // a new body kind, or a reinitialize that ended, starts a new configuration
        if self.published.version > 0 && (body_changed || replaced) {
            self.generation = ConfigGeneration::next();
            policy.generation = self.generation;
        }

        if self.published.version > 0 && policy == self.published.policy {
            return;
        }

        self.published = VersionedPolicy { version: self.published.version + 1, policy };
        debug!("Cloud Backup published write policy {:?}", self.published);
        send!(self.write.apply_policy(self.published.clone()));
    }

    /// Republishes after a lifecycle write moved the record under an idle body
    pub(crate) fn rebase_idle(&mut self) {
        self.publish();
    }

    pub(crate) fn clear(&mut self) {
        self.mutate(Runtime::clear);
    }

    pub(crate) fn start_standard(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_standard(claim))
    }

    pub(crate) fn start_restore(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_restore(run))
    }

    pub(crate) fn start_restore_all(&mut self, run: RestoreRun) -> Result<(), Unstartable> {
        self.mutate(|runtime| runtime.start_restore_all(run))
    }

    pub(crate) fn commit_turn_off(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        committed: CommittedDisable,
    ) {
        self.mutate(|runtime| runtime.commit_turn_off(claim, committed));
    }

    pub(crate) fn hand_off(
        &mut self,
        from: CloudBackupExclusiveOperationClaim,
        to: CloudBackupExclusiveOperationClaim,
    ) -> Result<(), Unstartable> {
        let record = RustCloudBackupManager::load_persisted_state();
        let pending_enable_recovery = self.shows_pending_enable_recovery();
        let facts = IdleFacts { record: &record, pending_enable_recovery };
        self.mutate(|runtime| runtime.hand_off(from, to, facts))
    }

    pub(crate) fn begin_effect(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        effect: Effect,
    ) -> bool {
        self.mutate(|runtime| runtime.begin_effect(claim, effect))
    }

    pub(crate) fn accepts_completion(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: &'static str,
    ) -> bool {
        self.mutate(|runtime| runtime.accepts_completion(claim, completion))
    }

    pub(crate) fn begin_reset(&mut self, ready: oneshot::Sender<()>) -> ResetStart {
        self.mutate(|runtime| runtime.begin_reset(ready))
    }

    pub(crate) fn finish_reset(&mut self, after: Runtime) {
        self.mutate(|runtime| runtime.finish_reset(after));
    }

    /// Effects don't change the policy, so waiting on them publishes nothing
    #[cfg(test)]
    pub(crate) fn when_effects_settle(&mut self, settled: oneshot::Sender<()>) {
        self.runtime.when_effects_settle(settled);
    }

    /// Restore All's run flags don't change the policy
    pub(crate) fn restore_all_mut(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> Option<&mut RestoreRun> {
        self.runtime.restore_all_mut(claim)
    }

    #[cfg(test)]
    pub(crate) fn take_claim(&mut self) -> Option<CloudBackupExclusiveOperationClaim> {
        self.mutate(Runtime::take_claim)
    }
}

/// Tests compare the running claim directly, as they do for the bare runtime
#[cfg(test)]
impl PartialEq<Option<CloudBackupExclusiveOperationClaim>> for OwnedRuntime {
    fn eq(&self, other: &Option<CloudBackupExclusiveOperationClaim>) -> bool {
        self.runtime == *other
    }
}

use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn begin_enable_operation(
        &mut self,
        context: CloudBackupEnableContext,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) =
            self.begin_permitted_operation(&manager, CloudBackupExclusiveOperation::Enable, ticket)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        let authority = self.runtime.authority(claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_enable(context, authority).await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }

    pub(crate) fn begin_enable_force_new_operation(
        &mut self,
        context: CloudBackupEnableContext,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::EnableForceNew,
            ticket,
        ) else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        manager.apply_enable_state(CloudBackupEnableState::CreatingPasskey);
        self.schedule_enable_passkey_registration(
            manager,
            claim,
            context,
            EnablePasskeyRegistrationFlow::ForceNew,
        );
    }

    pub(crate) fn begin_enable_no_discovery_operation(
        &mut self,
        context: CloudBackupEnableContext,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::EnableNoDiscovery,
            ticket,
        ) else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_no_discovery_enable(context).await;
            send!(addr.complete_no_discovery_enable_preparation(claim, result));
        });
    }

    pub(crate) fn start_reinitialize_backup_operation(&mut self, ticket: Option<PermitTicket>) {
        let Some(manager) = self.manager() else { return };
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::ReinitializeBackup,
            ticket,
        ) else {
            return;
        };

        // a reinitialize owns the namespaces from its start: its policy pauses background writes
        // and source cleanup, and this barrier waits out the write already in flight, so a cleanup
        // delete that started before the reinitialize can't remove a namespace it may recover. The
        // supervisor never waits on the write supervisor: the reply is
        // `complete_reinitialize_write_drain`
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr().send_fut_with(move |addr| async move {
            let drained = writes.drain().await;
            send!(addr.complete_reinitialize_write_drain(claim, drained));
        });
    }

    /// The writes queued before a reinitialize finished, so it can read the namespaces
    pub async fn complete_reinitialize_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        drained: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else { return Produces::ok(()) };
        if let Err(error) = drained {
            self.fail_enable_operation(&manager, claim, error);
            return Produces::ok(());
        }

        self.start_reinitialize_backup_operation_with_claim(manager, claim);
        Produces::ok(())
    }

    pub(crate) fn start_reinitialize_backup_operation_with_claim(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        let addr = self.addr();

        manager.apply_recovery_state(RecoveryState::Recovering(RecoveryAction::ReinitializeBackup));

        match self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        let authority = self.runtime.authority(claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager
                .prepare_enable(CloudBackupEnableContext::settings_manual(), authority)
                .await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }
}

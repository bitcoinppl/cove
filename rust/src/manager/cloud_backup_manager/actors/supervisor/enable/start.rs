use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn start_enable_operation(&mut self, context: CloudBackupEnableContext) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_enable(context).await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }

    pub(crate) fn start_enable_force_new_operation(&mut self, context: CloudBackupEnableContext) {
        let Some(manager) = self.manager() else { return };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOrForceNewConfirmation,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
        self.schedule_enable_passkey_registration(
            manager,
            claim,
            context,
            EnablePasskeyRegistrationFlow::ForceNew,
        );
    }

    pub(crate) fn start_enable_no_discovery_operation(
        &mut self,
        context: CloudBackupEnableContext,
    ) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableNoDiscovery)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }
        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_no_discovery_enable(context).await;
            send!(addr.complete_no_discovery_enable_preparation(claim, result));
        });
    }

    pub(crate) fn start_reinitialize_backup_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
        else {
            return;
        };

        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Started(
            RecoveryAction::ReinitializeBackup,
        ));

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_enable(CloudBackupEnableContext::settings_manual()).await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }
}

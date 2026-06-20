use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_util::ResultExt as _;
use flume::Receiver;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    app::FfiApp,
    database::Database,
    manager::{
        cloud_backup_manager::{
            CLOUD_BACKUP_MANAGER, CloudBackupEnableContext, CloudBackupPasskeyHint,
            CloudBackupRestoreEvent, CloudBackupRestoreFlow, CloudBackupRestoreReport,
            CloudBackupVerificationSource, CloudStorageIssue, SavedPasskeyConfirmationMode,
        },
        connectivity_manager::CONNECTIVITY_MANAGER,
    },
    mnemonic::{Mnemonic as StoredMnemonic, MnemonicExt, NumberOfBip39Words},
    pending_wallet::PendingWallet,
    router::{HotWalletRoute, NewWalletRoute, Route},
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata},
    },
    word_validator::WordValidator,
};

use super::deferred_sender::{DeferredSender, MessageSender, SingleOrMany};

mod cloud_restore;
mod flow_state;
mod progress;

#[cfg(test)]
use self::cloud_restore::{
    CloudRestoreBackupSnapshot, choose_restore_provider_hint, record_cloud_restore_download_error,
    resolve_provider_hint,
};
pub(crate) use self::cloud_restore::{
    determine_cloud_check_outcome, inspect_cloud_restore_backup,
};
#[cfg(test)]
use self::flow_state::RestoreOrigin;
pub(crate) use self::flow_state::{
    CloudRestoreDiscovery, CompletionTarget, CreatedWalletFlow, FlowState, InternalEvent,
    InternalState, OnboardingCloudBackupEnableStart, PostOnboardingDestination, TermsContext,
    TransitionCommand,
};
pub(crate) use self::progress::{OnboardingProgress, resolve_initial_flow};
#[cfg(test)]
use cove_cspp::backup_data::PasskeyProviderHint;

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Default, uniffi::Enum)]
pub enum OnboardingStep {
    #[default]
    CloudCheck,
    RestoreOffer,
    RestoreOffline,
    RestoreUnavailable,
    Restoring,
    RestoreComplete,
    RestoreFailed,
    Welcome,
    BitcoinChoice,
    StorageChoice,
    CreatingWallet,
    BackupWallet,
    CloudBackup,
    SecretWords,
    ExchangeFunding,
    HardwareImport,
    SoftwareImport,
    Terms,
    CloudBackupSuccess,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize, uniffi::Enum)]
pub enum OnboardingBranch {
    NewUser,
    Exchange,
    SoftwareCreate,
    SoftwareImport,
    Hardware,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum OnboardingStorageSelection {
    Exchange,
    HardwareWallet,
    SoftwareWallet,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Default, uniffi::Enum)]
pub enum OnboardingCloudRestoreState {
    #[default]
    Checking,
    BackupFound,
    NoBackupFound,
    Inconclusive,
}

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub branch: Option<OnboardingBranch>,
    pub created_words: Vec<String>,
    pub cloud_backup_enabled: bool,
    pub secret_words_saved: bool,
    pub cloud_restore_state: OnboardingCloudRestoreState,
    pub cloud_restore_issue: Option<CloudCheckIssue>,
    pub cloud_restore_provider_hint: Option<CloudRestoreProviderHint>,
    pub should_offer_cloud_restore: bool,
    pub cloud_restore_alert_visible: bool,
    pub restore_state: OnboardingRestoreState,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Enum)]
pub enum OnboardingRestoreState {
    #[default]
    Idle,
    Restoring(CloudBackupRestoreFlow),
    Complete(CloudBackupRestoreReport),
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudRestoreProviderHint {
    pub provider_name: Option<String>,
    pub registered_at: u64,
    pub name_suffix: String,
}

impl From<CloudRestoreProviderHint> for CloudBackupPasskeyHint {
    fn from(value: CloudRestoreProviderHint) -> Self {
        Self {
            provider_name: value.provider_name,
            registered_at: value.registered_at,
            name_suffix: value.name_suffix,
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum OnboardingAction {
    ContinueFromWelcome,
    SelectHasBitcoin { has_bitcoin: bool },
    SelectStorage { selection: OnboardingStorageSelection },
    CreateSoftwareWallet,
    ContinueWalletCreation,
    ShowSecretWords,
    SecretWordsSaved,
    OpenCloudBackup,
    CloudBackupEnabled,
    SkipCloudBackup,
    ContinueFromBackup,
    ContinueFromExchangeFunding,
    SoftwareImportCompleted { wallet_id: WalletId },
    HardwareImportCompleted { wallet_id: WalletId },
    OpenCloudRestore,
    DismissCloudRestoreAlert,
    StartRestore,
    RetryRestore,
    SkipRestore,
    ContinueWithoutCloudRestore,
    ContinueFromRestoreComplete,
    AcceptTerms,
    Back,
    BeginCloudBackupEnable,
    ContinueFromCloudBackupSuccess,
}

type Message = OnboardingReconcileMessage;

#[derive(Debug, Clone, uniffi::Enum)]
pub enum OnboardingReconcileMessage {
    Step(OnboardingStep),
    Branch(Option<OnboardingBranch>),
    CreatedWords(Vec<String>),
    CloudBackupEnabled(bool),
    SecretWordsSaved(bool),
    CloudRestoreState(OnboardingCloudRestoreState),
    CloudRestoreIssueChanged(Option<CloudCheckIssue>),
    CloudRestoreProviderHintChanged(Option<CloudRestoreProviderHint>),
    ShouldOfferCloudRestore(bool),
    CloudRestoreAlertVisible(bool),
    RestoreStateChanged(OnboardingRestoreState),
    ErrorMessageChanged(Option<String>),
    Complete,
}

#[uniffi::export(callback_interface)]
pub trait OnboardingManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: OnboardingReconcileMessage);
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum CloudCheckOutcome {
    BackupFound(Option<CloudRestoreProviderHint>),
    NoBackupConfirmed,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, uniffi::Enum)]
pub enum CloudCheckIssue {
    Offline,
    CloudUnavailable,
    Unknown,
}

impl From<CloudStorageError> for CloudCheckIssue {
    fn from(error: CloudStorageError) -> Self {
        match CloudStorageIssue::from(error) {
            CloudStorageIssue::AuthorizationRequired | CloudStorageIssue::Unavailable => {
                Self::CloudUnavailable
            }
            CloudStorageIssue::Offline => Self::Offline,
            CloudStorageIssue::NotFound
            | CloudStorageIssue::QuotaExceeded
            | CloudStorageIssue::Other => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustOnboardingManager {
    state: Arc<RwLock<InternalState>>,
    cloud_check_in_flight: Arc<AtomicBool>,
    pending_cloud_check_retry: Arc<AtomicBool>,
    reconciler: MessageSender<Message>,
    reconcile_receiver: Arc<Receiver<SingleOrMany<Message>>>,
}

#[uniffi::export]
impl RustOnboardingManager {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        let (sender, receiver) = flume::bounded(100);
        let has_wallets = !Database::global().wallets.all().unwrap_or_default().is_empty();
        let resolution = resolve_initial_flow(
            Self::load_onboarding_progress(),
            has_wallets,
            |wallet_id, network, wallet_mode| {
                let wallet_exists = Database::global()
                    .wallets
                    .get(wallet_id, network, wallet_mode)
                    .ok()
                    .flatten()
                    .is_some();
                if !wallet_exists {
                    return None;
                }

                let mnemonic: bip39::Mnemonic = StoredMnemonic::try_from_id(wallet_id).ok()?.into();
                Some(mnemonic)
            },
        );
        let should_start_cloud_check = resolution.start_cloud_check;

        if resolution.clear_persisted_progress {
            Self::sync_onboarding_progress(None);
        }

        let manager = Arc::new(Self {
            state: Arc::new(RwLock::new(InternalState::new(resolution.flow))),
            cloud_check_in_flight: Arc::new(AtomicBool::new(false)),
            pending_cloud_check_retry: Arc::new(AtomicBool::new(false)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

        manager.start_connectivity_listener();

        if should_start_cloud_check {
            manager.start_cloud_check();
        }

        manager
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn OnboardingManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                match field {
                    SingleOrMany::Single(message) => reconciler.reconcile(message),
                    SingleOrMany::Many(messages) => {
                        for message in messages {
                            reconciler.reconcile(message);
                        }
                    }
                }
            }
        });
    }

    pub fn state(&self) -> OnboardingState {
        self.state.read().ui.clone()
    }

    pub fn current_wallet_id(&self) -> Option<WalletId> {
        self.state.read().flow.current_wallet_id()
    }

    pub fn word_validator(&self) -> Option<Arc<WordValidator>> {
        self.state.read().flow.word_validator()
    }

    pub fn dispatch(&self, action: OnboardingAction) {
        info!("Onboarding: dispatch action={action:?}");

        let command = self.mutate_state(|state, deferred| {
            let restore_attempt_id = if matches!(
                action,
                OnboardingAction::StartRestore | OnboardingAction::RetryRestore
            ) {
                let attempt_id = state.next_restore_attempt_id;
                state.next_restore_attempt_id = state.next_restore_attempt_id.wrapping_add(1);
                Some(attempt_id)
            } else {
                None
            };
            let command = state.flow.apply_user_action(
                action.clone(),
                state.cloud_restore_discovery.clone(),
                &mut state.restore_offer_allowed,
                restore_attempt_id,
            );
            if matches!(action, OnboardingAction::DismissCloudRestoreAlert)
                && matches!(
                    state.flow,
                    FlowState::HardwareImport | FlowState::SoftwareImport { .. }
                )
            {
                state.cloud_restore_alert_dismissed = true;
            }

            state.sync_ui(deferred);
            command
        });
        self.run_command(command);
    }
}

impl RustOnboardingManager {
    fn start_connectivity_listener(self: &Arc<Self>) {
        let manager = Arc::downgrade(self);
        let receiver = CONNECTIVITY_MANAGER.subscribe();

        std::thread::spawn(move || {
            while receiver.recv().is_ok() {
                let Some(manager) = manager.upgrade() else {
                    break;
                };

                let connected = CONNECTIVITY_MANAGER.connected();
                manager.handle_connectivity_change(connected);
            }
        });
    }

    fn handle_connectivity_change(self: &Arc<Self>, connected: bool) {
        if !connected {
            return;
        }

        if self.cloud_check_in_flight.load(Ordering::Acquire)
            && !self.mark_pending_cloud_check_retry()
        {
            return;
        }

        self.start_offline_cloud_check_retry();
    }

    fn mark_pending_cloud_check_retry(&self) -> bool {
        self.pending_cloud_check_retry.store(true, Ordering::Release);

        if self.cloud_check_in_flight.load(Ordering::Acquire) {
            return false;
        }

        self.pending_cloud_check_retry.swap(false, Ordering::AcqRel)
    }

    fn start_offline_cloud_check_retry(self: &Arc<Self>) {
        if !self.prepare_offline_cloud_check_retry() {
            return;
        }

        self.start_cloud_check();
    }

    fn start_cloud_check(self: &Arc<Self>) {
        if self.cloud_check_in_flight.swap(true, Ordering::AcqRel) {
            return;
        }

        let me = Arc::clone(self);
        cove_tokio::task::spawn(async move {
            if CLOUD_BACKUP_MANAGER.is_known_offline() {
                me.finish_cloud_check(CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline));
                return;
            }

            let cloud = CloudStorage::global_silent_client();
            let check_cloud_backup = || {
                let cloud = cloud.clone();
                async move { inspect_cloud_restore_backup(cloud).await }
            };

            let outcome =
                determine_cloud_check_outcome(check_cloud_backup, tokio::time::sleep).await;
            me.finish_cloud_check(outcome);
        });
    }

    fn prepare_offline_cloud_check_retry(&self) -> bool {
        self.mutate_state(|state, deferred| state.prepare_offline_cloud_check_retry(deferred))
    }

    fn finish_cloud_check(self: &Arc<Self>, outcome: CloudCheckOutcome) {
        let should_retry =
            self.finish_cloud_check_and_prepare_retry(outcome, CONNECTIVITY_MANAGER.connected());
        if should_retry {
            self.start_cloud_check();
        }
    }

    fn finish_cloud_check_and_prepare_retry(
        &self,
        outcome: CloudCheckOutcome,
        connected: bool,
    ) -> bool {
        self.mutate_state(|state, deferred| {
            state.flow.apply_event(
                InternalEvent::CloudCheckFinished(outcome.clone()),
                &mut state.cloud_restore_discovery,
                state.restore_offer_allowed,
            );
            self.cloud_check_in_flight.store(false, Ordering::Release);

            let retry_was_requested = self.pending_cloud_check_retry.swap(false, Ordering::AcqRel);
            let should_retry_offline_cloud_check = retry_was_requested
                && outcome == CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline)
                && connected;
            if should_retry_offline_cloud_check && state.prepare_offline_cloud_check_retry(deferred)
            {
                return true;
            }

            state.sync_ui(deferred);
            false
        })
    }

    fn apply_event(&self, event: InternalEvent) {
        let command = self.mutate_state(|state, deferred| {
            state.flow.apply_event(
                event,
                &mut state.cloud_restore_discovery,
                state.restore_offer_allowed,
            );
            state.sync_ui(deferred);
            TransitionCommand::None
        });
        self.run_command(command);
    }

    fn run_command(&self, command: TransitionCommand) {
        match command {
            TransitionCommand::None => {}
            TransitionCommand::CreateWallet(branch) => self.create_wallet_for_branch(branch),
            TransitionCommand::StartRestore { attempt_id } => {
                self.start_restore_attempt(attempt_id);
            }
            TransitionCommand::BeginCloudBackupEnable { discovery } => {
                self.begin_cloud_backup_enable(discovery);
            }
            TransitionCommand::CompleteOnboarding(target) => self.complete_onboarding(target),
        }
    }

    fn start_restore_attempt(&self, attempt_id: u64) {
        let receiver = CLOUD_BACKUP_MANAGER.restore_from_cloud_backup_with_events();
        let state = self.state.clone();
        let reconciler = self.reconciler.clone();

        cove_tokio::task::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                let internal_event = match event {
                    CloudBackupRestoreEvent::Progress(flow) => {
                        InternalEvent::RestoreProgress { attempt_id, flow }
                    }
                    CloudBackupRestoreEvent::Complete(report) => {
                        InternalEvent::RestoreComplete { attempt_id, report }
                    }
                    CloudBackupRestoreEvent::NoBackupFound => {
                        InternalEvent::RestoreNoBackupFound { attempt_id }
                    }
                    CloudBackupRestoreEvent::Failed(message) => {
                        InternalEvent::RestoreFailed { attempt_id, message }
                    }
                };
                Self::apply_restore_event(&state, &reconciler, internal_event);
            }
        });

        let state = self.state.clone();
        let reconciler = self.reconciler.clone();
        cove_tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(120)).await;

            if !Self::is_restore_attempt_current(&state, attempt_id) {
                return;
            }

            CLOUD_BACKUP_MANAGER.cancel_restore_and_wait().await;

            Self::apply_restore_event(
                &state,
                &reconciler,
                InternalEvent::RestoreFailed { attempt_id, message: "Restore timed out".into() },
            );
        });
    }

    fn is_restore_attempt_current(state: &Arc<RwLock<InternalState>>, attempt_id: u64) -> bool {
        state.read().flow.is_restore_attempt_current(attempt_id)
    }

    fn apply_restore_event(
        state: &Arc<RwLock<InternalState>>,
        reconciler: &MessageSender<Message>,
        event: InternalEvent,
    ) -> bool {
        Self::mutate_state_fields(state, reconciler, |state, deferred| {
            let was_current_restore_attempt = state.flow.is_restore_event_current(&event);
            state.flow.apply_event(
                event,
                &mut state.cloud_restore_discovery,
                state.restore_offer_allowed,
            );
            state.sync_ui(deferred);
            was_current_restore_attempt
        })
    }

    fn begin_cloud_backup_enable(&self, discovery: CloudRestoreDiscovery) {
        info!("Onboarding: begin cloud backup enable discovery={discovery:?}");
        let context = CloudBackupEnableContext {
            saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
            verification_source: CloudBackupVerificationSource::Onboarding,
        };
        CLOUD_BACKUP_MANAGER.clear_existing_backup_found_prompt();
        CLOUD_BACKUP_MANAGER.clear_passkey_choice_prompt();

        match OnboardingCloudBackupEnableStart::from_discovery(discovery) {
            OnboardingCloudBackupEnableStart::ConfirmExistingBackup(hint) => {
                info!("Onboarding: confirming existing cloud backup before creating passkey");
                CLOUD_BACKUP_MANAGER.present_existing_backup_found_prompt(
                    context,
                    hint.map(CloudBackupPasskeyHint::from),
                );
            }
            OnboardingCloudBackupEnableStart::CreateNewPasskey => {
                info!("Onboarding: enabling cloud backup without passkey discovery");
                CLOUD_BACKUP_MANAGER.enable_cloud_backup_no_discovery(context);
            }
        }
    }

    fn create_wallet_for_branch(&self, branch: OnboardingBranch) {
        let event = match Self::create_wallet(branch) {
            Ok(flow) => InternalEvent::WalletCreated { flow },
            Err(error) => InternalEvent::WalletCreationFailed { branch, error },
        };
        self.apply_event(event);
    }

    fn complete_onboarding(&self, target: CompletionTarget) {
        let result = match target {
            CompletionTarget::SelectLatestOrNew => {
                FfiApp::global().select_latest_or_new_wallet().map_err_str(std::convert::identity)
            }
            CompletionTarget::SelectWallet { wallet_id, post_onboarding } => {
                let next_route = match post_onboarding {
                    PostOnboardingDestination::None => None,
                    PostOnboardingDestination::VerifyWords => Some(Route::NewWallet(
                        NewWalletRoute::HotWallet(HotWalletRoute::VerifyWords(wallet_id.clone())),
                    )),
                };

                FfiApp::global()
                    .select_wallet(wallet_id, next_route)
                    .map_err_str(std::convert::identity)
            }
        };

        match result {
            Ok(()) => {
                if let Err(error) = Database::global().global_flag.mark_onboarding_complete() {
                    self.apply_event(InternalEvent::CompletionFailed {
                        error: format!("failed to persist onboarding completion: {error}"),
                    });
                    return;
                }
                Self::sync_onboarding_progress(None);
                self.send(Message::Complete);
            }
            Err(error) => self.apply_event(InternalEvent::CompletionFailed { error }),
        }
    }

    fn mutate_state<F, R>(&self, mutate: F) -> R
    where
        F: FnOnce(&mut InternalState, &mut DeferredSender<Message>) -> R,
    {
        Self::mutate_state_fields(&self.state, &self.reconciler, mutate)
    }

    fn mutate_state_fields<F, R>(
        state: &Arc<RwLock<InternalState>>,
        reconciler: &MessageSender<Message>,
        mutate: F,
    ) -> R
    where
        F: FnOnce(&mut InternalState, &mut DeferredSender<Message>) -> R,
    {
        let mut deferred = DeferredSender::new(reconciler.clone());
        let (result, progress) = {
            let mut state = state.write();
            let result = mutate(&mut state, &mut deferred);
            let progress = state.flow.persisted_progress();
            (result, progress)
        };
        Self::sync_onboarding_progress(progress);
        result
    }

    fn send(&self, message: Message) {
        self.reconciler.send(message);
    }

    fn create_wallet(branch: OnboardingBranch) -> Result<CreatedWalletFlow, String> {
        let pending_wallet = PendingWallet::new(NumberOfBip39Words::Twelve, None);
        let mnemonic = pending_wallet.mnemonic.clone();
        let words = pending_wallet.words();
        let network = pending_wallet.network;
        let mode = Database::global().global_config.wallet_mode();
        let number_of_wallets = Database::global().wallets.len(network, mode).unwrap_or(0);

        let name = format!("Wallet {}", number_of_wallets + 1);
        let fingerprint: Fingerprint = mnemonic.xpub(network.into()).fingerprint().into();
        let wallet_metadata = WalletMetadata::new_cove_created_wallet(name, Some(fingerprint));
        let wallet =
            Wallet::try_new_persisted_and_selected(wallet_metadata, mnemonic.clone(), None)
                .map_err_str(std::convert::identity)?;
        CLOUD_BACKUP_MANAGER.mark_verification_required_after_wallet_change();

        Ok(CreatedWalletFlow {
            branch,
            wallet_id: wallet.metadata.id,
            network,
            wallet_mode: mode,
            created_words: words,
            word_validator: Arc::new(WordValidator::new(mnemonic)),
            cloud_backup_enabled: false,
            secret_words_saved: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use bip39::Mnemonic;
    use cove_cspp::backup_data::PasskeyRegistrationPlatform as BackupPasskeyRegistrationPlatform;
    use cove_types::network::Network;

    use super::flow_state::CloudBackupFlow;
    use super::*;
    use crate::wallet::metadata::WalletMode;

    #[test]
    fn continue_from_backup_requires_a_saved_backup_method() {
        let mut flow =
            FlowState::BackupWallet(preview_created_wallet_flow(OnboardingBranch::NewUser));
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromBackup,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::BackupWallet(_)));
    }

    #[test]
    fn software_import_completion_goes_to_cloud_backup() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::SoftwareImport { error_message: None };
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::SoftwareImportCompleted { wallet_id: wallet_id.clone() },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn hardware_import_completion_goes_to_cloud_backup() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::HardwareImport;
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::HardwareImportCompleted { wallet_id: wallet_id.clone() },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn enabling_cloud_backup_after_software_import_goes_to_success() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::CloudBackupEnabled,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::CloudBackupSuccess(CloudBackupFlow::SoftwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn enabling_cloud_backup_after_hardware_import_goes_to_success() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::CloudBackupEnabled,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::CloudBackupSuccess(CloudBackupFlow::HardwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn enabling_cloud_backup_after_created_wallet_goes_to_success() {
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::CreatedWallet(
            preview_created_wallet_flow(OnboardingBranch::NewUser),
        ));
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::CloudBackupEnabled,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(CreatedWalletFlow {
                cloud_backup_enabled: true,
                ..
            }))
        ));
    }

    #[test]
    fn continuing_from_created_wallet_cloud_backup_success_goes_to_backup_wallet() {
        let mut preview = preview_created_wallet_flow(OnboardingBranch::NewUser);
        preview.cloud_backup_enabled = true;
        let mut flow = FlowState::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(preview));
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromCloudBackupSuccess,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::BackupWallet(CreatedWalletFlow { cloud_backup_enabled: true, .. })
        ));
    }

    #[test]
    fn continuing_from_software_import_cloud_backup_success_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackupSuccess(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromCloudBackupSuccess,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::None);
    }

    #[test]
    fn continuing_from_hardware_import_cloud_backup_success_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackupSuccess(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromCloudBackupSuccess,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::None);
    }

    #[test]
    fn begin_cloud_backup_enable_uses_backup_found_discovery() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::BeginCloudBackupEnable,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(
            command,
            TransitionCommand::BeginCloudBackupEnable {
                discovery: CloudRestoreDiscovery::BackupFound(None)
            }
        );
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn begin_cloud_backup_enable_uses_no_discovery_when_no_backup_found() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::BeginCloudBackupEnable,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(
            command,
            TransitionCommand::BeginCloudBackupEnable {
                discovery: CloudRestoreDiscovery::NoBackupFound
            }
        );
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn onboarding_cloud_backup_enable_start_confirms_existing_backup() {
        assert_eq!(
            OnboardingCloudBackupEnableStart::from_discovery(CloudRestoreDiscovery::BackupFound(
                None,
            )),
            OnboardingCloudBackupEnableStart::ConfirmExistingBackup(None),
        );
    }

    #[test]
    fn onboarding_cloud_backup_enable_start_creates_new_without_passkey_discovery_by_default() {
        assert_eq!(
            OnboardingCloudBackupEnableStart::from_discovery(CloudRestoreDiscovery::NoBackupFound),
            OnboardingCloudBackupEnableStart::CreateNewPasskey,
        );
        assert_eq!(
            OnboardingCloudBackupEnableStart::from_discovery(CloudRestoreDiscovery::Checking),
            OnboardingCloudBackupEnableStart::CreateNewPasskey,
        );
        assert_eq!(
            OnboardingCloudBackupEnableStart::from_discovery(CloudRestoreDiscovery::Inconclusive(
                CloudCheckIssue::Offline,
            )),
            OnboardingCloudBackupEnableStart::CreateNewPasskey,
        );
    }

    #[test]
    fn begin_cloud_backup_enable_preserves_inconclusive_discovery() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::BeginCloudBackupEnable,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(
            command,
            TransitionCommand::BeginCloudBackupEnable {
                discovery: CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline)
            }
        );
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn begin_cloud_backup_enable_preserves_checking_discovery() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::BeginCloudBackupEnable,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(
            command,
            TransitionCommand::BeginCloudBackupEnable {
                discovery: CloudRestoreDiscovery::Checking
            }
        );
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn skipping_cloud_backup_after_software_import_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::SkipCloudBackup,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms {
                context:
                    TermsContext::SelectWallet {
                        wallet_id: id,
                        post_onboarding: PostOnboardingDestination::None,
                    },
                ..
            } => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn skipping_cloud_backup_after_hardware_import_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::SkipCloudBackup,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms {
                context:
                    TermsContext::SelectWallet {
                        wallet_id: id,
                        post_onboarding: PostOnboardingDestination::None,
                    },
                ..
            } => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn hardware_import_cloud_backup_ui_state_uses_hardware_branch() {
        let wallet_id = WalletId::new();
        let flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id });

        let state = flow.ui_state(&CloudRestoreDiscovery::Checking, false, false);

        assert_eq!(state.step, OnboardingStep::CloudBackup);
        assert_eq!(state.branch, Some(OnboardingBranch::Hardware));
    }

    #[test]
    fn new_user_saved_words_flow_completes_with_verify_destination() {
        let preview = preview_created_wallet_flow(OnboardingBranch::NewUser);
        let wallet_id = preview.wallet_id.clone();
        let mut flow = FlowState::BackupWallet(preview);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::ShowSecretWords),
            TransitionCommand::None
        );
        assert!(matches!(flow, FlowState::SecretWords(_)));

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::SecretWordsSaved),
            TransitionCommand::None
        );
        assert!(matches!(
            flow,
            FlowState::BackupWallet(CreatedWalletFlow { secret_words_saved: true, .. })
        ));

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::ContinueFromBackup),
            TransitionCommand::None
        );
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::VerifyWords);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::AcceptTerms),
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::VerifyWords,
            })
        );
    }

    #[test]
    fn exchange_created_wallet_flow_completes_after_funding_screen() {
        let mut preview = preview_created_wallet_flow(OnboardingBranch::Exchange);
        preview.secret_words_saved = true;
        let wallet_id = preview.wallet_id.clone();
        let mut flow = FlowState::BackupWallet(preview);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::ContinueFromBackup),
            TransitionCommand::None
        );
        assert!(matches!(flow, FlowState::ExchangeFunding(_)));

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::ContinueFromExchangeFunding),
            TransitionCommand::None
        );
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::VerifyWords);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::AcceptTerms),
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::VerifyWords,
            })
        );
    }

    #[test]
    fn software_create_saved_words_flow_completes_with_verify_destination() {
        let mut preview = preview_created_wallet_flow(OnboardingBranch::SoftwareCreate);
        preview.secret_words_saved = true;
        let wallet_id = preview.wallet_id.clone();
        let mut flow = FlowState::BackupWallet(preview);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::ContinueFromBackup),
            TransitionCommand::None
        );
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::VerifyWords);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::AcceptTerms),
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::VerifyWords,
            })
        );
    }

    #[test]
    fn software_import_skip_cloud_backup_flow_completes_selected_wallet() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::SoftwareImport { error_message: None };

        assert_eq!(
            apply_action(
                &mut flow,
                OnboardingAction::SoftwareImportCompleted { wallet_id: wallet_id.clone() },
            ),
            TransitionCommand::None
        );
        assert!(matches!(flow, FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { .. })));

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::SkipCloudBackup),
            TransitionCommand::None
        );
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::None);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::AcceptTerms),
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::None,
            })
        );
    }

    #[test]
    fn hardware_import_skip_cloud_backup_flow_completes_selected_wallet() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::HardwareImport;

        assert_eq!(
            apply_action(
                &mut flow,
                OnboardingAction::HardwareImportCompleted { wallet_id: wallet_id.clone() },
            ),
            TransitionCommand::None
        );
        assert!(matches!(flow, FlowState::CloudBackup(CloudBackupFlow::HardwareImport { .. })));

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::SkipCloudBackup),
            TransitionCommand::None
        );
        assert_terms_select_wallet(&flow, &wallet_id, PostOnboardingDestination::None);

        assert_eq!(
            apply_action(&mut flow, OnboardingAction::AcceptTerms),
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::None,
            })
        );
    }

    #[test]
    fn continue_from_backup_without_cloud_backup_goes_to_terms_with_verify_destination() {
        let mut preview = preview_created_wallet_flow(OnboardingBranch::NewUser);
        preview.secret_words_saved = true;

        let wallet_id = preview.wallet_id.clone();
        let mut flow = FlowState::BackupWallet(preview);
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromBackup,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms {
                context:
                    TermsContext::SelectWallet {
                        wallet_id: id,
                        post_onboarding: PostOnboardingDestination::VerifyWords,
                    },
                ..
            } => assert_eq!(id, wallet_id),
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn continue_from_exchange_without_cloud_backup_goes_to_terms_with_verify_destination() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::ExchangeFunding(CreatedWalletFlow {
            wallet_id: wallet_id.clone(),
            branch: OnboardingBranch::Exchange,
            ..preview_created_wallet_flow(OnboardingBranch::Exchange)
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromExchangeFunding,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms {
                context:
                    TermsContext::SelectWallet {
                        wallet_id: id,
                        post_onboarding: PostOnboardingDestination::VerifyWords,
                    },
                ..
            } => assert_eq!(id, wallet_id),
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn welcome_continues_to_bitcoin_choice() {
        let mut flow = FlowState::Welcome { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromWelcome,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::BitcoinChoice { error_message: None }));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn existing_bitcoin_choice_goes_directly_to_storage_choice() {
        let mut flow = FlowState::BitcoinChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SelectHasBitcoin { has_bitcoin: true },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn selecting_hardware_wallet_goes_to_hardware_import() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SelectStorage {
                selection: OnboardingStorageSelection::HardwareWallet,
            },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::HardwareImport));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn selecting_software_wallet_goes_to_software_import() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SelectStorage {
                selection: OnboardingStorageSelection::SoftwareWallet,
            },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn storage_choice_back_returns_to_bitcoin_choice() {
        let mut flow = FlowState::StorageChoice { error_message: Some("create failed".into()) };
        let mut restore_offer_allowed = true;

        flow.apply_user_action(
            OnboardingAction::Back,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert!(matches!(flow, FlowState::BitcoinChoice { error_message: None }));
    }

    #[test]
    fn hardware_back_returns_to_storage_choice() {
        let mut flow = FlowState::HardwareImport;
        let mut restore_offer_allowed = false;

        flow.apply_user_action(
            OnboardingAction::Back,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn software_back_returns_to_storage_choice() {
        let mut flow = FlowState::SoftwareImport { error_message: Some("create failed".into()) };
        let mut restore_offer_allowed = false;

        flow.apply_user_action(
            OnboardingAction::Back,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            None,
        );

        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn invalid_action_leaves_current_flow_unchanged() {
        let mut flow = FlowState::SoftwareImport { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromBackup,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn restoring_failure_enters_restore_failed_with_message() {
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::StorageChoice,
            attempt_id: 1,
            flow: CloudBackupRestoreFlow::Finding,
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreFailed {
                attempt_id: 1,
                message: "passkey verification failed".into(),
            },
            &mut discovery,
            true,
        );

        assert!(matches!(
            flow,
            FlowState::RestoreFailed {
                origin: RestoreOrigin::StorageChoice,
                message,
            } if message == "passkey verification failed"
        ));
    }

    #[test]
    fn start_restore_enters_restoring_finding() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::StartRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            Some(12),
        );

        assert_eq!(command, TransitionCommand::StartRestore { attempt_id: 12 });
        assert!(matches!(
            flow,
            FlowState::Restoring {
                origin: RestoreOrigin::Welcome,
                attempt_id: 12,
                flow: CloudBackupRestoreFlow::Finding,
            }
        ));

        let ui = flow.ui_state(&CloudRestoreDiscovery::BackupFound(None), true, false);
        assert_eq!(ui.step, OnboardingStep::Restoring);
        assert_eq!(
            ui.restore_state,
            OnboardingRestoreState::Restoring(CloudBackupRestoreFlow::Finding)
        );
    }

    #[test]
    fn restore_progress_keeps_restoring_and_updates_restore_state() {
        let progress = CloudBackupRestoreFlow::Downloading { completed: 2, total: 5 };
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::StorageChoice,
            attempt_id: 7,
            flow: CloudBackupRestoreFlow::Finding,
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreProgress { attempt_id: 7, flow: progress.clone() },
            &mut discovery,
            true,
        );

        assert!(matches!(
            flow,
            FlowState::Restoring {
                origin: RestoreOrigin::StorageChoice,
                attempt_id: 7,
                ref flow,
            } if flow == &progress
        ));

        let ui = flow.ui_state(&discovery, true, false);
        assert_eq!(ui.step, OnboardingStep::Restoring);
        assert_eq!(ui.restore_state, OnboardingRestoreState::Restoring(progress));
    }

    #[test]
    fn restore_success_enters_restore_complete_with_report() {
        let report = preview_restore_report();
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::Welcome,
            attempt_id: 7,
            flow: CloudBackupRestoreFlow::Restoring { completed: 1, total: 1 },
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreComplete { attempt_id: 7, report: report.clone() },
            &mut discovery,
            true,
        );

        assert!(matches!(
            flow,
            FlowState::RestoreComplete {
                origin: RestoreOrigin::Welcome,
                report: ref stored_report,
            } if stored_report == &report
        ));

        let ui = flow.ui_state(&discovery, true, false);
        assert_eq!(ui.step, OnboardingStep::RestoreComplete);
        assert_eq!(ui.restore_state, OnboardingRestoreState::Complete(report));
    }

    #[test]
    fn restore_no_backup_found_enters_restore_unavailable() {
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::StorageChoice,
            attempt_id: 7,
            flow: CloudBackupRestoreFlow::Finding,
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreNoBackupFound { attempt_id: 7 },
            &mut discovery,
            true,
        );

        assert!(matches!(
            flow,
            FlowState::RestoreUnavailable { origin: RestoreOrigin::StorageChoice }
        ));
        assert_eq!(discovery, CloudRestoreDiscovery::NoBackupFound);

        let ui = flow.ui_state(&discovery, true, false);
        assert_eq!(ui.step, OnboardingStep::RestoreUnavailable);
        assert_eq!(ui.cloud_restore_state, OnboardingCloudRestoreState::NoBackupFound);
    }

    #[test]
    fn stale_restore_no_backup_found_is_ignored() {
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::StorageChoice,
            attempt_id: 7,
            flow: CloudBackupRestoreFlow::Finding,
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreNoBackupFound { attempt_id: 6 },
            &mut discovery,
            true,
        );

        assert!(matches!(flow, FlowState::Restoring { attempt_id: 7, .. }));
        assert_eq!(discovery, CloudRestoreDiscovery::BackupFound(None));
    }

    #[test]
    fn done_from_restore_complete_goes_to_latest_wallet_terms() {
        let mut flow = FlowState::RestoreComplete {
            origin: RestoreOrigin::Welcome,
            report: preview_restore_report(),
        };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromRestoreComplete,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::Terms {
                context: TermsContext::SelectLatestOrNew,
                error_message: None,
                progress: None,
            }
        ));
    }

    #[test]
    fn retry_restore_starts_new_attempt_and_ignores_stale_old_attempt_events() {
        let mut flow = FlowState::RestoreFailed {
            origin: RestoreOrigin::StorageChoice,
            message: "restore failed".into(),
        };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::RetryRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            Some(2),
        );

        assert_eq!(command, TransitionCommand::StartRestore { attempt_id: 2 });
        assert!(matches!(flow, FlowState::Restoring { attempt_id: 2, .. }));

        let mut discovery = CloudRestoreDiscovery::BackupFound(None);
        flow.apply_event(
            InternalEvent::RestoreFailed { attempt_id: 1, message: "old failure".into() },
            &mut discovery,
            true,
        );

        assert!(matches!(flow, FlowState::Restoring { attempt_id: 2, .. }));
    }

    #[test]
    fn skip_restore_from_failed_follows_original_origin() {
        let mut flow =
            FlowState::RestoreFailed { origin: RestoreOrigin::Welcome, message: "failed".into() };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn timeout_failure_enters_restore_failed_with_timeout_message() {
        let mut flow = FlowState::Restoring {
            origin: RestoreOrigin::Welcome,
            attempt_id: 3,
            flow: CloudBackupRestoreFlow::Finding,
        };
        let mut discovery = CloudRestoreDiscovery::BackupFound(None);

        flow.apply_event(
            InternalEvent::RestoreFailed { attempt_id: 3, message: "Restore timed out".into() },
            &mut discovery,
            true,
        );

        assert!(matches!(
            flow,
            FlowState::RestoreFailed {
                origin: RestoreOrigin::Welcome,
                message,
            } if message == "Restore timed out"
        ));
    }

    #[test]
    fn explicit_restore_without_backup_goes_to_restore_unavailable() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::OpenCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreUnavailable { origin: RestoreOrigin::StorageChoice }
        ));
    }

    #[test]
    fn explicit_restore_from_bitcoin_choice_can_try_when_cloud_is_unavailable() {
        let mut flow = FlowState::BitcoinChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::OpenCloudRestore,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::CloudUnavailable),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreOffer { origin: RestoreOrigin::BitcoinChoice, error_message: None }
        ));
    }

    #[test]
    fn explicit_restore_while_offline_goes_to_restore_offline() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::OpenCloudRestore,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::RestoreOffline { origin: RestoreOrigin::StorageChoice }));
    }

    #[test]
    fn empty_wallet_startup_begins_at_welcome_and_starts_background_cloud_check() {
        let resolution = resolve_initial_flow(None, false, |_, _, _| None);

        assert!(!resolution.clear_persisted_progress);
        assert!(resolution.start_cloud_check);
        assert!(matches!(resolution.flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn backup_found_auto_switches_on_early_screens() {
        let scenarios = [
            (FlowState::Welcome { error_message: None }, RestoreOrigin::Welcome),
            (FlowState::BitcoinChoice { error_message: None }, RestoreOrigin::BitcoinChoice),
            (FlowState::StorageChoice { error_message: None }, RestoreOrigin::StorageChoice),
        ];

        for (mut flow, expected_origin) in scenarios {
            let mut discovery = CloudRestoreDiscovery::Checking;

            flow.apply_event(
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(None)),
                &mut discovery,
                true,
            );

            assert_eq!(discovery, CloudRestoreDiscovery::BackupFound(None));
            assert!(matches!(
                flow,
                FlowState::RestoreOffer { origin, error_message: None } if origin == expected_origin
            ));
        }
    }

    #[test]
    fn backup_found_on_import_screen_leaves_step_for_alert() {
        let mut flow = FlowState::HardwareImport;
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(None)),
            &mut discovery,
            true,
        );

        assert_eq!(discovery, CloudRestoreDiscovery::BackupFound(None));
        assert!(matches!(flow, FlowState::HardwareImport));
    }

    #[test]
    fn backup_found_on_import_screen_sets_alert_state() {
        let mut state = preview_internal_state(
            FlowState::SoftwareImport { error_message: None },
            CloudRestoreDiscovery::Checking,
        );

        state.flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(None)),
            &mut state.cloud_restore_discovery,
            state.restore_offer_allowed,
        );
        let mut deferred = DeferredSender::new(MessageSender::new(flume::bounded(16).0));
        state.sync_ui(&mut deferred);

        assert_eq!(state.ui.step, OnboardingStep::SoftwareImport);
        assert!(state.ui.cloud_restore_alert_visible);
    }

    #[test]
    fn dismissed_cloud_restore_alert_stays_hidden() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager =
            preview_manager(FlowState::HardwareImport, CloudRestoreDiscovery::BackupFound(None));

        manager.dispatch(OnboardingAction::DismissCloudRestoreAlert);

        assert_eq!(manager.state().step, OnboardingStep::HardwareImport);
        assert!(!manager.state().cloud_restore_alert_visible);
    }

    #[test]
    fn stale_dismiss_cloud_restore_alert_outside_import_does_not_hide_later_import_alert() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = preview_manager(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::BackupFound(None),
        );

        manager.dispatch(OnboardingAction::DismissCloudRestoreAlert);
        manager.dispatch(OnboardingAction::ContinueFromWelcome);
        manager.dispatch(OnboardingAction::SelectHasBitcoin { has_bitcoin: true });
        manager.dispatch(OnboardingAction::SelectStorage {
            selection: OnboardingStorageSelection::SoftwareWallet,
        });

        assert_eq!(manager.state().step, OnboardingStep::SoftwareImport);
        assert!(manager.state().cloud_restore_alert_visible);
    }

    #[test]
    fn opening_restore_from_import_returns_to_import_on_skip() {
        let mut flow = FlowState::SoftwareImport { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::OpenCloudRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreOffer { origin: RestoreOrigin::SoftwareImport, error_message: None }
        ));

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
    }

    #[test]
    fn wallet_creation_failures_return_to_origin_step_with_error() {
        let scenarios = [
            (
                FlowState::BitcoinChoice { error_message: None },
                OnboardingBranch::NewUser,
                OnboardingStep::BitcoinChoice,
            ),
            (
                FlowState::StorageChoice { error_message: None },
                OnboardingBranch::Exchange,
                OnboardingStep::StorageChoice,
            ),
            (
                FlowState::SoftwareImport { error_message: None },
                OnboardingBranch::SoftwareCreate,
                OnboardingStep::SoftwareImport,
            ),
        ];

        for (mut flow, branch, step) in scenarios {
            let mut discovery = CloudRestoreDiscovery::Checking;

            flow.apply_event(
                InternalEvent::WalletCreationFailed { branch, error: "create failed".into() },
                &mut discovery,
                false,
            );

            let state = flow.ui_state(&CloudRestoreDiscovery::Checking, false, false);
            assert_eq!(state.step, step);
            assert_eq!(state.error_message.as_deref(), Some("create failed"));
        }
    }

    #[test]
    fn retrying_wallet_creation_clears_branch_error() {
        let scenarios = [
            (
                FlowState::BitcoinChoice { error_message: Some("create failed".into()) },
                OnboardingAction::SelectHasBitcoin { has_bitcoin: false },
                TransitionCommand::CreateWallet(OnboardingBranch::NewUser),
                OnboardingStep::BitcoinChoice,
            ),
            (
                FlowState::StorageChoice { error_message: Some("create failed".into()) },
                OnboardingAction::SelectStorage { selection: OnboardingStorageSelection::Exchange },
                TransitionCommand::CreateWallet(OnboardingBranch::Exchange),
                OnboardingStep::StorageChoice,
            ),
            (
                FlowState::SoftwareImport { error_message: Some("create failed".into()) },
                OnboardingAction::CreateSoftwareWallet,
                TransitionCommand::CreateWallet(OnboardingBranch::SoftwareCreate),
                OnboardingStep::SoftwareImport,
            ),
        ];

        for (mut flow, action, expected_command, expected_step) in scenarios {
            let mut restore_offer_allowed = true;

            let command = flow.apply_user_action(
                action,
                CloudRestoreDiscovery::Checking,
                &mut restore_offer_allowed,
                None,
            );

            assert_eq!(command, expected_command);
            assert!(!restore_offer_allowed);

            let state = flow.ui_state(&CloudRestoreDiscovery::Checking, false, false);
            assert_eq!(state.step, expected_step);
            assert_eq!(state.error_message, None);
        }
    }

    #[test]
    fn back_navigation_drops_branch_errors() {
        let scenarios = [
            (
                FlowState::BitcoinChoice { error_message: Some("create failed".into()) },
                OnboardingStep::Welcome,
            ),
            (
                FlowState::StorageChoice { error_message: Some("create failed".into()) },
                OnboardingStep::BitcoinChoice,
            ),
            (
                FlowState::SoftwareImport { error_message: Some("create failed".into()) },
                OnboardingStep::StorageChoice,
            ),
        ];

        for (mut flow, expected_step) in scenarios {
            let mut restore_offer_allowed = true;

            let command = flow.apply_user_action(
                OnboardingAction::Back,
                CloudRestoreDiscovery::Checking,
                &mut restore_offer_allowed,
                None,
            );

            assert_eq!(command, TransitionCommand::None);

            let state = flow.ui_state(&CloudRestoreDiscovery::Checking, false, false);
            assert_eq!(state.step, expected_step);
            assert_eq!(state.error_message, None);
        }
    }

    #[test]
    fn branch_step_errors_project_into_ui_state() {
        let scenarios = [
            (
                FlowState::BitcoinChoice { error_message: Some("new user failed".into()) },
                OnboardingStep::BitcoinChoice,
                "new user failed",
            ),
            (
                FlowState::StorageChoice { error_message: Some("exchange failed".into()) },
                OnboardingStep::StorageChoice,
                "exchange failed",
            ),
            (
                FlowState::SoftwareImport { error_message: Some("software failed".into()) },
                OnboardingStep::SoftwareImport,
                "software failed",
            ),
        ];

        for (flow, expected_step, expected_error) in scenarios {
            let state = flow.ui_state(&CloudRestoreDiscovery::Checking, false, false);

            assert_eq!(state.step, expected_step);
            assert_eq!(state.error_message.as_deref(), Some(expected_error));
        }
    }

    #[test]
    fn no_backup_during_background_startup_check_leaves_welcome_visible() {
        let mut flow = FlowState::Welcome { error_message: None };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::NoBackupConfirmed),
            &mut discovery,
            true,
        );

        assert_eq!(discovery, CloudRestoreDiscovery::NoBackupFound);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn inconclusive_background_startup_check_leaves_welcome_visible() {
        let mut flow = FlowState::Welcome { error_message: None };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(
                CloudCheckIssue::CloudUnavailable,
            )),
            &mut discovery,
            true,
        );

        assert_eq!(
            discovery,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::CloudUnavailable)
        );
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn cloud_check_offline_goes_to_restore_offline_screen() {
        let mut flow = FlowState::CloudCheck { origin: RestoreOrigin::Welcome };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(
                CloudCheckIssue::Offline,
            )),
            &mut discovery,
            true,
        );

        assert_eq!(discovery, CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline));
        assert!(matches!(flow, FlowState::RestoreOffline { origin: RestoreOrigin::Welcome }));
        assert_eq!(flow.ui_state(&discovery, false, false).step, OnboardingStep::RestoreOffline);
    }

    #[test]
    fn cloud_check_non_offline_inconclusive_keeps_restore_offer_flow() {
        let mut flow = FlowState::CloudCheck { origin: RestoreOrigin::Welcome };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(
                CloudCheckIssue::CloudUnavailable,
            )),
            &mut discovery,
            true,
        );

        assert_eq!(
            discovery,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::CloudUnavailable)
        );
        assert!(matches!(
            flow,
            FlowState::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None }
        ));
    }

    #[test]
    fn skip_restore_returns_to_origin_and_disables_future_prompts() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::StorageChoice, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn back_from_restore_offer_returns_to_origin_and_keeps_future_prompts() {
        let scenarios = [
            RestoreOrigin::BitcoinChoice,
            RestoreOrigin::StorageChoice,
            RestoreOrigin::HardwareImport,
            RestoreOrigin::SoftwareImport,
        ];

        for origin in scenarios {
            let mut flow = FlowState::RestoreOffer { origin, error_message: None };
            let mut restore_offer_allowed = true;

            let command = flow.apply_user_action(
                OnboardingAction::Back,
                CloudRestoreDiscovery::BackupFound(None),
                &mut restore_offer_allowed,
                None,
            );

            assert_eq!(command, TransitionCommand::None);
            assert!(restore_offer_allowed);
            assert_restore_offer_back_origin(flow, origin);
        }
    }

    #[test]
    fn back_from_welcome_restore_offer_returns_to_welcome() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::Back,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(restore_offer_allowed);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn skip_restore_from_welcome_check_returns_to_welcome() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn continue_without_cloud_restore_from_welcome_returns_to_welcome() {
        let mut flow = FlowState::RestoreUnavailable { origin: RestoreOrigin::Welcome };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn continue_without_cloud_restore_from_welcome_offline_returns_to_welcome() {
        let mut flow = FlowState::RestoreOffline { origin: RestoreOrigin::Welcome };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn continue_without_cloud_restore_from_import_returns_to_import() {
        let mut flow = FlowState::RestoreUnavailable { origin: RestoreOrigin::SoftwareImport };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
            None,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
    }

    #[test]
    fn offline_retry_rechecks_from_restore_offline_screen() {
        let mut state = preview_internal_state(
            FlowState::RestoreOffline { origin: RestoreOrigin::Welcome },
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
        );

        assert!(prepare_offline_cloud_check_retry(&mut state));
        assert_eq!(state.cloud_restore_discovery, CloudRestoreDiscovery::Checking);
        assert!(matches!(state.flow, FlowState::CloudCheck { origin: RestoreOrigin::Welcome }));
        assert_eq!(state.ui.step, OnboardingStep::CloudCheck);
        assert_eq!(state.ui.cloud_restore_state, OnboardingCloudRestoreState::Checking);
    }

    #[test]
    fn offline_retry_rechecks_in_background_on_early_screens() {
        let mut state = preview_internal_state(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
        );

        assert!(prepare_offline_cloud_check_retry(&mut state));
        assert_eq!(state.cloud_restore_discovery, CloudRestoreDiscovery::Checking);
        assert!(matches!(state.flow, FlowState::Welcome { error_message: None }));
        assert_eq!(state.ui.step, OnboardingStep::Welcome);
        assert_eq!(state.ui.cloud_restore_state, OnboardingCloudRestoreState::Checking);
        assert_eq!(state.ui.cloud_restore_issue, None);
    }

    #[test]
    fn offline_retry_rechecks_on_import_screens_and_shows_alert_after_backup_found() {
        let scenarios = [
            (FlowState::HardwareImport, OnboardingStep::HardwareImport),
            (FlowState::SoftwareImport { error_message: None }, OnboardingStep::SoftwareImport),
        ];

        for (flow, expected_step) in scenarios {
            let mut state = preview_internal_state(
                flow,
                CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
            );

            assert!(prepare_offline_cloud_check_retry(&mut state));
            assert_eq!(state.cloud_restore_discovery, CloudRestoreDiscovery::Checking);
            assert_eq!(state.ui.step, expected_step);
            assert_eq!(state.ui.cloud_restore_state, OnboardingCloudRestoreState::Checking);

            state.flow.apply_event(
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(None)),
                &mut state.cloud_restore_discovery,
                state.restore_offer_allowed,
            );
            let mut deferred = DeferredSender::new(MessageSender::new(flume::bounded(16).0));
            state.sync_ui(&mut deferred);

            assert_eq!(state.ui.step, expected_step);
            assert!(state.ui.cloud_restore_alert_visible);
        }
    }

    #[test]
    fn offline_retry_ignores_non_offline_issues_and_late_states() {
        let mut cloud_unavailable = preview_internal_state(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::CloudUnavailable),
        );
        let mut late_state = preview_internal_state(
            FlowState::CreatingWallet(preview_created_wallet_flow(OnboardingBranch::NewUser)),
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
        );

        assert!(!prepare_offline_cloud_check_retry(&mut cloud_unavailable));
        assert_eq!(
            cloud_unavailable.cloud_restore_discovery,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::CloudUnavailable)
        );
        assert!(!prepare_offline_cloud_check_retry(&mut late_state));
        assert_eq!(
            late_state.cloud_restore_discovery,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline)
        );
        assert!(matches!(late_state.flow, FlowState::CreatingWallet(_)));
    }

    #[test]
    fn connectivity_reconnect_while_cloud_check_is_in_flight_retries_after_offline_finish() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = preview_manager(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::Checking,
        );
        manager.cloud_check_in_flight.store(true, Ordering::Release);

        manager.handle_connectivity_change(true);

        assert!(manager.pending_cloud_check_retry.load(Ordering::Acquire));
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::Checking);

        assert!(manager.finish_cloud_check_and_prepare_retry(
            CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline),
            true,
        ));
        assert!(!manager.cloud_check_in_flight.load(Ordering::Acquire));
        assert!(!manager.pending_cloud_check_retry.load(Ordering::Acquire));
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::Checking);
        assert_eq!(manager.state().cloud_restore_issue, None);
        assert_no_reconcile_messages(&manager);
    }

    #[test]
    fn late_pending_connectivity_retry_after_offline_finish_is_taken_over() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = preview_manager(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::Checking,
        );
        manager.cloud_check_in_flight.store(true, Ordering::Release);

        assert!(!manager.finish_cloud_check_and_prepare_retry(
            CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline),
            true,
        ));
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::Inconclusive);

        assert!(manager.mark_pending_cloud_check_retry());
        assert!(!manager.pending_cloud_check_retry.load(Ordering::Acquire));
        assert!(manager.prepare_offline_cloud_check_retry());
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::Checking);
        assert_eq!(manager.state().cloud_restore_issue, None);
    }

    #[test]
    fn restore_retry_after_offline_finish_skips_transient_offline_messages() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = preview_manager(
            FlowState::CloudCheck { origin: RestoreOrigin::Welcome },
            CloudRestoreDiscovery::Checking,
        );
        manager.cloud_check_in_flight.store(true, Ordering::Release);

        manager.handle_connectivity_change(true);

        assert!(manager.finish_cloud_check_and_prepare_retry(
            CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline),
            true,
        ));
        assert!(!manager.cloud_check_in_flight.load(Ordering::Acquire));
        assert!(!manager.pending_cloud_check_retry.load(Ordering::Acquire));
        assert_eq!(manager.state().step, OnboardingStep::CloudCheck);
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::Checking);
        assert_eq!(manager.state().cloud_restore_issue, None);
        assert_no_reconcile_messages(&manager);
    }

    #[test]
    fn connectivity_reconnect_while_cloud_check_is_in_flight_does_not_retry_non_offline_finish() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        let manager = preview_manager(
            FlowState::Welcome { error_message: None },
            CloudRestoreDiscovery::Checking,
        );
        manager.cloud_check_in_flight.store(true, Ordering::Release);

        manager.handle_connectivity_change(true);

        assert!(
            !manager
                .finish_cloud_check_and_prepare_retry(CloudCheckOutcome::NoBackupConfirmed, true,)
        );
        assert!(!manager.cloud_check_in_flight.load(Ordering::Acquire));
        assert!(!manager.pending_cloud_check_retry.load(Ordering::Acquire));
        assert_eq!(manager.state().cloud_restore_state, OnboardingCloudRestoreState::NoBackupFound);
    }

    #[test]
    fn cloud_check_timeout_is_treated_as_cloud_unavailable() {
        let error = CloudStorageError::NotAvailable("iCloud metadata query timed out".into());

        assert_eq!(CloudCheckIssue::from(error), CloudCheckIssue::CloudUnavailable);
    }

    #[test]
    fn cloud_drive_unavailable_is_treated_as_cloud_unavailable() {
        let error = CloudStorageError::NotAvailable("iCloud Drive is not available".into());

        assert_eq!(CloudCheckIssue::from(error), CloudCheckIssue::CloudUnavailable);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cloud_check_false_short_circuits_without_sleeping() {
        let slept = Arc::new(Mutex::new(Vec::new()));
        let sleep_log = Arc::clone(&slept);
        let outcome = determine_cloud_check_outcome(
            || async { Ok(CloudRestoreBackupSnapshot { has_backup: false, provider_hint: None }) },
            move |duration| {
                let sleep_log = Arc::clone(&sleep_log);
                async move {
                    sleep_log.lock().unwrap().push(duration);
                }
            },
        )
        .await;

        assert_eq!(outcome, CloudCheckOutcome::NoBackupConfirmed);
        assert!(slept.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cloud_check_retries_errors_and_returns_inconclusive() {
        let outcome = determine_cloud_check_outcome(
            || async { Err(CloudStorageError::NotAvailable("network timed out".into())) },
            |_| async {},
        )
        .await;

        assert_eq!(outcome, CloudCheckOutcome::Inconclusive(CloudCheckIssue::CloudUnavailable));
    }

    #[test]
    fn cloud_restore_download_error_tracking_ignores_not_found_before_harder_error() {
        let mut error = None;

        record_cloud_restore_download_error(
            &mut error,
            CloudStorageError::NotFound("old-namespace".into()),
        );
        record_cloud_restore_download_error(
            &mut error,
            CloudStorageError::NotAvailable("iCloud unavailable".into()),
        );

        assert_eq!(error, Some(CloudStorageError::NotAvailable("iCloud unavailable".into())));
    }

    #[test]
    fn restore_provider_hint_uses_known_provider_name_and_date() {
        let hint = resolve_provider_hint(&PasskeyProviderHint {
            aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
            registered_platform: BackupPasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_234,
            name_suffix: "09IX".into(),
        });

        assert_eq!(
            hint,
            CloudRestoreProviderHint {
                provider_name: Some("Google Password Manager".into()),
                registered_at: 1_777_661_234,
                name_suffix: "09IX".into(),
            }
        );
    }

    #[test]
    fn restore_provider_hint_preserves_unknown_provider_date() {
        let hint = resolve_provider_hint(&PasskeyProviderHint {
            aaguid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            registered_platform: BackupPasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_236,
            name_suffix: "09IY".into(),
        });

        assert_eq!(
            hint,
            CloudRestoreProviderHint {
                provider_name: None,
                registered_at: 1_777_661_236,
                name_suffix: "09IY".into(),
            }
        );
    }

    #[test]
    fn restore_provider_hint_selects_latest_hint() {
        let hint = choose_restore_provider_hint(vec![
            CloudRestoreProviderHint {
                provider_name: Some("Apple Passwords".into()),
                registered_at: 1_777_661_234,
                name_suffix: "09IX".into(),
            },
            CloudRestoreProviderHint {
                provider_name: None,
                registered_at: 1_777_661_236,
                name_suffix: "09IY".into(),
            },
        ])
        .expect("latest hint should be selected");

        assert_eq!(
            hint,
            CloudRestoreProviderHint {
                provider_name: None,
                registered_at: 1_777_661_236,
                name_suffix: "09IY".into(),
            }
        );
    }

    #[test]
    fn persisted_created_wallet_progress_resumes_to_backup_wallet() {
        let wallet_id = WalletId::new();
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("should parse mnemonic");
        let progress = OnboardingProgress::CreatedWallet {
            wallet_id: wallet_id.clone(),
            branch: OnboardingBranch::Exchange,
            network: Network::Bitcoin,
            wallet_mode: WalletMode::Main,
            secret_words_saved: true,
            cloud_backup_enabled: false,
        };

        let flow = progress
            .restore_flow(|requested_wallet_id, _, _| {
                assert_eq!(requested_wallet_id, &wallet_id);
                Some(mnemonic.clone())
            })
            .expect("should restore backup flow");

        match flow {
            FlowState::BackupWallet(flow) => {
                assert_eq!(flow.wallet_id, wallet_id);
                assert_eq!(flow.branch, OnboardingBranch::Exchange);
                assert!(flow.secret_words_saved);
                assert!(!flow.cloud_backup_enabled);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn stale_persisted_progress_falls_back_and_requests_clear() {
        let resolution = resolve_initial_flow(
            Some(OnboardingProgress::CreatedWallet {
                wallet_id: WalletId::new(),
                branch: OnboardingBranch::NewUser,
                network: Network::Bitcoin,
                wallet_mode: WalletMode::Main,
                secret_words_saved: false,
                cloud_backup_enabled: false,
            }),
            true,
            |_, _, _| None,
        );

        assert!(resolution.clear_persisted_progress);
        assert!(!resolution.start_cloud_check);
        assert!(matches!(
            resolution.flow,
            FlowState::Terms {
                context: TermsContext::SelectLatestOrNew,
                error_message: None,
                progress: None,
            }
        ));
    }

    #[test]
    fn completion_failure_sets_terms_error_without_completing() {
        let mut flow = FlowState::Terms {
            context: TermsContext::SelectWallet {
                wallet_id: WalletId::new(),
                post_onboarding: PostOnboardingDestination::None,
            },
            error_message: None,
            progress: None,
        };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CompletionFailed { error: "selection failed".into() },
            &mut discovery,
            false,
        );

        match flow {
            FlowState::Terms { error_message: Some(error), .. } => {
                assert_eq!(error, "selection failed")
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    fn preview_created_wallet_flow(branch: OnboardingBranch) -> CreatedWalletFlow {
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("should parse preview mnemonic");

        CreatedWalletFlow {
            branch,
            wallet_id: WalletId::new(),
            network: Network::Bitcoin,
            wallet_mode: WalletMode::Main,
            created_words: mnemonic.words().map(str::to_string).collect(),
            word_validator: Arc::new(WordValidator::new(mnemonic)),
            cloud_backup_enabled: false,
            secret_words_saved: false,
        }
    }

    fn preview_restore_report() -> CloudBackupRestoreReport {
        CloudBackupRestoreReport {
            wallets_restored: 1,
            wallets_failed: 0,
            failed_wallet_errors: Vec::new(),
            labels_failed_wallet_names: Vec::new(),
            labels_failed_errors: Vec::new(),
        }
    }

    fn preview_internal_state(
        flow: FlowState,
        cloud_restore_discovery: CloudRestoreDiscovery,
    ) -> InternalState {
        let restore_offer_allowed = true;
        let cloud_restore_alert_dismissed = false;
        let should_offer_cloud_restore =
            matches!(cloud_restore_discovery, CloudRestoreDiscovery::BackupFound(_));
        let cloud_restore_alert_visible = should_offer_cloud_restore
            && matches!(flow, FlowState::HardwareImport | FlowState::SoftwareImport { .. });
        let ui = flow.ui_state(
            &cloud_restore_discovery,
            should_offer_cloud_restore,
            cloud_restore_alert_visible,
        );

        InternalState {
            flow,
            cloud_restore_discovery,
            restore_offer_allowed,
            cloud_restore_alert_dismissed,
            next_restore_attempt_id: 1,
            ui,
        }
    }

    fn preview_manager(
        flow: FlowState,
        cloud_restore_discovery: CloudRestoreDiscovery,
    ) -> Arc<RustOnboardingManager> {
        crate::database::test_support::init_test_database();

        let (sender, receiver) = flume::bounded(16);

        Arc::new(RustOnboardingManager {
            state: Arc::new(RwLock::new(preview_internal_state(flow, cloud_restore_discovery))),
            cloud_check_in_flight: Arc::new(AtomicBool::new(false)),
            pending_cloud_check_retry: Arc::new(AtomicBool::new(false)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        })
    }

    fn assert_no_reconcile_messages(manager: &RustOnboardingManager) {
        assert!(matches!(manager.reconcile_receiver.try_recv(), Err(flume::TryRecvError::Empty)));
    }

    fn apply_action(flow: &mut FlowState, action: OnboardingAction) -> TransitionCommand {
        let mut restore_offer_allowed = false;
        flow.apply_user_action(
            action,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
            Some(1),
        )
    }

    fn assert_terms_select_wallet(
        flow: &FlowState,
        expected_wallet_id: &WalletId,
        expected_destination: PostOnboardingDestination,
    ) {
        match flow {
            FlowState::Terms {
                context: TermsContext::SelectWallet { wallet_id, post_onboarding },
                ..
            } => {
                assert_eq!(wallet_id, expected_wallet_id);
                assert_eq!(*post_onboarding, expected_destination);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    fn assert_restore_offer_back_origin(flow: FlowState, origin: RestoreOrigin) {
        match (flow, origin) {
            (FlowState::BitcoinChoice { error_message: None }, RestoreOrigin::BitcoinChoice)
            | (FlowState::StorageChoice { error_message: None }, RestoreOrigin::StorageChoice)
            | (FlowState::HardwareImport, RestoreOrigin::HardwareImport)
            | (FlowState::SoftwareImport { error_message: None }, RestoreOrigin::SoftwareImport) => {
            }
            (flow, origin) => {
                panic!("unexpected flow state after restore offer back: {flow:?} for {origin:?}")
            }
        }
    }

    fn prepare_offline_cloud_check_retry(state: &mut InternalState) -> bool {
        let (sender, _receiver) = flume::bounded(16);
        let mut deferred = DeferredSender::new(MessageSender::new(sender));
        state.prepare_offline_cloud_check_retry(&mut deferred)
    }
}

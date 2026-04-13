use std::{sync::Arc, thread, time::Duration};

use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_util::ResultExt as _;
use flume::Receiver;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    app::{App, AppAction, FfiApp},
    database::{Database, global_config::GlobalConfigKey},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    mnemonic::{Mnemonic as StoredMnemonic, MnemonicExt, NumberOfBip39Words},
    network::Network,
    pending_wallet::PendingWallet,
    router::{NewWalletRoute, Route},
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata, WalletMode},
    },
    word_validator::WordValidator,
};

use super::deferred_sender::{DeferredSender, MessageSender, SingleOrMany};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Default, uniffi::Enum)]
pub enum OnboardingStep {
    #[default]
    CloudCheck,
    RestoreOffer,
    Restoring,
    Welcome,
    BitcoinChoice,
    StorageChoice,
    SoftwareChoice,
    CreatingWallet,
    BackupWallet,
    CloudBackup,
    SecretWords,
    VerifyWords,
    ExchangeFunding,
    HardwareDeviceSelection,
    HardwareImport,
    SoftwareImport,
    Terms,
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

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum OnboardingSoftwareSelection {
    CreateNewWallet,
    ImportExistingWallet,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum OnboardingHardwareDevice {
    Coldcard,
    Ledger,
    Trezor,
    Other,
}

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub branch: Option<OnboardingBranch>,
    pub hardware_device: Option<OnboardingHardwareDevice>,
    pub created_words: Vec<String>,
    pub cloud_backup_enabled: bool,
    pub secret_words_saved: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum OnboardingAction {
    ContinueFromWelcome,
    SelectHasBitcoin { has_bitcoin: bool },
    SelectStorage { selection: OnboardingStorageSelection },
    SelectSoftwareAction { selection: OnboardingSoftwareSelection },
    ContinueWalletCreation,
    ShowSecretWords,
    SecretWordsSaved,
    OpenCloudBackup,
    CloudBackupEnabled,
    SkipCloudBackup,
    ContinueFromBackup,
    ContinueFromExchangeFunding,
    SelectHardwareDevice { device: OnboardingHardwareDevice },
    SoftwareImportCompleted { wallet_id: WalletId },
    HardwareImportCompleted { wallet_id: WalletId },
    BackupImportCompleted,
    StartRestore,
    SkipRestore,
    RestoreComplete,
    RestoreFailed { error: String },
    VerifyWordsCompleted,
    AcceptTerms,
    Back,
}

type Message = OnboardingReconcileMessage;

#[derive(Debug, Clone, uniffi::Enum)]
pub enum OnboardingReconcileMessage {
    Step(OnboardingStep),
    Branch(Option<OnboardingBranch>),
    HardwareDevice(Option<OnboardingHardwareDevice>),
    CreatedWords(Vec<String>),
    CloudBackupEnabled(bool),
    SecretWordsSaved(bool),
    ErrorMessageChanged(Option<String>),
    Complete,
}

#[uniffi::export(callback_interface)]
pub trait OnboardingManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: OnboardingReconcileMessage);
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum CompletionTarget {
    SelectLatestOrNew,
    NewWalletSelect,
    SelectWallet(WalletId),
}

#[derive(Debug, Clone)]
struct InitialFlowResolution {
    flow: FlowState,
    clear_persisted_progress: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CloudCheckOutcome {
    BackupFound,
    NoBackupConfirmed,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CloudCheckIssue {
    Offline,
    CloudUnavailable,
    Unknown,
}

#[derive(Debug, Clone)]
struct CreatedWalletFlow {
    branch: OnboardingBranch,
    wallet_id: WalletId,
    network: Network,
    wallet_mode: WalletMode,
    created_words: Vec<String>,
    word_validator: Arc<WordValidator>,
    cloud_backup_enabled: bool,
    secret_words_saved: bool,
}

#[derive(Debug, Clone)]
enum CloudBackupFlow {
    CreatedWallet(CreatedWalletFlow),
    SoftwareImport { wallet_id: WalletId },
}

#[derive(Debug, Clone)]
enum FlowState {
    CloudCheck,
    RestoreOffer {
        error_message: Option<String>,
    },
    Restoring,
    Welcome {
        error_message: Option<String>,
    },
    BitcoinChoice,
    StorageChoice,
    SoftwareChoice,
    CreatingWallet(CreatedWalletFlow),
    BackupWallet(CreatedWalletFlow),
    CloudBackup(CloudBackupFlow),
    SecretWords(CreatedWalletFlow),
    VerifyWords(CreatedWalletFlow),
    ExchangeFunding(CreatedWalletFlow),
    HardwareDeviceSelection {
        selected_device: Option<OnboardingHardwareDevice>,
    },
    HardwareImport {
        device: OnboardingHardwareDevice,
    },
    SoftwareImport,
    Terms {
        target: CompletionTarget,
        error_message: Option<String>,
        progress: Option<OnboardingProgress>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TransitionCommand {
    None,
    CreateWallet(OnboardingBranch),
    CompleteOnboarding(CompletionTarget),
}

#[derive(Debug, Clone)]
enum InternalEvent {
    CloudCheckFinished(CloudCheckOutcome),
    WalletCreated { flow: CreatedWalletFlow },
    WalletCreationFailed { branch: OnboardingBranch, error: String },
    CompletionFailed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum OnboardingProgress {
    CreatedWallet {
        wallet_id: WalletId,
        branch: OnboardingBranch,
        network: Network,
        wallet_mode: WalletMode,
        secret_words_saved: bool,
        cloud_backup_enabled: bool,
    },
}

#[derive(Debug, Clone)]
struct InternalState {
    flow: FlowState,
    ui: OnboardingState,
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustOnboardingManager {
    state: Arc<RwLock<InternalState>>,
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
        let should_start_cloud_check = matches!(&resolution.flow, FlowState::CloudCheck);

        if resolution.clear_persisted_progress {
            Self::sync_onboarding_progress(None);
        }

        let manager = Arc::new(Self {
            state: Arc::new(RwLock::new(InternalState::new(resolution.flow))),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

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
            let command = state.flow.apply_user_action(action);
            state.sync_ui(deferred);
            command
        });
        self.run_command(command);
    }
}

impl RustOnboardingManager {
    fn start_cloud_check(self: &Arc<Self>) {
        let me = Arc::clone(self);
        thread::spawn(move || {
            let retry_delays = [1u64, 2, 2, 3, 5, 10];
            let cloud = CloudStorage::global().clone();
            let outcome = determine_cloud_check_outcome(
                &retry_delays,
                || cloud.has_any_cloud_backup(),
                thread::sleep,
            );
            me.finish_cloud_check(outcome);
        });
    }

    fn finish_cloud_check(&self, outcome: CloudCheckOutcome) {
        self.apply_event(InternalEvent::CloudCheckFinished(outcome));
    }

    fn apply_event(&self, event: InternalEvent) {
        self.mutate_state(|state, deferred| {
            state.flow.apply_event(event);
            state.sync_ui(deferred);
        });
    }

    fn run_command(&self, command: TransitionCommand) {
        match command {
            TransitionCommand::None => {}
            TransitionCommand::CreateWallet(branch) => self.create_wallet_for_branch(branch),
            TransitionCommand::CompleteOnboarding(target) => self.complete_onboarding(target),
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
                FfiApp::global().select_latest_or_new_wallet();
                Ok(())
            }
            CompletionTarget::NewWalletSelect => {
                FfiApp::global()
                    .load_and_reset_default_route(Route::NewWallet(NewWalletRoute::default()));
                Ok(())
            }
            CompletionTarget::SelectWallet(wallet_id) => {
                FfiApp::global().select_wallet(wallet_id, None).map_err(|error| error.to_string())
            }
        };

        match result {
            Ok(()) => {
                App::global().handle_action(AppAction::AcceptTerms);
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
        let mut deferred = DeferredSender::new(self.reconciler.clone());
        let (result, progress) = {
            let mut state = self.state.write();
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
        let wallet_metadata = WalletMetadata::new(name, Some(fingerprint));
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

    fn load_onboarding_progress() -> Option<OnboardingProgress> {
        let config = &Database::global().global_config;
        let raw = match config.get(GlobalConfigKey::OnboardingProgress) {
            Ok(Some(raw)) => raw,
            Ok(None) => return None,
            Err(error) => {
                warn!("Onboarding: failed to load persisted onboarding progress: {error}");
                return None;
            }
        };

        match serde_json::from_str(&raw) {
            Ok(progress) => Some(progress),
            Err(error) => {
                warn!("Onboarding: invalid persisted onboarding progress: {error}");
                if let Err(delete_error) = config.delete(GlobalConfigKey::OnboardingProgress) {
                    warn!(
                        "Onboarding: failed to clear invalid onboarding progress: {delete_error}"
                    );
                }
                None
            }
        }
    }

    fn sync_onboarding_progress(progress: Option<OnboardingProgress>) {
        let config = &Database::global().global_config;
        let current = config.get(GlobalConfigKey::OnboardingProgress).ok().flatten();

        match progress {
            Some(progress) => match serde_json::to_string(&progress) {
                Ok(serialized) => {
                    if current.as_deref() == Some(serialized.as_str()) {
                        return;
                    }
                    if let Err(error) = config.set(GlobalConfigKey::OnboardingProgress, serialized)
                    {
                        warn!("Onboarding: failed to persist onboarding progress: {error}");
                    }
                }
                Err(error) => warn!("Onboarding: failed to encode onboarding progress: {error}"),
            },
            None => {
                if current.is_none() {
                    return;
                }
                if let Err(error) = config.delete(GlobalConfigKey::OnboardingProgress) {
                    warn!("Onboarding: failed to clear onboarding progress: {error}");
                }
            }
        }
    }
}

impl InternalState {
    fn new(flow: FlowState) -> Self {
        let ui = flow.ui_state();
        Self { flow, ui }
    }

    fn sync_ui(&mut self, deferred: &mut DeferredSender<Message>) {
        let next_ui = self.flow.ui_state();

        if self.ui.branch != next_ui.branch {
            deferred.queue(Message::Branch(next_ui.branch));
        }
        if self.ui.hardware_device != next_ui.hardware_device {
            deferred.queue(Message::HardwareDevice(next_ui.hardware_device));
        }
        if self.ui.created_words != next_ui.created_words {
            deferred.queue(Message::CreatedWords(next_ui.created_words.clone()));
        }
        if self.ui.cloud_backup_enabled != next_ui.cloud_backup_enabled {
            deferred.queue(Message::CloudBackupEnabled(next_ui.cloud_backup_enabled));
        }
        if self.ui.secret_words_saved != next_ui.secret_words_saved {
            deferred.queue(Message::SecretWordsSaved(next_ui.secret_words_saved));
        }
        if self.ui.error_message != next_ui.error_message {
            deferred.queue(Message::ErrorMessageChanged(next_ui.error_message.clone()));
        }
        if self.ui.step != next_ui.step {
            deferred.queue(Message::Step(next_ui.step));
        }

        self.ui = next_ui;
    }
}

impl FlowState {
    fn apply_user_action(&mut self, action: OnboardingAction) -> TransitionCommand {
        let current = std::mem::replace(self, Self::CloudCheck);

        let (next, command) = match (current, action) {
            (Self::Welcome { .. }, OnboardingAction::ContinueFromWelcome) => {
                (Self::BitcoinChoice, TransitionCommand::None)
            }
            (Self::BitcoinChoice, OnboardingAction::SelectHasBitcoin { has_bitcoin: true }) => {
                (Self::StorageChoice, TransitionCommand::None)
            }
            (Self::BitcoinChoice, OnboardingAction::SelectHasBitcoin { has_bitcoin: false }) => {
                (Self::BitcoinChoice, TransitionCommand::CreateWallet(OnboardingBranch::NewUser))
            }
            (
                Self::StorageChoice,
                OnboardingAction::SelectStorage { selection: OnboardingStorageSelection::Exchange },
            ) => (Self::StorageChoice, TransitionCommand::CreateWallet(OnboardingBranch::Exchange)),
            (
                Self::StorageChoice,
                OnboardingAction::SelectStorage {
                    selection: OnboardingStorageSelection::HardwareWallet,
                },
            ) => (Self::HardwareDeviceSelection { selected_device: None }, TransitionCommand::None),
            (
                Self::StorageChoice,
                OnboardingAction::SelectStorage {
                    selection: OnboardingStorageSelection::SoftwareWallet,
                },
            ) => (Self::SoftwareChoice, TransitionCommand::None),
            (
                Self::SoftwareChoice,
                OnboardingAction::SelectSoftwareAction {
                    selection: OnboardingSoftwareSelection::CreateNewWallet,
                },
            ) => (
                Self::SoftwareChoice,
                TransitionCommand::CreateWallet(OnboardingBranch::SoftwareCreate),
            ),
            (
                Self::SoftwareChoice,
                OnboardingAction::SelectSoftwareAction {
                    selection: OnboardingSoftwareSelection::ImportExistingWallet,
                },
            ) => (Self::SoftwareImport, TransitionCommand::None),
            (Self::CreatingWallet(flow), OnboardingAction::ContinueWalletCreation) => {
                (Self::BackupWallet(flow), TransitionCommand::None)
            }
            (Self::BackupWallet(flow), OnboardingAction::ShowSecretWords) => {
                (Self::SecretWords(flow), TransitionCommand::None)
            }
            (Self::SecretWords(mut flow), OnboardingAction::SecretWordsSaved) => {
                flow.secret_words_saved = true;
                (Self::BackupWallet(flow), TransitionCommand::None)
            }
            (Self::BackupWallet(flow), OnboardingAction::OpenCloudBackup) => {
                (Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)), TransitionCommand::None)
            }
            (
                Self::CloudBackup(CloudBackupFlow::CreatedWallet(mut flow)),
                OnboardingAction::CloudBackupEnabled,
            ) => {
                flow.cloud_backup_enabled = true;
                (Self::BackupWallet(flow), TransitionCommand::None)
            }
            (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                OnboardingAction::CloudBackupEnabled,
            ) => (
                Self::Terms {
                    target: CompletionTarget::SelectWallet(wallet_id),
                    error_message: None,
                    progress: None,
                },
                TransitionCommand::None,
            ),
            (
                Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)),
                OnboardingAction::SkipCloudBackup,
            ) => (Self::BackupWallet(flow), TransitionCommand::None),
            (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                OnboardingAction::SkipCloudBackup,
            ) => (
                Self::Terms {
                    target: CompletionTarget::SelectWallet(wallet_id),
                    error_message: None,
                    progress: None,
                },
                TransitionCommand::None,
            ),
            (Self::BackupWallet(flow), OnboardingAction::ContinueFromBackup)
                if flow.secret_words_saved || flow.cloud_backup_enabled =>
            {
                if flow.branch == OnboardingBranch::Exchange {
                    (Self::ExchangeFunding(flow), TransitionCommand::None)
                } else if flow.cloud_backup_enabled {
                    (
                        Self::Terms {
                            target: CompletionTarget::SelectWallet(flow.wallet_id.clone()),
                            error_message: None,
                            progress: Some(OnboardingProgress::from(&flow)),
                        },
                        TransitionCommand::None,
                    )
                } else {
                    (Self::VerifyWords(flow), TransitionCommand::None)
                }
            }
            (Self::ExchangeFunding(flow), OnboardingAction::ContinueFromExchangeFunding) => {
                if flow.cloud_backup_enabled {
                    (
                        Self::Terms {
                            target: CompletionTarget::SelectWallet(flow.wallet_id.clone()),
                            error_message: None,
                            progress: Some(OnboardingProgress::from(&flow)),
                        },
                        TransitionCommand::None,
                    )
                } else {
                    (Self::VerifyWords(flow), TransitionCommand::None)
                }
            }
            (
                Self::HardwareDeviceSelection { .. },
                OnboardingAction::SelectHardwareDevice { device },
            ) => (Self::HardwareImport { device }, TransitionCommand::None),
            (Self::SoftwareImport, OnboardingAction::SoftwareImportCompleted { wallet_id }) => {
                Self::software_import_completed(wallet_id)
            }
            (
                Self::HardwareImport { .. },
                OnboardingAction::HardwareImportCompleted { wallet_id },
            ) => (
                Self::Terms {
                    target: CompletionTarget::SelectWallet(wallet_id),
                    error_message: None,
                    progress: None,
                },
                TransitionCommand::None,
            ),
            (Self::SoftwareImport, OnboardingAction::BackupImportCompleted) => (
                Self::Terms {
                    target: CompletionTarget::SelectLatestOrNew,
                    error_message: None,
                    progress: None,
                },
                TransitionCommand::None,
            ),
            (Self::RestoreOffer { .. }, OnboardingAction::StartRestore) => {
                (Self::Restoring, TransitionCommand::None)
            }
            (Self::RestoreOffer { error_message }, OnboardingAction::SkipRestore) => {
                Self::restore_check_exit(
                    Self::RestoreOffer { error_message },
                    CompletionTarget::NewWalletSelect,
                )
            }
            (Self::Restoring, OnboardingAction::RestoreComplete) => {
                Self::restore_check_exit(Self::Restoring, CompletionTarget::SelectLatestOrNew)
            }
            (Self::Restoring, OnboardingAction::RestoreFailed { error }) => {
                (Self::RestoreOffer { error_message: Some(error) }, TransitionCommand::None)
            }
            (Self::VerifyWords(flow), OnboardingAction::VerifyWordsCompleted) => (
                Self::Terms {
                    target: CompletionTarget::SelectWallet(flow.wallet_id.clone()),
                    error_message: None,
                    progress: Some(OnboardingProgress::from(&flow)),
                },
                TransitionCommand::None,
            ),
            (Self::Terms { target, progress, .. }, OnboardingAction::AcceptTerms) => {
                let command = TransitionCommand::CompleteOnboarding(target.clone());
                (Self::Terms { target, error_message: None, progress }, command)
            }
            (Self::BitcoinChoice, OnboardingAction::Back) => {
                (Self::Welcome { error_message: None }, TransitionCommand::None)
            }
            (Self::StorageChoice, OnboardingAction::Back) => {
                (Self::BitcoinChoice, TransitionCommand::None)
            }
            (Self::SoftwareChoice, OnboardingAction::Back) => {
                (Self::StorageChoice, TransitionCommand::None)
            }
            (Self::SoftwareImport, OnboardingAction::Back) => {
                (Self::SoftwareChoice, TransitionCommand::None)
            }
            (Self::HardwareDeviceSelection { .. }, OnboardingAction::Back) => {
                (Self::StorageChoice, TransitionCommand::None)
            }
            (Self::HardwareImport { device }, OnboardingAction::Back) => (
                Self::HardwareDeviceSelection { selected_device: Some(device) },
                TransitionCommand::None,
            ),
            (Self::SecretWords(flow), OnboardingAction::Back) => {
                (Self::BackupWallet(flow), TransitionCommand::None)
            }
            (Self::ExchangeFunding(flow), OnboardingAction::Back) => {
                (Self::BackupWallet(flow), TransitionCommand::None)
            }
            (state, action) => {
                warn!("Onboarding: invalid action={action:?} flow={state:?}");
                (state, TransitionCommand::None)
            }
        };

        *self = next;
        command
    }

    fn software_import_completed(wallet_id: WalletId) -> (Self, TransitionCommand) {
        software_import_transition(wallet_id, cloud_backup_is_configured_for_import())
    }

    fn restore_check_exit(
        fallback_state: Self,
        target: CompletionTarget,
    ) -> (Self, TransitionCommand) {
        restore_check_transition(fallback_state, target, terms_are_accepted_for_restore_check())
    }

    fn apply_event(&mut self, event: InternalEvent) {
        let current = std::mem::replace(self, Self::CloudCheck);

        let next = match (current, event) {
            (
                Self::CloudCheck,
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            ) => Self::RestoreOffer { error_message: None },
            (
                Self::CloudCheck,
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::NoBackupConfirmed),
            ) => Self::Welcome { error_message: None },
            (
                Self::CloudCheck,
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(issue)),
            ) => {
                Self::RestoreOffer { error_message: Some(cloud_check_inconclusive_message(issue)) }
            }
            (Self::BitcoinChoice, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::NewUser =>
            {
                Self::CreatingWallet(flow)
            }
            (Self::StorageChoice, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::Exchange =>
            {
                Self::CreatingWallet(flow)
            }
            (Self::SoftwareChoice, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::SoftwareCreate =>
            {
                Self::CreatingWallet(flow)
            }
            (
                Self::BitcoinChoice,
                InternalEvent::WalletCreationFailed { branch: OnboardingBranch::NewUser, error },
            )
            | (
                Self::StorageChoice,
                InternalEvent::WalletCreationFailed { branch: OnboardingBranch::Exchange, error },
            )
            | (
                Self::SoftwareChoice,
                InternalEvent::WalletCreationFailed {
                    branch: OnboardingBranch::SoftwareCreate,
                    error,
                },
            ) => Self::Welcome { error_message: Some(error) },
            (Self::Terms { target, progress, .. }, InternalEvent::CompletionFailed { error }) => {
                Self::Terms { target, error_message: Some(error), progress }
            }
            (state, event) => {
                warn!("Onboarding: invalid event={event:?} flow={state:?}");
                state
            }
        };

        *self = next;
    }

    fn ui_state(&self) -> OnboardingState {
        match self {
            Self::CloudCheck => {
                OnboardingState { step: OnboardingStep::CloudCheck, ..OnboardingState::default() }
            }
            Self::RestoreOffer { error_message } => OnboardingState {
                step: OnboardingStep::RestoreOffer,
                error_message: error_message.clone(),
                ..OnboardingState::default()
            },
            Self::Restoring => {
                OnboardingState { step: OnboardingStep::Restoring, ..OnboardingState::default() }
            }
            Self::Welcome { error_message } => OnboardingState {
                step: OnboardingStep::Welcome,
                error_message: error_message.clone(),
                ..OnboardingState::default()
            },
            Self::BitcoinChoice => OnboardingState {
                step: OnboardingStep::BitcoinChoice,
                ..OnboardingState::default()
            },
            Self::StorageChoice => OnboardingState {
                step: OnboardingStep::StorageChoice,
                ..OnboardingState::default()
            },
            Self::SoftwareChoice => OnboardingState {
                step: OnboardingStep::SoftwareChoice,
                ..OnboardingState::default()
            },
            Self::CreatingWallet(flow) => {
                Self::project_created_wallet(OnboardingStep::CreatingWallet, flow)
            }
            Self::BackupWallet(flow) => {
                Self::project_created_wallet(OnboardingStep::BackupWallet, flow)
            }
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => {
                Self::project_created_wallet(OnboardingStep::CloudBackup, flow)
            }
            Self::CloudBackup(CloudBackupFlow::SoftwareImport { .. }) => OnboardingState {
                step: OnboardingStep::CloudBackup,
                branch: Some(OnboardingBranch::SoftwareImport),
                ..OnboardingState::default()
            },
            Self::SecretWords(flow) => {
                Self::project_created_wallet(OnboardingStep::SecretWords, flow)
            }
            Self::VerifyWords(flow) => {
                Self::project_created_wallet(OnboardingStep::VerifyWords, flow)
            }
            Self::ExchangeFunding(flow) => {
                Self::project_created_wallet(OnboardingStep::ExchangeFunding, flow)
            }
            Self::HardwareDeviceSelection { selected_device } => OnboardingState {
                step: OnboardingStep::HardwareDeviceSelection,
                branch: Some(OnboardingBranch::Hardware),
                hardware_device: *selected_device,
                ..OnboardingState::default()
            },
            Self::HardwareImport { device } => OnboardingState {
                step: OnboardingStep::HardwareImport,
                branch: Some(OnboardingBranch::Hardware),
                hardware_device: Some(*device),
                ..OnboardingState::default()
            },
            Self::SoftwareImport => OnboardingState {
                step: OnboardingStep::SoftwareImport,
                branch: Some(OnboardingBranch::SoftwareImport),
                ..OnboardingState::default()
            },
            Self::Terms { error_message, .. } => OnboardingState {
                step: OnboardingStep::Terms,
                error_message: error_message.clone(),
                ..OnboardingState::default()
            },
        }
    }

    fn current_wallet_id(&self) -> Option<WalletId> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::SecretWords(flow)
            | Self::VerifyWords(flow)
            | Self::ExchangeFunding(flow) => Some(flow.wallet_id.clone()),
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => Some(flow.wallet_id.clone()),
            Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id })
            | Self::Terms { target: CompletionTarget::SelectWallet(wallet_id), .. } => {
                Some(wallet_id.clone())
            }
            _ => None,
        }
    }

    fn word_validator(&self) -> Option<Arc<WordValidator>> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow))
            | Self::SecretWords(flow)
            | Self::VerifyWords(flow)
            | Self::ExchangeFunding(flow) => Some(flow.word_validator.clone()),
            _ => None,
        }
    }

    fn project_created_wallet(step: OnboardingStep, flow: &CreatedWalletFlow) -> OnboardingState {
        OnboardingState {
            step,
            branch: Some(flow.branch),
            hardware_device: None,
            created_words: flow.created_words.clone(),
            cloud_backup_enabled: flow.cloud_backup_enabled,
            secret_words_saved: flow.secret_words_saved,
            error_message: None,
        }
    }

    fn persisted_progress(&self) -> Option<OnboardingProgress> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow))
            | Self::SecretWords(flow)
            | Self::VerifyWords(flow)
            | Self::ExchangeFunding(flow) => Some(OnboardingProgress::from(flow)),
            Self::Terms { target: CompletionTarget::SelectWallet(_), progress, .. } => {
                progress.clone()
            }
            _ => None,
        }
    }
}

impl From<&CreatedWalletFlow> for OnboardingProgress {
    fn from(flow: &CreatedWalletFlow) -> Self {
        Self::CreatedWallet {
            wallet_id: flow.wallet_id.clone(),
            branch: flow.branch,
            network: flow.network,
            wallet_mode: flow.wallet_mode,
            secret_words_saved: flow.secret_words_saved,
            cloud_backup_enabled: flow.cloud_backup_enabled,
        }
    }
}

impl OnboardingProgress {
    fn restore_flow<F>(&self, load_mnemonic: F) -> Option<FlowState>
    where
        F: FnOnce(&WalletId, Network, WalletMode) -> Option<bip39::Mnemonic>,
    {
        match self {
            Self::CreatedWallet {
                wallet_id,
                branch,
                network,
                wallet_mode,
                secret_words_saved,
                cloud_backup_enabled,
            } => {
                let mnemonic = load_mnemonic(wallet_id, *network, *wallet_mode)?;
                let created_words = mnemonic.words().map(str::to_string).collect();

                Some(FlowState::BackupWallet(CreatedWalletFlow {
                    branch: *branch,
                    wallet_id: wallet_id.clone(),
                    network: *network,
                    wallet_mode: *wallet_mode,
                    created_words,
                    word_validator: Arc::new(WordValidator::new(mnemonic)),
                    cloud_backup_enabled: *cloud_backup_enabled,
                    secret_words_saved: *secret_words_saved,
                }))
            }
        }
    }
}

fn software_import_transition(
    wallet_id: WalletId,
    cloud_backup_configured: bool,
) -> (FlowState, TransitionCommand) {
    if cloud_backup_configured {
        (
            FlowState::Terms {
                target: CompletionTarget::SelectWallet(wallet_id),
                error_message: None,
                progress: None,
            },
            TransitionCommand::None,
        )
    } else {
        (
            FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
            TransitionCommand::None,
        )
    }
}

fn restore_check_transition(
    fallback_state: FlowState,
    target: CompletionTarget,
    terms_accepted: bool,
) -> (FlowState, TransitionCommand) {
    if terms_accepted {
        (fallback_state, TransitionCommand::CompleteOnboarding(target))
    } else {
        (FlowState::Terms { target, error_message: None, progress: None }, TransitionCommand::None)
    }
}

fn cloud_backup_is_configured_for_import() -> bool {
    match Database::global().cloud_backup_state.get() {
        Ok(state) => state.is_configured(),
        Err(error) => {
            warn!("Onboarding: failed to load cloud backup state after software import: {error}");
            false
        }
    }
}

fn terms_are_accepted_for_restore_check() -> bool {
    Database::global().global_flag.is_terms_accepted()
}

fn default_initial_flow(has_wallets: bool) -> FlowState {
    if has_wallets {
        FlowState::Terms {
            target: CompletionTarget::SelectLatestOrNew,
            error_message: None,
            progress: None,
        }
    } else {
        FlowState::CloudCheck
    }
}

fn resolve_initial_flow<F>(
    progress: Option<OnboardingProgress>,
    has_wallets: bool,
    load_mnemonic: F,
) -> InitialFlowResolution
where
    F: FnOnce(&WalletId, Network, WalletMode) -> Option<bip39::Mnemonic>,
{
    match progress {
        Some(progress) => match progress.restore_flow(load_mnemonic) {
            Some(flow) => InitialFlowResolution { flow, clear_persisted_progress: false },
            None => InitialFlowResolution {
                flow: default_initial_flow(has_wallets),
                clear_persisted_progress: true,
            },
        },
        None => InitialFlowResolution {
            flow: default_initial_flow(has_wallets),
            clear_persisted_progress: false,
        },
    }
}

fn classify_cloud_check_error(error: &CloudStorageError) -> CloudCheckIssue {
    let message = error.to_string().to_ascii_lowercase();

    if message.contains("timed out") || message.contains("offline") || message.contains("network") {
        return CloudCheckIssue::Offline;
    }

    match error {
        CloudStorageError::NotAvailable(_) => CloudCheckIssue::CloudUnavailable,
        CloudStorageError::UploadFailed(_)
        | CloudStorageError::DownloadFailed(_)
        | CloudStorageError::NotFound(_)
        | CloudStorageError::QuotaExceeded => CloudCheckIssue::Unknown,
    }
}

fn cloud_check_inconclusive_message(issue: CloudCheckIssue) -> String {
    match issue {
        CloudCheckIssue::Offline => {
            "You may be offline. Connect to the internet and try again, or you can still try restoring with your passkey if you're reinstalling this device.".into()
        }
        CloudCheckIssue::CloudUnavailable => {
            "We couldn't confirm whether an iCloud backup is available because iCloud may be unavailable. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
        CloudCheckIssue::Unknown => {
            "We couldn't confirm whether an iCloud backup is available. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
    }
}

fn determine_cloud_check_outcome<F, S>(
    retry_delays: &[u64],
    mut has_any_cloud_backup: F,
    mut sleep: S,
) -> CloudCheckOutcome
where
    F: FnMut() -> Result<bool, CloudStorageError>,
    S: FnMut(Duration),
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        info!(
            "Onboarding: checking cloud backup attempt={}/{}",
            attempt + 1,
            retry_delays.len() + 1
        );

        match has_any_cloud_backup() {
            Ok(true) => return CloudCheckOutcome::BackupFound,
            Ok(false) => return CloudCheckOutcome::NoBackupConfirmed,
            Err(error) => warn!("Onboarding: cloud backup check failed: {error}"),
        }

        sleep(Duration::from_secs(*delay));
    }

    match has_any_cloud_backup() {
        Ok(true) => CloudCheckOutcome::BackupFound,
        Ok(false) => CloudCheckOutcome::NoBackupConfirmed,
        Err(error) => {
            warn!("Onboarding: final cloud backup check failed: {error}");
            CloudCheckOutcome::Inconclusive(classify_cloud_check_error(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use bip39::Mnemonic;

    use super::*;

    #[test]
    fn continue_from_backup_requires_a_saved_backup_method() {
        let mut flow =
            FlowState::BackupWallet(preview_created_wallet_flow(OnboardingBranch::NewUser));

        let command = flow.apply_user_action(OnboardingAction::ContinueFromBackup);

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::BackupWallet(_)));
    }

    #[test]
    fn enabling_cloud_backup_after_software_import_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });

        let command = flow.apply_user_action(OnboardingAction::CloudBackupEnabled);

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms { target: CompletionTarget::SelectWallet(id), .. } => {
                assert_eq!(id, wallet_id)
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn software_import_with_disabled_cloud_backup_enters_enable_step() {
        let wallet_id = WalletId::new();

        let (flow, command) = software_import_transition(wallet_id.clone(), false);

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id: id }) => {
                assert_eq!(id, wallet_id);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn software_import_with_configured_cloud_backup_bypasses_enable_step() {
        let wallet_id = WalletId::new();

        let (flow, command) = software_import_transition(wallet_id.clone(), true);

        assert_eq!(command, TransitionCommand::None);
        match flow {
            FlowState::Terms { target: CompletionTarget::SelectWallet(id), .. } => {
                assert_eq!(id, wallet_id);
            }
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn restore_complete_with_accepted_terms_completes_immediately() {
        let (flow, command) = restore_check_transition(
            FlowState::Restoring,
            CompletionTarget::SelectLatestOrNew,
            true,
        );

        assert_eq!(
            command,
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectLatestOrNew)
        );
        assert!(matches!(flow, FlowState::Restoring));
    }

    #[test]
    fn skip_restore_with_accepted_terms_completes_immediately() {
        let (flow, command) = restore_check_transition(
            FlowState::RestoreOffer { error_message: None },
            CompletionTarget::NewWalletSelect,
            true,
        );

        assert_eq!(
            command,
            TransitionCommand::CompleteOnboarding(CompletionTarget::NewWalletSelect)
        );
        assert!(matches!(flow, FlowState::RestoreOffer { error_message: None }));
    }

    #[test]
    fn restore_complete_without_accepted_terms_shows_terms() {
        let (flow, command) = restore_check_transition(
            FlowState::Restoring,
            CompletionTarget::SelectLatestOrNew,
            false,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::Terms {
                target: CompletionTarget::SelectLatestOrNew,
                error_message: None,
                progress: None,
            }
        ));
    }

    #[test]
    fn skip_restore_without_accepted_terms_shows_terms() {
        let (flow, command) = restore_check_transition(
            FlowState::RestoreOffer { error_message: None },
            CompletionTarget::NewWalletSelect,
            false,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::Terms {
                target: CompletionTarget::NewWalletSelect,
                error_message: None,
                progress: None,
            }
        ));
    }

    #[test]
    fn hardware_back_preserves_selected_device() {
        let mut flow = FlowState::HardwareImport { device: OnboardingHardwareDevice::Ledger };

        flow.apply_user_action(OnboardingAction::Back);

        match flow {
            FlowState::HardwareDeviceSelection {
                selected_device: Some(OnboardingHardwareDevice::Ledger),
            } => {}
            other => panic!("unexpected flow state: {other:?}"),
        }
    }

    #[test]
    fn cloud_check_timeout_is_treated_as_offline() {
        let error = CloudStorageError::NotAvailable("iCloud metadata query timed out".into());

        assert_eq!(classify_cloud_check_error(&error), CloudCheckIssue::Offline);
    }

    #[test]
    fn cloud_drive_unavailable_is_treated_as_cloud_unavailable() {
        let error = CloudStorageError::NotAvailable("iCloud Drive is not available".into());

        assert_eq!(classify_cloud_check_error(&error), CloudCheckIssue::CloudUnavailable);
    }

    #[test]
    fn cloud_check_false_short_circuits_without_sleeping() {
        let mut slept = Vec::new();
        let outcome = determine_cloud_check_outcome(
            &[1, 2, 3],
            || Ok(false),
            |duration| slept.push(duration),
        );

        assert_eq!(outcome, CloudCheckOutcome::NoBackupConfirmed);
        assert!(slept.is_empty());
    }

    #[test]
    fn cloud_check_retries_errors_and_returns_inconclusive() {
        let mut slept = Vec::new();
        let outcome = determine_cloud_check_outcome(
            &[1, 2],
            || Err(CloudStorageError::NotAvailable("network timed out".into())),
            |duration| slept.push(duration),
        );

        assert_eq!(outcome, CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline));
        assert_eq!(slept, vec![Duration::from_secs(1), Duration::from_secs(2)]);
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
        assert!(matches!(
            resolution.flow,
            FlowState::Terms {
                target: CompletionTarget::SelectLatestOrNew,
                error_message: None,
                progress: None,
            }
        ));
    }

    #[test]
    fn completion_failure_sets_terms_error_without_completing() {
        let mut flow = FlowState::Terms {
            target: CompletionTarget::SelectWallet(WalletId::new()),
            error_message: None,
            progress: None,
        };

        flow.apply_event(InternalEvent::CompletionFailed { error: "selection failed".into() });

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
}

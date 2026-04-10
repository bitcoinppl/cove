use std::{sync::Arc, thread, time::Duration};

use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use flume::Receiver;
use parking_lot::RwLock;
use tracing::{info, warn};

use crate::{
    app::{App, AppAction, FfiApp},
    database::Database,
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    mnemonic::{MnemonicExt, NumberOfBip39Words},
    pending_wallet::PendingWallet,
    router::{NewWalletRoute, Route},
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata},
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

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
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
    RestoreOffer { error_message: Option<String> },
    Restoring,
    Welcome { error_message: Option<String> },
    BitcoinChoice,
    StorageChoice,
    SoftwareChoice,
    CreatingWallet(CreatedWalletFlow),
    BackupWallet(CreatedWalletFlow),
    CloudBackup(CloudBackupFlow),
    SecretWords(CreatedWalletFlow),
    VerifyWords(CreatedWalletFlow),
    ExchangeFunding(CreatedWalletFlow),
    HardwareDeviceSelection { selected_device: Option<OnboardingHardwareDevice> },
    HardwareImport { device: OnboardingHardwareDevice },
    SoftwareImport,
    Terms(CompletionTarget),
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
        let initial_flow = if has_wallets {
            FlowState::Terms(CompletionTarget::SelectLatestOrNew)
        } else {
            FlowState::CloudCheck
        };

        let manager = Arc::new(Self {
            state: Arc::new(RwLock::new(InternalState::new(initial_flow))),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

        if !has_wallets {
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

            for (attempt, delay) in retry_delays.iter().enumerate() {
                info!(
                    "Onboarding: checking cloud backup attempt={}/{}",
                    attempt + 1,
                    retry_delays.len() + 1
                );

                match cloud.has_any_cloud_backup() {
                    Ok(true) => {
                        me.finish_cloud_check(CloudCheckOutcome::BackupFound);
                        return;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        warn!("Onboarding: cloud backup check failed: {error}");
                    }
                }

                thread::sleep(Duration::from_secs(*delay));
            }

            let outcome = match cloud.has_any_cloud_backup() {
                Ok(true) => CloudCheckOutcome::BackupFound,
                Ok(false) => CloudCheckOutcome::NoBackupConfirmed,
                Err(error) => {
                    warn!("Onboarding: final cloud backup check failed: {error}");
                    CloudCheckOutcome::Inconclusive(classify_cloud_check_error(&error))
                }
            };
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
        App::global().handle_action(AppAction::AcceptTerms);

        match target {
            CompletionTarget::SelectLatestOrNew => {
                FfiApp::global().select_latest_or_new_wallet();
            }
            CompletionTarget::NewWalletSelect => {
                FfiApp::global()
                    .load_and_reset_default_route(Route::NewWallet(NewWalletRoute::default()));
            }
            CompletionTarget::SelectWallet(wallet_id) => {
                let _ = FfiApp::global().select_wallet(wallet_id, None);
            }
        }

        self.send(Message::Complete);
    }

    fn mutate_state<F, R>(&self, mutate: F) -> R
    where
        F: FnOnce(&mut InternalState, &mut DeferredSender<Message>) -> R,
    {
        let mut deferred = DeferredSender::new(self.reconciler.clone());
        {
            let mut state = self.state.write();
            mutate(&mut state, &mut deferred)
        }
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
                .map_err(|error| error.to_string())?;
        CLOUD_BACKUP_MANAGER.mark_verification_required_after_wallet_change();

        Ok(CreatedWalletFlow {
            branch,
            wallet_id: wallet.metadata.id,
            created_words: words,
            word_validator: Arc::new(WordValidator::new(mnemonic)),
            cloud_backup_enabled: false,
            secret_words_saved: false,
        })
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
            ) => (Self::Terms(CompletionTarget::SelectWallet(wallet_id)), TransitionCommand::None),
            (
                Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)),
                OnboardingAction::SkipCloudBackup,
            ) => (Self::BackupWallet(flow), TransitionCommand::None),
            (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                OnboardingAction::SkipCloudBackup,
            ) => (Self::Terms(CompletionTarget::SelectWallet(wallet_id)), TransitionCommand::None),
            (Self::BackupWallet(flow), OnboardingAction::ContinueFromBackup)
                if flow.secret_words_saved || flow.cloud_backup_enabled =>
            {
                if flow.branch == OnboardingBranch::Exchange {
                    (Self::ExchangeFunding(flow), TransitionCommand::None)
                } else if flow.cloud_backup_enabled {
                    (
                        Self::Terms(CompletionTarget::SelectWallet(flow.wallet_id.clone())),
                        TransitionCommand::None,
                    )
                } else {
                    (Self::VerifyWords(flow), TransitionCommand::None)
                }
            }
            (Self::ExchangeFunding(flow), OnboardingAction::ContinueFromExchangeFunding) => {
                if flow.cloud_backup_enabled {
                    (
                        Self::Terms(CompletionTarget::SelectWallet(flow.wallet_id.clone())),
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
            (Self::SoftwareImport, OnboardingAction::SoftwareImportCompleted { wallet_id }) => (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                TransitionCommand::None,
            ),
            (
                Self::HardwareImport { .. },
                OnboardingAction::HardwareImportCompleted { wallet_id },
            ) => (Self::Terms(CompletionTarget::SelectWallet(wallet_id)), TransitionCommand::None),
            (Self::SoftwareImport, OnboardingAction::BackupImportCompleted) => {
                (Self::Terms(CompletionTarget::SelectLatestOrNew), TransitionCommand::None)
            }
            (Self::RestoreOffer { .. }, OnboardingAction::StartRestore) => {
                (Self::Restoring, TransitionCommand::None)
            }
            (Self::RestoreOffer { .. }, OnboardingAction::SkipRestore) => {
                (Self::Terms(CompletionTarget::NewWalletSelect), TransitionCommand::None)
            }
            (Self::Restoring, OnboardingAction::RestoreComplete) => {
                (Self::Terms(CompletionTarget::SelectLatestOrNew), TransitionCommand::None)
            }
            (Self::Restoring, OnboardingAction::RestoreFailed { error }) => {
                (Self::RestoreOffer { error_message: Some(error) }, TransitionCommand::None)
            }
            (Self::VerifyWords(flow), OnboardingAction::VerifyWordsCompleted) => (
                Self::Terms(CompletionTarget::SelectWallet(flow.wallet_id.clone())),
                TransitionCommand::None,
            ),
            (Self::Terms(target), OnboardingAction::AcceptTerms) => {
                let command = TransitionCommand::CompleteOnboarding(target.clone());
                (Self::Terms(target), command)
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
            Self::Terms(_) => {
                OnboardingState { step: OnboardingStep::Terms, ..OnboardingState::default() }
            }
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
            | Self::Terms(CompletionTarget::SelectWallet(wallet_id)) => Some(wallet_id.clone()),
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
            FlowState::Terms(CompletionTarget::SelectWallet(id)) => assert_eq!(id, wallet_id),
            other => panic!("unexpected flow state: {other:?}"),
        }
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

    fn preview_created_wallet_flow(branch: OnboardingBranch) -> CreatedWalletFlow {
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("should parse preview mnemonic");

        CreatedWalletFlow {
            branch,
            wallet_id: WalletId::new(),
            created_words: mnemonic.words().map(str::to_string).collect(),
            word_validator: Arc::new(WordValidator::new(mnemonic)),
            cloud_backup_enabled: false,
            secret_words_saved: false,
        }
    }
}

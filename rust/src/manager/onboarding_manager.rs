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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CompletionTarget {
    None,
    SelectLatestOrNew,
    NewWalletSelect,
    SelectWallet,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CloudBackupContext {
    CreatedWallet,
    SoftwareImport,
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
struct InternalState {
    ui: OnboardingState,
    completion_target: CompletionTarget,
    wallet_id: Option<WalletId>,
    word_validator: Option<Arc<WordValidator>>,
    cloud_backup_context: Option<CloudBackupContext>,
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
        let initial_step =
            if has_wallets { OnboardingStep::Terms } else { OnboardingStep::CloudCheck };

        let manager = Arc::new(Self {
            state: Arc::new(RwLock::new(InternalState {
                ui: OnboardingState { step: initial_step, ..OnboardingState::default() },
                completion_target: if has_wallets {
                    CompletionTarget::SelectLatestOrNew
                } else {
                    CompletionTarget::None
                },
                wallet_id: None,
                word_validator: None,
                cloud_backup_context: None,
            })),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

        if initial_step == OnboardingStep::CloudCheck {
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
        self.state.read().wallet_id.clone()
    }

    pub fn word_validator(&self) -> Option<Arc<WordValidator>> {
        self.state.read().word_validator.clone()
    }

    pub fn dispatch(&self, action: OnboardingAction) {
        info!("Onboarding: dispatch action={action:?}");

        match action {
            OnboardingAction::ContinueFromWelcome => {
                self.set_step(OnboardingStep::BitcoinChoice);
            }
            OnboardingAction::SelectHasBitcoin { has_bitcoin } => {
                if has_bitcoin {
                    self.set_step(OnboardingStep::StorageChoice);
                } else {
                    self.begin_created_wallet_flow(OnboardingBranch::NewUser);
                }
            }
            OnboardingAction::SelectStorage { selection } => match selection {
                OnboardingStorageSelection::Exchange => {
                    self.begin_created_wallet_flow(OnboardingBranch::Exchange);
                }
                OnboardingStorageSelection::HardwareWallet => {
                    self.mutate_state(|state, deferred| {
                        Self::queue_ui_field(
                            state,
                            deferred,
                            Some(OnboardingBranch::Hardware),
                            |ui| &mut ui.branch,
                            Message::Branch,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            None,
                            |ui| &mut ui.hardware_device,
                            Message::HardwareDevice,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            None,
                            |ui| &mut ui.error_message,
                            Message::ErrorMessageChanged,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            OnboardingStep::HardwareDeviceSelection,
                            |ui| &mut ui.step,
                            Message::Step,
                        );
                    });
                }
                OnboardingStorageSelection::SoftwareWallet => {
                    self.mutate_state(|state, deferred| {
                        Self::queue_ui_field(
                            state,
                            deferred,
                            Some(OnboardingBranch::SoftwareImport),
                            |ui| &mut ui.branch,
                            Message::Branch,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            None,
                            |ui| &mut ui.error_message,
                            Message::ErrorMessageChanged,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            OnboardingStep::SoftwareChoice,
                            |ui| &mut ui.step,
                            Message::Step,
                        );
                    });
                }
            },
            OnboardingAction::SelectSoftwareAction { selection } => match selection {
                OnboardingSoftwareSelection::CreateNewWallet => {
                    self.begin_created_wallet_flow(OnboardingBranch::SoftwareCreate);
                }
                OnboardingSoftwareSelection::ImportExistingWallet => {
                    self.mutate_state(|state, deferred| {
                        Self::queue_ui_field(
                            state,
                            deferred,
                            Some(OnboardingBranch::SoftwareImport),
                            |ui| &mut ui.branch,
                            Message::Branch,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            None,
                            |ui| &mut ui.error_message,
                            Message::ErrorMessageChanged,
                        );
                        Self::queue_ui_field(
                            state,
                            deferred,
                            OnboardingStep::SoftwareImport,
                            |ui| &mut ui.step,
                            Message::Step,
                        );
                    });
                }
            },
            OnboardingAction::ContinueWalletCreation => {
                self.set_step(OnboardingStep::BackupWallet);
            }
            OnboardingAction::ShowSecretWords => {
                self.set_step(OnboardingStep::SecretWords);
            }
            OnboardingAction::SecretWordsSaved => {
                self.mutate_state(|state, deferred| {
                    Self::queue_ui_field(
                        state,
                        deferred,
                        true,
                        |ui| &mut ui.secret_words_saved,
                        Message::SecretWordsSaved,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::BackupWallet,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::OpenCloudBackup => {
                self.mutate_state(|state, deferred| {
                    state.cloud_backup_context = match state.ui.branch {
                        Some(OnboardingBranch::SoftwareImport) => {
                            Some(CloudBackupContext::SoftwareImport)
                        }
                        Some(
                            OnboardingBranch::NewUser
                            | OnboardingBranch::Exchange
                            | OnboardingBranch::SoftwareCreate,
                        ) => Some(CloudBackupContext::CreatedWallet),
                        _ => None,
                    };
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::CloudBackup,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::CloudBackupEnabled => {
                self.mutate_state(|state, deferred| {
                    let step = match state.cloud_backup_context {
                        Some(CloudBackupContext::SoftwareImport) => OnboardingStep::Terms,
                        _ => OnboardingStep::BackupWallet,
                    };
                    Self::queue_ui_field(
                        state,
                        deferred,
                        true,
                        |ui| &mut ui.cloud_backup_enabled,
                        Message::CloudBackupEnabled,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
                });
            }
            OnboardingAction::SkipCloudBackup => {
                self.mutate_state(|state, deferred| {
                    let step = match state.cloud_backup_context {
                        Some(CloudBackupContext::SoftwareImport) => OnboardingStep::Terms,
                        _ => OnboardingStep::BackupWallet,
                    };
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
                });
            }
            OnboardingAction::ContinueFromBackup => {
                self.advance_from_backup();
            }
            OnboardingAction::ContinueFromExchangeFunding => {
                self.advance_after_exchange_funding();
            }
            OnboardingAction::SelectHardwareDevice { device } => {
                self.mutate_state(|state, deferred| {
                    Self::queue_ui_field(
                        state,
                        deferred,
                        Some(device),
                        |ui| &mut ui.hardware_device,
                        Message::HardwareDevice,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::HardwareImport,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::SoftwareImportCompleted { wallet_id }
            | OnboardingAction::HardwareImportCompleted { wallet_id } => {
                self.mutate_state(|state, deferred| {
                    state.wallet_id = Some(wallet_id.clone());
                    state.completion_target = CompletionTarget::SelectWallet;
                    let step = if state.ui.branch == Some(OnboardingBranch::SoftwareImport) {
                        OnboardingStep::CloudBackup
                    } else {
                        OnboardingStep::Terms
                    };
                    state.cloud_backup_context =
                        if state.ui.branch == Some(OnboardingBranch::SoftwareImport) {
                            Some(CloudBackupContext::SoftwareImport)
                        } else {
                            None
                        };
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
                });
            }
            OnboardingAction::BackupImportCompleted => {
                self.mutate_state(|state, deferred| {
                    state.completion_target = CompletionTarget::SelectLatestOrNew;
                    state.cloud_backup_context = None;
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::Terms,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::StartRestore => {
                self.mutate_state(|state, deferred| {
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::Restoring,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::SkipRestore => {
                self.mutate_state(|state, deferred| {
                    state.completion_target = CompletionTarget::NewWalletSelect;
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::Terms,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::RestoreComplete => {
                self.mutate_state(|state, deferred| {
                    state.completion_target = CompletionTarget::SelectLatestOrNew;
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::Terms,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::RestoreFailed { error } => {
                self.mutate_state(|state, deferred| {
                    Self::queue_ui_field(
                        state,
                        deferred,
                        Some(error),
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::RestoreOffer,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            OnboardingAction::VerifyWordsCompleted => {
                self.set_step(OnboardingStep::Terms);
            }
            OnboardingAction::AcceptTerms => {
                self.complete_onboarding();
            }
            OnboardingAction::Back => {
                self.go_back();
            }
        }
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
        self.mutate_state(|state, deferred| {
            let (step, message) = match outcome {
                CloudCheckOutcome::BackupFound => (OnboardingStep::RestoreOffer, None),
                CloudCheckOutcome::NoBackupConfirmed => (OnboardingStep::Welcome, None),
                CloudCheckOutcome::Inconclusive(issue) => {
                    (OnboardingStep::RestoreOffer, Some(cloud_check_inconclusive_message(issue)))
                }
            };
            Self::queue_ui_field(
                state,
                deferred,
                message,
                |ui| &mut ui.error_message,
                Message::ErrorMessageChanged,
            );
            Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
        });
    }

    fn set_step(&self, step: OnboardingStep) {
        self.mutate_state(|state, deferred| {
            Self::queue_ui_field(
                state,
                deferred,
                None,
                |ui| &mut ui.error_message,
                Message::ErrorMessageChanged,
            );
            Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
        });
    }

    fn begin_created_wallet_flow(&self, branch: OnboardingBranch) {
        match Self::create_wallet() {
            Ok((wallet_id, words, word_validator)) => {
                self.mutate_state(|state, deferred| {
                    state.wallet_id = Some(wallet_id);
                    state.word_validator = Some(word_validator);
                    state.completion_target = CompletionTarget::SelectWallet;
                    state.cloud_backup_context = Some(CloudBackupContext::CreatedWallet);
                    Self::queue_ui_field(
                        state,
                        deferred,
                        Some(branch),
                        |ui| &mut ui.branch,
                        Message::Branch,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        words,
                        |ui| &mut ui.created_words,
                        Message::CreatedWords,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        false,
                        |ui| &mut ui.secret_words_saved,
                        Message::SecretWordsSaved,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        false,
                        |ui| &mut ui.cloud_backup_enabled,
                        Message::CloudBackupEnabled,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.hardware_device,
                        Message::HardwareDevice,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        None,
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::CreatingWallet,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
            Err(error) => {
                self.mutate_state(|state, deferred| {
                    Self::queue_ui_field(
                        state,
                        deferred,
                        Some(error),
                        |ui| &mut ui.error_message,
                        Message::ErrorMessageChanged,
                    );
                    Self::queue_ui_field(
                        state,
                        deferred,
                        OnboardingStep::Welcome,
                        |ui| &mut ui.step,
                        Message::Step,
                    );
                });
            }
        }
    }

    fn advance_from_backup(&self) {
        self.mutate_state(|state, deferred| {
            if !state.ui.secret_words_saved && !state.ui.cloud_backup_enabled {
                return;
            }

            let step = if state.ui.branch == Some(OnboardingBranch::Exchange) {
                OnboardingStep::ExchangeFunding
            } else if state.ui.cloud_backup_enabled {
                OnboardingStep::Terms
            } else {
                OnboardingStep::VerifyWords
            };
            Self::queue_ui_field(
                state,
                deferred,
                None,
                |ui| &mut ui.error_message,
                Message::ErrorMessageChanged,
            );
            Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
        });
    }

    fn advance_after_exchange_funding(&self) {
        self.mutate_state(|state, deferred| {
            let step = if state.ui.cloud_backup_enabled {
                OnboardingStep::Terms
            } else {
                OnboardingStep::VerifyWords
            };
            Self::queue_ui_field(
                state,
                deferred,
                None,
                |ui| &mut ui.error_message,
                Message::ErrorMessageChanged,
            );
            Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
        });
    }

    fn go_back(&self) {
        self.mutate_state(|state, deferred| {
            let step = match state.ui.step {
                OnboardingStep::BitcoinChoice => OnboardingStep::Welcome,
                OnboardingStep::StorageChoice => OnboardingStep::BitcoinChoice,
                OnboardingStep::SoftwareChoice => OnboardingStep::StorageChoice,
                OnboardingStep::SoftwareImport => OnboardingStep::SoftwareChoice,
                OnboardingStep::HardwareDeviceSelection => OnboardingStep::StorageChoice,
                OnboardingStep::HardwareImport => OnboardingStep::HardwareDeviceSelection,
                OnboardingStep::SecretWords => OnboardingStep::BackupWallet,
                OnboardingStep::ExchangeFunding => OnboardingStep::BackupWallet,
                current => current,
            };
            Self::queue_ui_field(
                state,
                deferred,
                None,
                |ui| &mut ui.error_message,
                Message::ErrorMessageChanged,
            );
            Self::queue_ui_field(state, deferred, step, |ui| &mut ui.step, Message::Step);
        });
    }

    fn complete_onboarding(&self) {
        let (target, wallet_id) = {
            let state = self.state.read();
            (state.completion_target, state.wallet_id.clone())
        };

        App::global().handle_action(AppAction::AcceptTerms);

        match target {
            CompletionTarget::None => {}
            CompletionTarget::SelectLatestOrNew => {
                FfiApp::global().select_latest_or_new_wallet();
            }
            CompletionTarget::NewWalletSelect => {
                FfiApp::global()
                    .load_and_reset_default_route(Route::NewWallet(NewWalletRoute::default()));
            }
            CompletionTarget::SelectWallet => {
                if let Some(wallet_id) = wallet_id {
                    let _ = FfiApp::global().select_wallet(wallet_id, None);
                }
            }
        }

        self.send(Message::Complete);
    }

    fn mutate_state<F>(&self, mutate: F)
    where
        F: FnOnce(&mut InternalState, &mut DeferredSender<Message>),
    {
        let mut deferred = DeferredSender::new(self.reconciler.clone());
        {
            let mut state = self.state.write();
            mutate(&mut state, &mut deferred);
        }
    }

    fn queue_ui_field<T>(
        state: &mut InternalState,
        deferred: &mut DeferredSender<Message>,
        value: T,
        field: impl FnOnce(&mut OnboardingState) -> &mut T,
        notify: fn(T) -> Message,
    ) where
        T: PartialEq + Clone,
    {
        let slot = field(&mut state.ui);
        if *slot == value {
            return;
        }

        *slot = value.clone();
        deferred.queue(notify(value));
    }

    fn send(&self, message: Message) {
        self.reconciler.send(message);
    }

    fn create_wallet() -> Result<(WalletId, Vec<String>, Arc<WordValidator>), String> {
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

        Ok((wallet.metadata.id, words, Arc::new(WordValidator::new(mnemonic))))
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
    use super::*;

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
}

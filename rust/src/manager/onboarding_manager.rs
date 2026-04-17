use std::{sync::Arc, time::Duration};

use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_util::ResultExt as _;
use flume::Receiver;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    app::{App, AppAction, FfiApp},
    database::{Database, global_config::GlobalConfigKey},
    manager::cloud_backup_manager::{CLOUD_BACKUP_MANAGER, RustCloudBackupManager},
    mnemonic::{Mnemonic as StoredMnemonic, MnemonicExt, NumberOfBip39Words},
    network::Network,
    pending_wallet::PendingWallet,
    router::{HotWalletRoute, NewWalletRoute, Route},
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
    RestoreUnavailable,
    Restoring,
    Welcome,
    BitcoinChoice,
    ReturningUserChoice,
    StorageChoice,
    SoftwareChoice,
    CreatingWallet,
    BackupWallet,
    CloudBackup,
    SecretWords,
    ExchangeFunding,
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
pub enum OnboardingReturningUserSelection {
    RestoreFromCoveBackup,
    UseAnotherWallet,
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
    pub cloud_restore_message: Option<String>,
    pub should_offer_cloud_restore: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum OnboardingAction {
    ContinueFromWelcome,
    SelectHasBitcoin { has_bitcoin: bool },
    SelectReturningUserFlow { selection: OnboardingReturningUserSelection },
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
    SoftwareImportCompleted { wallet_id: WalletId },
    HardwareImportCompleted { wallet_id: WalletId },
    OpenCloudRestore,
    StartRestore,
    SkipRestore,
    ContinueWithoutCloudRestore,
    RestoreComplete,
    RestoreFailed { error: String },
    AcceptTerms,
    Back,
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
    CloudRestoreMessageChanged(Option<String>),
    ShouldOfferCloudRestore(bool),
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
    SelectWallet { wallet_id: WalletId, post_onboarding: PostOnboardingDestination },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PostOnboardingDestination {
    None,
    VerifyWords,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TermsContext {
    SelectLatestOrNew,
    SelectWallet { wallet_id: WalletId, post_onboarding: PostOnboardingDestination },
    StartupRestoreRecovery,
}

#[derive(Debug, Clone)]
struct InitialFlowResolution {
    flow: FlowState,
    clear_persisted_progress: bool,
    start_cloud_check: bool,
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
    HardwareImport { wallet_id: WalletId },
}

#[derive(Debug, Clone)]
enum FlowState {
    CloudCheck {
        origin: RestoreOrigin,
    },
    RestoreOffer {
        origin: RestoreOrigin,
        error_message: Option<String>,
    },
    RestoreUnavailable {
        origin: RestoreOrigin,
    },
    Restoring {
        origin: RestoreOrigin,
    },
    Welcome {
        error_message: Option<String>,
    },
    BitcoinChoice {
        error_message: Option<String>,
    },
    ReturningUserChoice,
    StorageChoice {
        error_message: Option<String>,
    },
    SoftwareChoice {
        error_message: Option<String>,
    },
    CreatingWallet(CreatedWalletFlow),
    BackupWallet(CreatedWalletFlow),
    CloudBackup(CloudBackupFlow),
    SecretWords(CreatedWalletFlow),
    ExchangeFunding(CreatedWalletFlow),
    HardwareImport,
    SoftwareImport,
    Terms {
        context: TermsContext,
        error_message: Option<String>,
        progress: Option<OnboardingProgress>,
        allow_auto_advance: bool,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CloudRestoreDiscovery {
    Checking,
    BackupFound,
    NoBackupFound,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RestoreOrigin {
    Startup,
    Welcome,
    BitcoinChoice,
    ReturningUserChoice,
    StorageChoice,
    SoftwareChoice,
}

#[derive(Debug, Clone)]
struct InternalState {
    flow: FlowState,
    cloud_restore_discovery: CloudRestoreDiscovery,
    restore_offer_allowed: bool,
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
        let terms_accepted = Database::global().global_flag.is_terms_accepted();
        let resolution = resolve_initial_flow(
            Self::load_onboarding_progress(),
            has_wallets,
            terms_accepted,
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
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

        manager.maybe_advance_accepted_terms();

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
            let command = state.flow.apply_user_action(
                action,
                state.cloud_restore_discovery,
                &mut state.restore_offer_allowed,
            );
            let command = state.maybe_advance_accepted_terms(command);
            state.sync_ui(deferred);
            command
        });
        self.run_command(command);
    }
}

impl RustOnboardingManager {
    fn start_cloud_check(self: &Arc<Self>) {
        let me = Arc::clone(self);
        cove_tokio::task::spawn(async move {
            if CLOUD_BACKUP_MANAGER.is_offline() {
                me.finish_cloud_check(CloudCheckOutcome::Inconclusive(CloudCheckIssue::Offline));
                return;
            }

            let retry_delays = [1u64, 2, 2, 3, 5, 10];
            let cloud = CloudStorage::global().clone();
            let outcome = determine_cloud_check_outcome_async(
                &retry_delays,
                || {
                    let cloud = cloud.clone();
                    async move { cloud.has_any_cloud_backup().await }
                },
                |duration| tokio::time::sleep(duration),
            )
            .await;
            me.finish_cloud_check(outcome);
        });
    }

    fn finish_cloud_check(&self, outcome: CloudCheckOutcome) {
        self.apply_event(InternalEvent::CloudCheckFinished(outcome));
    }

    fn apply_event(&self, event: InternalEvent) {
        let command = self.mutate_state(|state, deferred| {
            state.flow.apply_event(
                event,
                &mut state.cloud_restore_discovery,
                state.restore_offer_allowed,
            );
            let command = state.maybe_advance_accepted_terms(TransitionCommand::None);
            state.sync_ui(deferred);
            command
        });
        self.run_command(command);
    }

    fn run_command(&self, command: TransitionCommand) {
        match command {
            TransitionCommand::None => {}
            TransitionCommand::CreateWallet(branch) => self.create_wallet_for_branch(branch),
            TransitionCommand::CompleteOnboarding(target) => self.complete_onboarding(target),
        }
    }

    fn maybe_advance_accepted_terms(&self) {
        if !Database::global().global_flag.is_terms_accepted() {
            return;
        }

        let command = self.mutate_state(|state, deferred| {
            let command = state.flow.resolve_terms_acceptance(true);
            state.sync_ui(deferred);
            command
        });
        self.run_command(command);
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
        let cloud_restore_discovery = CloudRestoreDiscovery::Checking;
        let restore_offer_allowed = true;
        let ui = flow.ui_state(cloud_restore_discovery, false);
        Self { flow, cloud_restore_discovery, restore_offer_allowed, ui }
    }

    fn maybe_advance_accepted_terms(&mut self, command: TransitionCommand) -> TransitionCommand {
        if !matches!(command, TransitionCommand::None)
            || !Database::global().global_flag.is_terms_accepted()
        {
            return command;
        }

        self.flow.resolve_terms_acceptance(true)
    }

    fn sync_ui(&mut self, deferred: &mut DeferredSender<Message>) {
        let next_ui =
            self.flow.ui_state(self.cloud_restore_discovery, self.should_offer_cloud_restore());

        if self.ui.branch != next_ui.branch {
            deferred.queue(Message::Branch(next_ui.branch));
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
        if self.ui.cloud_restore_state != next_ui.cloud_restore_state {
            deferred.queue(Message::CloudRestoreState(next_ui.cloud_restore_state));
        }
        if self.ui.cloud_restore_message != next_ui.cloud_restore_message {
            deferred
                .queue(Message::CloudRestoreMessageChanged(next_ui.cloud_restore_message.clone()));
        }
        if self.ui.should_offer_cloud_restore != next_ui.should_offer_cloud_restore {
            deferred.queue(Message::ShouldOfferCloudRestore(next_ui.should_offer_cloud_restore));
        }
        if self.ui.error_message != next_ui.error_message {
            deferred.queue(Message::ErrorMessageChanged(next_ui.error_message.clone()));
        }
        if self.ui.step != next_ui.step {
            deferred.queue(Message::Step(next_ui.step));
        }

        self.ui = next_ui;
    }

    fn should_offer_cloud_restore(&self) -> bool {
        self.restore_offer_allowed
            && self.cloud_restore_discovery == CloudRestoreDiscovery::BackupFound
    }
}

impl FlowState {
    fn terms(context: TermsContext, progress: Option<OnboardingProgress>) -> Self {
        Self::Terms { context, error_message: None, progress, allow_auto_advance: true }
    }

    fn apply_user_action(
        &mut self,
        action: OnboardingAction,
        cloud_restore_discovery: CloudRestoreDiscovery,
        restore_offer_allowed: &mut bool,
    ) -> TransitionCommand {
        let current = std::mem::replace(self, Self::Welcome { error_message: None });

        let (next, command) = match (current, action) {
            (Self::Welcome { .. }, OnboardingAction::ContinueFromWelcome) => {
                (Self::BitcoinChoice { error_message: None }, TransitionCommand::None)
            }
            (
                Self::BitcoinChoice { .. },
                OnboardingAction::SelectHasBitcoin { has_bitcoin: true },
            ) => (Self::ReturningUserChoice, TransitionCommand::None),
            (
                Self::BitcoinChoice { .. },
                OnboardingAction::SelectHasBitcoin { has_bitcoin: false },
            ) => {
                *restore_offer_allowed = false;
                (
                    Self::BitcoinChoice { error_message: None },
                    TransitionCommand::CreateWallet(OnboardingBranch::NewUser),
                )
            }
            (
                Self::ReturningUserChoice,
                OnboardingAction::SelectReturningUserFlow {
                    selection: OnboardingReturningUserSelection::RestoreFromCoveBackup,
                },
            ) => (
                Self::restore_entry_for(
                    cloud_restore_discovery,
                    RestoreOrigin::ReturningUserChoice,
                ),
                TransitionCommand::None,
            ),
            (
                Self::ReturningUserChoice,
                OnboardingAction::SelectReturningUserFlow {
                    selection: OnboardingReturningUserSelection::UseAnotherWallet,
                },
            ) => (Self::StorageChoice { error_message: None }, TransitionCommand::None),
            (
                Self::StorageChoice { .. },
                OnboardingAction::SelectStorage { selection: OnboardingStorageSelection::Exchange },
            ) => {
                *restore_offer_allowed = false;
                (
                    Self::StorageChoice { error_message: None },
                    TransitionCommand::CreateWallet(OnboardingBranch::Exchange),
                )
            }
            (
                Self::StorageChoice { .. },
                OnboardingAction::SelectStorage {
                    selection: OnboardingStorageSelection::HardwareWallet,
                },
            ) => {
                *restore_offer_allowed = false;
                (Self::HardwareImport, TransitionCommand::None)
            }
            (
                Self::StorageChoice { .. },
                OnboardingAction::SelectStorage {
                    selection: OnboardingStorageSelection::SoftwareWallet,
                },
            ) => (Self::SoftwareChoice { error_message: None }, TransitionCommand::None),
            (
                Self::SoftwareChoice { .. },
                OnboardingAction::SelectSoftwareAction {
                    selection: OnboardingSoftwareSelection::CreateNewWallet,
                },
            ) => {
                *restore_offer_allowed = false;
                (
                    Self::SoftwareChoice { error_message: None },
                    TransitionCommand::CreateWallet(OnboardingBranch::SoftwareCreate),
                )
            }
            (
                Self::SoftwareChoice { .. },
                OnboardingAction::SelectSoftwareAction {
                    selection: OnboardingSoftwareSelection::ImportExistingWallet,
                },
            ) => {
                *restore_offer_allowed = false;
                (Self::SoftwareImport, TransitionCommand::None)
            }
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
                Self::CloudBackup(
                    CloudBackupFlow::SoftwareImport { wallet_id }
                    | CloudBackupFlow::HardwareImport { wallet_id },
                ),
                OnboardingAction::CloudBackupEnabled,
            ) => (
                Self::terms(
                    TermsContext::SelectWallet {
                        wallet_id,
                        post_onboarding: PostOnboardingDestination::None,
                    },
                    None,
                ),
                TransitionCommand::None,
            ),
            (
                Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)),
                OnboardingAction::SkipCloudBackup,
            ) => (Self::BackupWallet(flow), TransitionCommand::None),
            (
                Self::CloudBackup(
                    CloudBackupFlow::SoftwareImport { wallet_id }
                    | CloudBackupFlow::HardwareImport { wallet_id },
                ),
                OnboardingAction::SkipCloudBackup,
            ) => (
                Self::terms(
                    TermsContext::SelectWallet {
                        wallet_id,
                        post_onboarding: PostOnboardingDestination::None,
                    },
                    None,
                ),
                TransitionCommand::None,
            ),
            (Self::BackupWallet(flow), OnboardingAction::ContinueFromBackup)
                if flow.secret_words_saved || flow.cloud_backup_enabled =>
            {
                if flow.branch == OnboardingBranch::Exchange {
                    (Self::ExchangeFunding(flow), TransitionCommand::None)
                } else {
                    let post_onboarding = if flow.cloud_backup_enabled {
                        PostOnboardingDestination::None
                    } else {
                        PostOnboardingDestination::VerifyWords
                    };

                    (
                        Self::terms(
                            TermsContext::SelectWallet {
                                wallet_id: flow.wallet_id.clone(),
                                post_onboarding,
                            },
                            Some(OnboardingProgress::from(&flow)),
                        ),
                        TransitionCommand::None,
                    )
                }
            }
            (Self::ExchangeFunding(flow), OnboardingAction::ContinueFromExchangeFunding) => {
                let post_onboarding = if flow.cloud_backup_enabled {
                    PostOnboardingDestination::None
                } else {
                    PostOnboardingDestination::VerifyWords
                };

                (
                    Self::terms(
                        TermsContext::SelectWallet {
                            wallet_id: flow.wallet_id.clone(),
                            post_onboarding,
                        },
                        Some(OnboardingProgress::from(&flow)),
                    ),
                    TransitionCommand::None,
                )
            }
            (Self::SoftwareImport, OnboardingAction::SoftwareImportCompleted { wallet_id }) => (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                TransitionCommand::None,
            ),
            (Self::HardwareImport, OnboardingAction::HardwareImportCompleted { wallet_id }) => (
                Self::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id }),
                TransitionCommand::None,
            ),
            (Self::StorageChoice { .. }, OnboardingAction::OpenCloudRestore) => (
                Self::restore_entry_for(cloud_restore_discovery, RestoreOrigin::StorageChoice),
                TransitionCommand::None,
            ),
            (Self::SoftwareChoice { .. }, OnboardingAction::OpenCloudRestore) => (
                Self::restore_entry_for(cloud_restore_discovery, RestoreOrigin::SoftwareChoice),
                TransitionCommand::None,
            ),
            (Self::RestoreOffer { origin, .. }, OnboardingAction::StartRestore) => {
                (Self::Restoring { origin }, TransitionCommand::None)
            }
            (Self::RestoreOffer { origin, .. }, OnboardingAction::SkipRestore) => {
                *restore_offer_allowed = false;
                (origin.flow_state(), TransitionCommand::None)
            }
            (
                Self::RestoreUnavailable { origin },
                OnboardingAction::ContinueWithoutCloudRestore,
            ) => (origin.flow_state_after_restore_unavailable(), TransitionCommand::None),
            (Self::Restoring { .. }, OnboardingAction::RestoreComplete) => {
                (Self::terms(TermsContext::SelectLatestOrNew, None), TransitionCommand::None)
            }
            (Self::Restoring { origin }, OnboardingAction::RestoreFailed { error }) => {
                (Self::RestoreOffer { origin, error_message: Some(error) }, TransitionCommand::None)
            }
            (mut terms @ Self::Terms { .. }, OnboardingAction::AcceptTerms) => {
                let command = terms.resolve_terms_acceptance(false);
                (terms, command)
            }
            (Self::BitcoinChoice { .. }, OnboardingAction::Back) => {
                (Self::Welcome { error_message: None }, TransitionCommand::None)
            }
            (Self::ReturningUserChoice, OnboardingAction::Back) => {
                (Self::BitcoinChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::StorageChoice { .. }, OnboardingAction::Back) => {
                (Self::ReturningUserChoice, TransitionCommand::None)
            }
            (Self::SoftwareChoice { .. }, OnboardingAction::Back) => {
                (Self::StorageChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::SoftwareImport, OnboardingAction::Back) => {
                (Self::SoftwareChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::HardwareImport, OnboardingAction::Back) => {
                (Self::StorageChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::RestoreUnavailable { origin }, OnboardingAction::Back) => {
                (origin.flow_state(), TransitionCommand::None)
            }
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

    fn resolve_terms_acceptance(&mut self, automatic: bool) -> TransitionCommand {
        let Self::Terms { context, progress, allow_auto_advance, .. } = self else {
            return TransitionCommand::None;
        };

        if automatic && !*allow_auto_advance {
            return TransitionCommand::None;
        }

        let context = context.clone();
        let progress = progress.clone();

        if let Some(next_flow) = context.next_flow_after_acceptance() {
            *self = next_flow;
            return TransitionCommand::None;
        }

        let target = context
            .completion_target()
            .expect("terminal terms context should resolve to a completion target");
        *self = Self::Terms { context, error_message: None, progress, allow_auto_advance: false };
        TransitionCommand::CompleteOnboarding(target)
    }

    fn apply_event(
        &mut self,
        event: InternalEvent,
        cloud_restore_discovery: &mut CloudRestoreDiscovery,
        restore_offer_allowed: bool,
    ) {
        if let InternalEvent::CloudCheckFinished(outcome) = &event {
            *cloud_restore_discovery = CloudRestoreDiscovery::from(*outcome);
        }

        let current = std::mem::replace(self, Self::Welcome { error_message: None });

        let next = match (current, event) {
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            ) => Self::RestoreOffer { origin, error_message: None },
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::NoBackupConfirmed),
            ) => Self::RestoreUnavailable { origin },
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(issue)),
            ) => {
                let _ = issue;
                Self::RestoreOffer { origin, error_message: None }
            }
            (
                Self::Welcome { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None }
            }
            (
                Self::BitcoinChoice { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::BitcoinChoice, error_message: None }
            }
            (
                Self::ReturningUserChoice,
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            ) if restore_offer_allowed => Self::RestoreOffer {
                origin: RestoreOrigin::ReturningUserChoice,
                error_message: None,
            },
            (state, InternalEvent::CloudCheckFinished(_)) => state,
            (Self::BitcoinChoice { .. }, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::NewUser =>
            {
                Self::CreatingWallet(flow)
            }
            (Self::StorageChoice { .. }, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::Exchange =>
            {
                Self::CreatingWallet(flow)
            }
            (Self::SoftwareChoice { .. }, InternalEvent::WalletCreated { flow })
                if flow.branch == OnboardingBranch::SoftwareCreate =>
            {
                Self::CreatingWallet(flow)
            }
            (
                Self::BitcoinChoice { .. },
                InternalEvent::WalletCreationFailed { branch: OnboardingBranch::NewUser, error },
            ) => Self::BitcoinChoice { error_message: Some(error) },
            (
                Self::StorageChoice { .. },
                InternalEvent::WalletCreationFailed { branch: OnboardingBranch::Exchange, error },
            ) => Self::StorageChoice { error_message: Some(error) },
            (
                Self::SoftwareChoice { .. },
                InternalEvent::WalletCreationFailed {
                    branch: OnboardingBranch::SoftwareCreate,
                    error,
                },
            ) => Self::SoftwareChoice { error_message: Some(error) },
            (
                Self::Terms { context, progress, allow_auto_advance: _, .. },
                InternalEvent::CompletionFailed { error },
            ) => Self::Terms {
                context,
                error_message: Some(error),
                progress,
                allow_auto_advance: false,
            },
            (state, event) => {
                warn!("Onboarding: invalid event={event:?} flow={state:?}");
                state
            }
        };

        *self = next;
    }

    fn ui_state(
        &self,
        cloud_restore_discovery: CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
    ) -> OnboardingState {
        let mut state = Self::base_ui_state(cloud_restore_discovery, should_offer_cloud_restore);

        match self {
            Self::CloudCheck { .. } => {
                state.step = OnboardingStep::CloudCheck;
                state
            }
            Self::RestoreOffer { error_message, .. } => {
                state.step = OnboardingStep::RestoreOffer;
                state.error_message = error_message.clone();
                state
            }
            Self::RestoreUnavailable { .. } => {
                state.step = OnboardingStep::RestoreUnavailable;
                state
            }
            Self::Restoring { .. } => {
                state.step = OnboardingStep::Restoring;
                state
            }
            Self::Welcome { error_message } => {
                state.step = OnboardingStep::Welcome;
                state.error_message = error_message.clone();
                state
            }
            Self::BitcoinChoice { error_message } => {
                state.step = OnboardingStep::BitcoinChoice;
                state.error_message = error_message.clone();
                state
            }
            Self::ReturningUserChoice => {
                state.step = OnboardingStep::ReturningUserChoice;
                state
            }
            Self::StorageChoice { error_message } => {
                state.step = OnboardingStep::StorageChoice;
                state.error_message = error_message.clone();
                state
            }
            Self::SoftwareChoice { error_message } => {
                state.step = OnboardingStep::SoftwareChoice;
                state.error_message = error_message.clone();
                state
            }
            Self::CreatingWallet(flow) => Self::project_created_wallet(
                OnboardingStep::CreatingWallet,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
            ),
            Self::BackupWallet(flow) => Self::project_created_wallet(
                OnboardingStep::BackupWallet,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
            ),
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => {
                Self::project_created_wallet(
                    OnboardingStep::CloudBackup,
                    flow,
                    cloud_restore_discovery,
                    should_offer_cloud_restore,
                )
            }
            Self::CloudBackup(CloudBackupFlow::SoftwareImport { .. }) => {
                state.step = OnboardingStep::CloudBackup;
                state.branch = Some(OnboardingBranch::SoftwareImport);
                state
            }
            Self::CloudBackup(CloudBackupFlow::HardwareImport { .. }) => {
                state.step = OnboardingStep::CloudBackup;
                state.branch = Some(OnboardingBranch::Hardware);
                state
            }
            Self::SecretWords(flow) => Self::project_created_wallet(
                OnboardingStep::SecretWords,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
            ),
            Self::ExchangeFunding(flow) => Self::project_created_wallet(
                OnboardingStep::ExchangeFunding,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
            ),
            Self::HardwareImport => {
                state.step = OnboardingStep::HardwareImport;
                state.branch = Some(OnboardingBranch::Hardware);
                state
            }
            Self::SoftwareImport => {
                state.step = OnboardingStep::SoftwareImport;
                state.branch = Some(OnboardingBranch::SoftwareImport);
                state
            }
            Self::Terms { error_message, .. } => {
                state.step = OnboardingStep::Terms;
                state.error_message = error_message.clone();
                state
            }
        }
    }

    fn current_wallet_id(&self) -> Option<WalletId> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::SecretWords(flow)
            | Self::ExchangeFunding(flow) => Some(flow.wallet_id.clone()),
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => Some(flow.wallet_id.clone()),
            Self::CloudBackup(
                CloudBackupFlow::SoftwareImport { wallet_id }
                | CloudBackupFlow::HardwareImport { wallet_id },
            ) => Some(wallet_id.clone()),
            Self::Terms { context: TermsContext::SelectWallet { wallet_id, .. }, .. } => {
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
            | Self::ExchangeFunding(flow) => Some(flow.word_validator.clone()),
            _ => None,
        }
    }

    fn restore_entry_for(
        cloud_restore_discovery: CloudRestoreDiscovery,
        origin: RestoreOrigin,
    ) -> Self {
        match cloud_restore_discovery {
            CloudRestoreDiscovery::Checking => Self::CloudCheck { origin },
            CloudRestoreDiscovery::BackupFound => {
                Self::RestoreOffer { origin, error_message: None }
            }
            CloudRestoreDiscovery::NoBackupFound => Self::RestoreUnavailable { origin },
            CloudRestoreDiscovery::Inconclusive(_) => {
                Self::RestoreOffer { origin, error_message: None }
            }
        }
    }

    fn base_ui_state(
        cloud_restore_discovery: CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
    ) -> OnboardingState {
        OnboardingState {
            cloud_restore_state: cloud_restore_discovery.ui_state(),
            cloud_restore_message: cloud_restore_discovery.message(),
            should_offer_cloud_restore,
            ..OnboardingState::default()
        }
    }

    fn project_created_wallet(
        step: OnboardingStep,
        flow: &CreatedWalletFlow,
        cloud_restore_discovery: CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
    ) -> OnboardingState {
        OnboardingState {
            step,
            branch: Some(flow.branch),
            created_words: flow.created_words.clone(),
            cloud_backup_enabled: flow.cloud_backup_enabled,
            secret_words_saved: flow.secret_words_saved,
            cloud_restore_state: cloud_restore_discovery.ui_state(),
            cloud_restore_message: cloud_restore_discovery.message(),
            should_offer_cloud_restore,
            error_message: None,
        }
    }

    fn persisted_progress(&self) -> Option<OnboardingProgress> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow))
            | Self::SecretWords(flow)
            | Self::ExchangeFunding(flow) => Some(OnboardingProgress::from(flow)),
            Self::Terms { context: TermsContext::SelectWallet { .. }, progress, .. } => {
                progress.clone()
            }
            _ => None,
        }
    }
}

impl TermsContext {
    fn completion_target(&self) -> Option<CompletionTarget> {
        match self {
            Self::SelectLatestOrNew => Some(CompletionTarget::SelectLatestOrNew),
            Self::SelectWallet { wallet_id, post_onboarding } => {
                Some(CompletionTarget::SelectWallet {
                    wallet_id: wallet_id.clone(),
                    post_onboarding: *post_onboarding,
                })
            }
            Self::StartupRestoreRecovery => None,
        }
    }

    fn next_flow_after_acceptance(&self) -> Option<FlowState> {
        match self {
            Self::StartupRestoreRecovery => Some(FlowState::Welcome { error_message: None }),
            Self::SelectLatestOrNew | Self::SelectWallet { .. } => None,
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

fn default_initial_flow(has_wallets: bool, terms_accepted: bool) -> FlowState {
    if has_wallets {
        FlowState::terms(TermsContext::SelectLatestOrNew, None)
    } else if terms_accepted {
        FlowState::CloudCheck { origin: RestoreOrigin::Startup }
    } else {
        FlowState::Welcome { error_message: None }
    }
}

fn resolve_initial_flow<F>(
    progress: Option<OnboardingProgress>,
    has_wallets: bool,
    terms_accepted: bool,
    load_mnemonic: F,
) -> InitialFlowResolution
where
    F: FnOnce(&WalletId, Network, WalletMode) -> Option<bip39::Mnemonic>,
{
    match progress {
        Some(progress) => match progress.restore_flow(load_mnemonic) {
            Some(flow) => InitialFlowResolution {
                flow,
                clear_persisted_progress: false,
                start_cloud_check: false,
            },
            None => InitialFlowResolution {
                flow: default_initial_flow(has_wallets, terms_accepted),
                clear_persisted_progress: true,
                start_cloud_check: !has_wallets,
            },
        },
        None => InitialFlowResolution {
            flow: default_initial_flow(has_wallets, terms_accepted),
            clear_persisted_progress: false,
            start_cloud_check: !has_wallets,
        },
    }
}

impl CloudRestoreDiscovery {
    fn ui_state(self) -> OnboardingCloudRestoreState {
        match self {
            Self::Checking => OnboardingCloudRestoreState::Checking,
            Self::BackupFound => OnboardingCloudRestoreState::BackupFound,
            Self::NoBackupFound => OnboardingCloudRestoreState::NoBackupFound,
            Self::Inconclusive(_) => OnboardingCloudRestoreState::Inconclusive,
        }
    }

    fn message(self) -> Option<String> {
        match self {
            Self::Inconclusive(issue) => Some(cloud_check_inconclusive_message(issue)),
            _ => None,
        }
    }
}

impl From<CloudCheckOutcome> for CloudRestoreDiscovery {
    fn from(value: CloudCheckOutcome) -> Self {
        match value {
            CloudCheckOutcome::BackupFound => Self::BackupFound,
            CloudCheckOutcome::NoBackupConfirmed => Self::NoBackupFound,
            CloudCheckOutcome::Inconclusive(issue) => Self::Inconclusive(issue),
        }
    }
}

impl RestoreOrigin {
    fn flow_state(self) -> FlowState {
        match self {
            Self::Startup => FlowState::terms(TermsContext::StartupRestoreRecovery, None),
            Self::Welcome => FlowState::Welcome { error_message: None },
            Self::BitcoinChoice => FlowState::BitcoinChoice { error_message: None },
            Self::ReturningUserChoice => FlowState::ReturningUserChoice,
            Self::StorageChoice => FlowState::StorageChoice { error_message: None },
            Self::SoftwareChoice => FlowState::SoftwareChoice { error_message: None },
        }
    }

    fn flow_state_after_restore_unavailable(self) -> FlowState {
        match self {
            Self::ReturningUserChoice => FlowState::StorageChoice { error_message: None },
            _ => self.flow_state(),
        }
    }
}

fn classify_cloud_check_error(error: &CloudStorageError) -> CloudCheckIssue {
    match RustCloudBackupManager::cloud_storage_issue(error) {
        crate::manager::cloud_backup_manager::CloudStorageIssue::Offline => {
            CloudCheckIssue::Offline
        }
        crate::manager::cloud_backup_manager::CloudStorageIssue::Unavailable => {
            CloudCheckIssue::CloudUnavailable
        }
        crate::manager::cloud_backup_manager::CloudStorageIssue::NotFound
        | crate::manager::cloud_backup_manager::CloudStorageIssue::QuotaExceeded
        | crate::manager::cloud_backup_manager::CloudStorageIssue::Other => {
            CloudCheckIssue::Unknown
        }
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

async fn determine_cloud_check_outcome_async<F, Fut, S, SleepFut>(
    retry_delays: &[u64],
    mut has_any_cloud_backup: F,
    mut sleep: S,
) -> CloudCheckOutcome
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, CloudStorageError>>,
    S: FnMut(Duration) -> SleepFut,
    SleepFut: std::future::Future<Output = ()>,
{
    for (attempt, delay) in retry_delays.iter().enumerate() {
        info!(
            "Onboarding: checking cloud backup attempt={}/{}",
            attempt + 1,
            retry_delays.len() + 1
        );

        match has_any_cloud_backup().await {
            Ok(true) => return CloudCheckOutcome::BackupFound,
            Ok(false) => return CloudCheckOutcome::NoBackupConfirmed,
            Err(error) => warn!("Onboarding: cloud backup check failed: {error}"),
        }

        sleep(Duration::from_secs(*delay)).await;
    }

    match has_any_cloud_backup().await {
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

    #[test]
    fn continue_from_backup_requires_a_saved_backup_method() {
        let mut flow =
            FlowState::BackupWallet(preview_created_wallet_flow(OnboardingBranch::NewUser));
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromBackup,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::BackupWallet(_)));
    }

    #[test]
    fn software_import_completion_goes_to_cloud_backup() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::SoftwareImport;
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::SoftwareImportCompleted { wallet_id: wallet_id.clone() },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
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
    fn enabling_cloud_backup_after_software_import_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::SoftwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::CloudBackupEnabled,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
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
    fn enabling_cloud_backup_after_hardware_import_goes_to_terms() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::CloudBackup(CloudBackupFlow::HardwareImport {
            wallet_id: wallet_id.clone(),
        });
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::CloudBackupEnabled,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
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

        let state = flow.ui_state(CloudRestoreDiscovery::Checking, false);

        assert_eq!(state.step, OnboardingStep::CloudBackup);
        assert_eq!(state.branch, Some(OnboardingBranch::Hardware));
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
    fn selecting_hardware_wallet_goes_to_hardware_import() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::SelectStorage {
                selection: OnboardingStorageSelection::HardwareWallet,
            },
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::HardwareImport));
        assert!(!restore_offer_allowed);
    }

    #[test]
    fn hardware_back_returns_to_storage_choice() {
        let mut flow = FlowState::HardwareImport;
        let mut restore_offer_allowed = false;

        flow.apply_user_action(
            OnboardingAction::Back,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
        );

        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn explicit_restore_without_backup_goes_to_restore_unavailable() {
        let mut flow = FlowState::ReturningUserChoice;
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SelectReturningUserFlow {
                selection: OnboardingReturningUserSelection::RestoreFromCoveBackup,
            },
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreUnavailable { origin: RestoreOrigin::ReturningUserChoice }
        ));
    }

    #[test]
    fn empty_wallet_startup_begins_at_welcome_and_starts_background_cloud_check() {
        let resolution = resolve_initial_flow(None, false, false, |_, _, _| None);

        assert!(!resolution.clear_persisted_progress);
        assert!(resolution.start_cloud_check);
        assert!(matches!(resolution.flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn accepted_terms_without_wallets_begins_at_startup_cloud_check() {
        let resolution = resolve_initial_flow(None, false, true, |_, _, _| None);

        assert!(!resolution.clear_persisted_progress);
        assert!(resolution.start_cloud_check);
        assert!(matches!(
            resolution.flow,
            FlowState::CloudCheck { origin: RestoreOrigin::Startup }
        ));
    }

    #[test]
    fn backup_found_auto_switches_on_early_screens() {
        let scenarios = [
            (FlowState::Welcome { error_message: None }, RestoreOrigin::Welcome),
            (FlowState::BitcoinChoice { error_message: None }, RestoreOrigin::BitcoinChoice),
            (FlowState::ReturningUserChoice, RestoreOrigin::ReturningUserChoice),
        ];

        for (mut flow, expected_origin) in scenarios {
            let mut discovery = CloudRestoreDiscovery::Checking;

            flow.apply_event(
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
                &mut discovery,
                true,
            );

            assert_eq!(discovery, CloudRestoreDiscovery::BackupFound);
            assert!(matches!(
                flow,
                FlowState::RestoreOffer { origin, error_message: None } if origin == expected_origin
            ));
        }
    }

    #[test]
    fn backup_found_does_not_auto_switch_on_late_screens() {
        let mut flow = FlowState::StorageChoice { error_message: None };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound),
            &mut discovery,
            true,
        );

        assert_eq!(discovery, CloudRestoreDiscovery::BackupFound);
        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
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
                FlowState::SoftwareChoice { error_message: None },
                OnboardingBranch::SoftwareCreate,
                OnboardingStep::SoftwareChoice,
            ),
        ];

        for (mut flow, branch, step) in scenarios {
            let mut discovery = CloudRestoreDiscovery::Checking;

            flow.apply_event(
                InternalEvent::WalletCreationFailed { branch, error: "create failed".into() },
                &mut discovery,
                false,
            );

            let state = flow.ui_state(CloudRestoreDiscovery::Checking, false);
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
                FlowState::SoftwareChoice { error_message: Some("create failed".into()) },
                OnboardingAction::SelectSoftwareAction {
                    selection: OnboardingSoftwareSelection::CreateNewWallet,
                },
                TransitionCommand::CreateWallet(OnboardingBranch::SoftwareCreate),
                OnboardingStep::SoftwareChoice,
            ),
        ];

        for (mut flow, action, expected_command, expected_step) in scenarios {
            let mut restore_offer_allowed = true;

            let command = flow.apply_user_action(
                action,
                CloudRestoreDiscovery::Checking,
                &mut restore_offer_allowed,
            );

            assert_eq!(command, expected_command);
            assert!(!restore_offer_allowed);

            let state = flow.ui_state(CloudRestoreDiscovery::Checking, false);
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
                OnboardingStep::ReturningUserChoice,
            ),
            (
                FlowState::SoftwareChoice { error_message: Some("create failed".into()) },
                OnboardingStep::StorageChoice,
            ),
        ];

        for (mut flow, expected_step) in scenarios {
            let mut restore_offer_allowed = true;

            let command = flow.apply_user_action(
                OnboardingAction::Back,
                CloudRestoreDiscovery::Checking,
                &mut restore_offer_allowed,
            );

            assert_eq!(command, TransitionCommand::None);

            let state = flow.ui_state(CloudRestoreDiscovery::Checking, false);
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
                FlowState::SoftwareChoice { error_message: Some("software failed".into()) },
                OnboardingStep::SoftwareChoice,
                "software failed",
            ),
        ];

        for (flow, expected_step, expected_error) in scenarios {
            let state = flow.ui_state(CloudRestoreDiscovery::Checking, false);

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
    fn skip_restore_returns_to_origin_and_disables_future_prompts() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::StorageChoice, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn skip_restore_from_startup_check_goes_to_terms() {
        let mut flow =
            FlowState::RestoreOffer { origin: RestoreOrigin::Startup, error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::SkipRestore,
            CloudRestoreDiscovery::BackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(!restore_offer_allowed);
        assert!(matches!(
            flow,
            FlowState::Terms {
                context: TermsContext::StartupRestoreRecovery,
                error_message: None,
                progress: None,
                allow_auto_advance: true,
            }
        ));
    }

    #[test]
    fn continue_without_cloud_restore_from_startup_goes_to_terms() {
        let mut flow = FlowState::RestoreUnavailable { origin: RestoreOrigin::Startup };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::Terms {
                context: TermsContext::StartupRestoreRecovery,
                error_message: None,
                progress: None,
                allow_auto_advance: true,
            }
        ));
    }

    #[test]
    fn accepting_startup_recovery_terms_goes_to_welcome() {
        let mut flow = FlowState::terms(TermsContext::StartupRestoreRecovery, None);
        let mut restore_offer_allowed = false;

        let command = flow.apply_user_action(
            OnboardingAction::AcceptTerms,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn auto_accepting_startup_recovery_terms_goes_to_welcome() {
        let mut flow = FlowState::terms(TermsContext::StartupRestoreRecovery, None);

        let command = flow.resolve_terms_acceptance(true);

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::Welcome { error_message: None }));
    }

    #[test]
    fn auto_accepting_terminal_terms_triggers_completion() {
        let wallet_id = WalletId::new();
        let mut flow = FlowState::terms(
            TermsContext::SelectWallet {
                wallet_id: wallet_id.clone(),
                post_onboarding: PostOnboardingDestination::VerifyWords,
            },
            None,
        );

        let command = flow.resolve_terms_acceptance(true);

        assert_eq!(
            command,
            TransitionCommand::CompleteOnboarding(CompletionTarget::SelectWallet {
                wallet_id,
                post_onboarding: PostOnboardingDestination::VerifyWords,
            })
        );
        assert!(matches!(
            flow,
            FlowState::Terms {
                context: TermsContext::SelectWallet { .. },
                error_message: None,
                progress: None,
                allow_auto_advance: false,
            }
        ));
    }

    #[test]
    fn continue_without_cloud_restore_from_returning_user_goes_to_storage_choice() {
        let mut flow = FlowState::RestoreUnavailable { origin: RestoreOrigin::ReturningUserChoice };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::StorageChoice { error_message: None }));
    }

    #[test]
    fn cloud_check_timeout_is_treated_as_cloud_unavailable() {
        let error = CloudStorageError::NotAvailable("iCloud metadata query timed out".into());

        assert_eq!(classify_cloud_check_error(&error), CloudCheckIssue::CloudUnavailable);
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

        assert_eq!(outcome, CloudCheckOutcome::Inconclusive(CloudCheckIssue::CloudUnavailable));
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
            false,
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
                allow_auto_advance: true,
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
            allow_auto_advance: false,
        };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CompletionFailed { error: "selection failed".into() },
            &mut discovery,
            false,
        );

        match flow {
            FlowState::Terms { error_message: Some(error), allow_auto_advance: false, .. } => {
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

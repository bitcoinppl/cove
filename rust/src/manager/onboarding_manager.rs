use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use backon::{FibonacciBuilder, Retryable as _};
use cove_cspp::backup_data::{EncryptedMasterKeyBackup, PasskeyProviderHint};
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_util::ResultExt as _;
use flume::Receiver;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    app::{App, AppAction, FfiApp},
    database::{Database, global_config::GlobalConfigKey},
    manager::{
        cloud_backup_manager::{
            CLOUD_BACKUP_MANAGER, CloudBackupPasskeyChoiceFlow, CloudStorageIssue,
        },
        connectivity_manager::CONNECTIVITY_MANAGER,
    },
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
    RestoreOffline,
    RestoreUnavailable,
    Restoring,
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
    pub cloud_restore_message: Option<String>,
    pub cloud_restore_provider_hint: Option<CloudRestoreProviderHint>,
    pub should_offer_cloud_restore: bool,
    pub cloud_restore_alert_visible: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct CloudRestoreProviderHint {
    pub provider_name: Option<String>,
    pub registered_at: u64,
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
    SkipRestore,
    ContinueWithoutCloudRestore,
    RestoreComplete,
    RestoreFailed { error: String },
    AcceptTerms,
    Back,
    BeginCloudBackupEnable,
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
    CloudRestoreProviderHintChanged(Option<CloudRestoreProviderHint>),
    ShouldOfferCloudRestore(bool),
    CloudRestoreAlertVisible(bool),
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

#[derive(Debug, Clone, Eq, PartialEq)]
enum CloudCheckOutcome {
    BackupFound(Option<CloudRestoreProviderHint>),
    NoBackupConfirmed,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CloudCheckIssue {
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
    RestoreOffline {
        origin: RestoreOrigin,
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
    StorageChoice {
        error_message: Option<String>,
    },
    CreatingWallet(CreatedWalletFlow),
    BackupWallet(CreatedWalletFlow),
    CloudBackup(CloudBackupFlow),
    SecretWords(CreatedWalletFlow),
    ExchangeFunding(CreatedWalletFlow),
    HardwareImport,
    SoftwareImport {
        error_message: Option<String>,
    },
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
    BeginCloudBackupEnable { backup_found: bool },
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

#[derive(Debug, Clone, Eq, PartialEq)]
enum CloudRestoreDiscovery {
    Checking,
    BackupFound(Option<CloudRestoreProviderHint>),
    NoBackupFound,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RestoreOrigin {
    Startup,
    Welcome,
    BitcoinChoice,
    StorageChoice,
    HardwareImport,
    SoftwareImport,
}

#[derive(Debug, Clone)]
struct InternalState {
    flow: FlowState,
    cloud_restore_discovery: CloudRestoreDiscovery,
    restore_offer_allowed: bool,
    cloud_restore_alert_dismissed: bool,
    ui: OnboardingState,
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
            cloud_check_in_flight: Arc::new(AtomicBool::new(false)),
            pending_cloud_check_retry: Arc::new(AtomicBool::new(false)),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
        });

        manager.start_connectivity_listener();
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
            if matches!(action, OnboardingAction::DismissCloudRestoreAlert) {
                state.cloud_restore_alert_dismissed = true;
            }

            let command = state.flow.apply_user_action(
                action,
                state.cloud_restore_discovery.clone(),
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
            TransitionCommand::BeginCloudBackupEnable { backup_found } => {
                self.begin_cloud_backup_enable(backup_found);
            }
            TransitionCommand::CompleteOnboarding(target) => self.complete_onboarding(target),
        }
    }

    fn begin_cloud_backup_enable(&self, backup_found: bool) {
        if backup_found {
            CLOUD_BACKUP_MANAGER.clear_existing_backup_found_prompt();
            CLOUD_BACKUP_MANAGER.set_passkey_choice_prompt(CloudBackupPasskeyChoiceFlow::Enable);
            return;
        }

        CLOUD_BACKUP_MANAGER.clear_existing_backup_found_prompt();
        CLOUD_BACKUP_MANAGER.clear_passkey_choice_prompt();
        CLOUD_BACKUP_MANAGER.enable_cloud_backup_no_discovery();
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
        let cloud_restore_alert_dismissed = false;
        let ui = flow.ui_state(&cloud_restore_discovery, false, false);
        Self {
            flow,
            cloud_restore_discovery,
            restore_offer_allowed,
            cloud_restore_alert_dismissed,
            ui,
        }
    }

    fn maybe_advance_accepted_terms(&mut self, command: TransitionCommand) -> TransitionCommand {
        if !matches!(command, TransitionCommand::None)
            || !Database::global().global_flag.is_terms_accepted()
        {
            return command;
        }

        self.flow.resolve_terms_acceptance(true)
    }

    fn prepare_offline_cloud_check_retry(
        &mut self,
        deferred: &mut DeferredSender<Message>,
    ) -> bool {
        if self.cloud_restore_discovery
            != CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline)
            || !self.flow.is_offline_cloud_check_retry_eligible()
        {
            return false;
        }

        self.cloud_restore_discovery = CloudRestoreDiscovery::Checking;
        self.flow.prepare_for_cloud_check_retry();
        self.sync_ui(deferred);
        true
    }

    fn sync_ui(&mut self, deferred: &mut DeferredSender<Message>) {
        let next_ui = self.flow.ui_state(
            &self.cloud_restore_discovery,
            self.should_offer_cloud_restore(),
            self.cloud_restore_alert_visible(),
        );

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
        if self.ui.cloud_restore_provider_hint != next_ui.cloud_restore_provider_hint {
            deferred.queue(Message::CloudRestoreProviderHintChanged(
                next_ui.cloud_restore_provider_hint.clone(),
            ));
        }
        if self.ui.should_offer_cloud_restore != next_ui.should_offer_cloud_restore {
            deferred.queue(Message::ShouldOfferCloudRestore(next_ui.should_offer_cloud_restore));
        }
        if self.ui.cloud_restore_alert_visible != next_ui.cloud_restore_alert_visible {
            deferred.queue(Message::CloudRestoreAlertVisible(next_ui.cloud_restore_alert_visible));
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
            && matches!(self.cloud_restore_discovery, CloudRestoreDiscovery::BackupFound(_))
    }

    fn cloud_restore_alert_visible(&self) -> bool {
        self.should_offer_cloud_restore()
            && !self.cloud_restore_alert_dismissed
            && matches!(self.flow, FlowState::HardwareImport | FlowState::SoftwareImport { .. })
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
            ) => (Self::StorageChoice { error_message: None }, TransitionCommand::None),
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
            ) => (Self::HardwareImport, TransitionCommand::None),
            (
                Self::StorageChoice { .. },
                OnboardingAction::SelectStorage {
                    selection: OnboardingStorageSelection::SoftwareWallet,
                },
            ) => (Self::SoftwareImport { error_message: None }, TransitionCommand::None),
            (Self::SoftwareImport { .. }, OnboardingAction::CreateSoftwareWallet) => {
                *restore_offer_allowed = false;
                (
                    Self::SoftwareImport { error_message: None },
                    TransitionCommand::CreateWallet(OnboardingBranch::SoftwareCreate),
                )
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
            (state @ Self::CloudBackup(_), OnboardingAction::BeginCloudBackupEnable) => (
                state,
                TransitionCommand::BeginCloudBackupEnable {
                    backup_found: matches!(
                        cloud_restore_discovery,
                        CloudRestoreDiscovery::BackupFound(_)
                    ),
                },
            ),
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
            (
                Self::SoftwareImport { .. },
                OnboardingAction::SoftwareImportCompleted { wallet_id },
            ) => (
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
            (Self::HardwareImport, OnboardingAction::OpenCloudRestore) => (
                Self::restore_entry_for(cloud_restore_discovery, RestoreOrigin::HardwareImport),
                TransitionCommand::None,
            ),
            (Self::SoftwareImport { .. }, OnboardingAction::OpenCloudRestore) => (
                Self::restore_entry_for(cloud_restore_discovery, RestoreOrigin::SoftwareImport),
                TransitionCommand::None,
            ),
            (
                state @ (Self::HardwareImport | Self::SoftwareImport { .. }),
                OnboardingAction::DismissCloudRestoreAlert,
            ) => (state, TransitionCommand::None),
            (Self::RestoreOffer { origin, .. }, OnboardingAction::StartRestore) => {
                (Self::Restoring { origin }, TransitionCommand::None)
            }
            (Self::RestoreOffer { origin, .. }, OnboardingAction::SkipRestore) => {
                *restore_offer_allowed = false;
                (origin.flow_state(), TransitionCommand::None)
            }
            (Self::RestoreOffline { origin }, OnboardingAction::ContinueWithoutCloudRestore) => {
                (origin.flow_state_after_restore_unavailable(), TransitionCommand::None)
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
            (Self::StorageChoice { .. }, OnboardingAction::Back) => {
                (Self::BitcoinChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::SoftwareImport { .. }, OnboardingAction::Back) => {
                (Self::StorageChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::HardwareImport, OnboardingAction::Back) => {
                (Self::StorageChoice { error_message: None }, TransitionCommand::None)
            }
            (Self::RestoreOffline { origin }, OnboardingAction::Back) => {
                (origin.flow_state(), TransitionCommand::None)
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
            *cloud_restore_discovery = CloudRestoreDiscovery::from(outcome.clone());
        }

        let current = std::mem::replace(self, Self::Welcome { error_message: None });

        let next = match (current, event) {
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) => Self::RestoreOffer { origin, error_message: None },
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::NoBackupConfirmed),
            ) => Self::RestoreUnavailable { origin },
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(issue)),
            ) => Self::restore_inconclusive_entry_for(issue, origin),
            (
                Self::Welcome { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::Welcome, error_message: None }
            }
            (
                Self::BitcoinChoice { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::BitcoinChoice, error_message: None }
            }
            (
                Self::StorageChoice { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::StorageChoice, error_message: None }
            }
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
            (Self::SoftwareImport { .. }, InternalEvent::WalletCreated { flow })
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
                Self::SoftwareImport { .. },
                InternalEvent::WalletCreationFailed {
                    branch: OnboardingBranch::SoftwareCreate,
                    error,
                },
            ) => Self::SoftwareImport { error_message: Some(error) },
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
        cloud_restore_discovery: &CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
        cloud_restore_alert_visible: bool,
    ) -> OnboardingState {
        let mut state = Self::base_ui_state(
            cloud_restore_discovery,
            should_offer_cloud_restore,
            cloud_restore_alert_visible,
        );

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
            Self::RestoreOffline { .. } => {
                state.step = OnboardingStep::RestoreOffline;
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
            Self::StorageChoice { error_message } => {
                state.step = OnboardingStep::StorageChoice;
                state.error_message = error_message.clone();
                state
            }
            Self::CreatingWallet(flow) => Self::project_created_wallet(
                OnboardingStep::CreatingWallet,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
                cloud_restore_alert_visible,
            ),
            Self::BackupWallet(flow) => Self::project_created_wallet(
                OnboardingStep::BackupWallet,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
                cloud_restore_alert_visible,
            ),
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => {
                Self::project_created_wallet(
                    OnboardingStep::CloudBackup,
                    flow,
                    cloud_restore_discovery,
                    should_offer_cloud_restore,
                    cloud_restore_alert_visible,
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
                cloud_restore_alert_visible,
            ),
            Self::ExchangeFunding(flow) => Self::project_created_wallet(
                OnboardingStep::ExchangeFunding,
                flow,
                cloud_restore_discovery,
                should_offer_cloud_restore,
                cloud_restore_alert_visible,
            ),
            Self::HardwareImport => {
                state.step = OnboardingStep::HardwareImport;
                state.branch = Some(OnboardingBranch::Hardware);
                state
            }
            Self::SoftwareImport { error_message } => {
                state.step = OnboardingStep::SoftwareImport;
                state.branch = Some(OnboardingBranch::SoftwareImport);
                state.error_message = error_message.clone();
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
            CloudRestoreDiscovery::BackupFound(_) => {
                Self::RestoreOffer { origin, error_message: None }
            }
            CloudRestoreDiscovery::NoBackupFound => Self::RestoreUnavailable { origin },
            CloudRestoreDiscovery::Inconclusive(issue) => {
                Self::restore_inconclusive_entry_for(issue, origin)
            }
        }
    }

    fn restore_inconclusive_entry_for(issue: CloudCheckIssue, origin: RestoreOrigin) -> Self {
        match issue {
            CloudCheckIssue::Offline => Self::RestoreOffline { origin },
            CloudCheckIssue::CloudUnavailable | CloudCheckIssue::Unknown => {
                Self::RestoreOffer { origin, error_message: None }
            }
        }
    }

    fn base_ui_state(
        cloud_restore_discovery: &CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
        cloud_restore_alert_visible: bool,
    ) -> OnboardingState {
        OnboardingState {
            cloud_restore_state: cloud_restore_discovery.ui_state(),
            cloud_restore_message: cloud_restore_discovery.message(),
            cloud_restore_provider_hint: cloud_restore_discovery.provider_hint(),
            should_offer_cloud_restore,
            cloud_restore_alert_visible,
            ..OnboardingState::default()
        }
    }

    fn project_created_wallet(
        step: OnboardingStep,
        flow: &CreatedWalletFlow,
        cloud_restore_discovery: &CloudRestoreDiscovery,
        should_offer_cloud_restore: bool,
        cloud_restore_alert_visible: bool,
    ) -> OnboardingState {
        OnboardingState {
            step,
            branch: Some(flow.branch),
            created_words: flow.created_words.clone(),
            cloud_backup_enabled: flow.cloud_backup_enabled,
            secret_words_saved: flow.secret_words_saved,
            cloud_restore_state: cloud_restore_discovery.ui_state(),
            cloud_restore_message: cloud_restore_discovery.message(),
            cloud_restore_provider_hint: cloud_restore_discovery.provider_hint(),
            should_offer_cloud_restore,
            cloud_restore_alert_visible,
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

    fn is_offline_cloud_check_retry_eligible(&self) -> bool {
        matches!(
            self,
            Self::CloudCheck { .. }
                | Self::RestoreOffer { .. }
                | Self::RestoreOffline { .. }
                | Self::Welcome { .. }
                | Self::BitcoinChoice { .. }
                | Self::StorageChoice { .. }
        )
    }

    fn prepare_for_cloud_check_retry(&mut self) {
        let origin = match self {
            Self::CloudCheck { origin }
            | Self::RestoreOffer { origin, .. }
            | Self::RestoreOffline { origin } => Some(*origin),
            _ => None,
        };

        if let Some(origin) = origin {
            *self = Self::CloudCheck { origin };
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
    fn ui_state(&self) -> OnboardingCloudRestoreState {
        match self {
            Self::Checking => OnboardingCloudRestoreState::Checking,
            Self::BackupFound(_) => OnboardingCloudRestoreState::BackupFound,
            Self::NoBackupFound => OnboardingCloudRestoreState::NoBackupFound,
            Self::Inconclusive(_) => OnboardingCloudRestoreState::Inconclusive,
        }
    }

    fn message(&self) -> Option<String> {
        match self {
            Self::Inconclusive(issue) => Some(cloud_check_inconclusive_message(*issue)),
            _ => None,
        }
    }

    fn provider_hint(&self) -> Option<CloudRestoreProviderHint> {
        match self {
            Self::BackupFound(hint) => hint.clone(),
            _ => None,
        }
    }
}

impl From<CloudCheckOutcome> for CloudRestoreDiscovery {
    fn from(value: CloudCheckOutcome) -> Self {
        match value {
            CloudCheckOutcome::BackupFound(hint) => Self::BackupFound(hint),
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
            Self::StorageChoice => FlowState::StorageChoice { error_message: None },
            Self::HardwareImport => FlowState::HardwareImport,
            Self::SoftwareImport => FlowState::SoftwareImport { error_message: None },
        }
    }

    fn flow_state_after_restore_unavailable(self) -> FlowState {
        self.flow_state()
    }
}

fn cloud_check_inconclusive_message(issue: CloudCheckIssue) -> String {
    match issue {
        CloudCheckIssue::Offline => {
            "You're offline, so Cove can't check for a cloud backup right now. You can continue onboarding now and check Cloud Backup later in Settings.".into()
        }
        CloudCheckIssue::CloudUnavailable => {
            "We couldn't confirm whether a cloud backup is available because cloud storage may be unavailable. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
        CloudCheckIssue::Unknown => {
            "We couldn't confirm whether a cloud backup is available. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CloudRestoreBackupSnapshot {
    has_backup: bool,
    provider_hint: Option<CloudRestoreProviderHint>,
}

async fn inspect_cloud_restore_backup(
    cloud: cove_device::cloud_storage::CloudStorageClient,
) -> Result<CloudRestoreBackupSnapshot, CloudStorageError> {
    let namespaces = cloud.list_namespaces().await?;
    if namespaces.is_empty() {
        info!("Onboarding: cloud backup namespace check found no namespaces");
        return Ok(CloudRestoreBackupSnapshot { has_backup: false, provider_hint: None });
    }

    info!("Onboarding: cloud backup namespace check found {} namespace(s)", namespaces.len());

    let provider_hint = inspect_cloud_restore_namespaces(&cloud, namespaces).await?;
    Ok(CloudRestoreBackupSnapshot {
        has_backup: provider_hint.has_backup,
        provider_hint: provider_hint.provider_hint,
    })
}

struct InspectedCloudRestoreNamespaces {
    has_backup: bool,
    provider_hint: Option<CloudRestoreProviderHint>,
}

async fn inspect_cloud_restore_namespaces(
    cloud: &cove_device::cloud_storage::CloudStorageClient,
    namespaces: Vec<String>,
) -> Result<InspectedCloudRestoreNamespaces, CloudStorageError> {
    let mut hints = Vec::new();
    let mut found_backup = false;
    let mut first_non_not_found_error = None;

    for namespace in namespaces {
        let master_json = match cloud.download_master_key_backup(namespace.clone()).await {
            Ok(master_json) => master_json,
            Err(error @ CloudStorageError::NotFound(_)) => {
                info!("No cloud restore backup namespace={namespace} reason=not_found");
                record_cloud_restore_download_error(&mut first_non_not_found_error, error);
                continue;
            }
            Err(error) => {
                info!("No cloud restore backup namespace={namespace} reason=download_failed");
                record_cloud_restore_download_error(&mut first_non_not_found_error, error);
                continue;
            }
        };

        let Ok(encrypted) = serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) else {
            info!(
                "No cloud restore passkey provider hint namespace={namespace} reason=deserialize_failed"
            );
            continue;
        };
        found_backup = true;

        let Some(raw_hint) = encrypted.passkey_provider_hint.as_ref() else {
            info!("No cloud restore passkey provider hint namespace={namespace} reason=missing");
            continue;
        };

        debug!(
            "Found cloud restore passkey provider hint namespace={namespace} aaguid={} registered_platform={:?} registered_at={}",
            raw_hint.aaguid, raw_hint.registered_platform, raw_hint.registered_at
        );

        let hint = resolve_provider_hint(raw_hint);
        if hint.provider_name.is_none() {
            debug!(
                "No resolved cloud restore passkey provider hint namespace={namespace} aaguid={} registered_platform={:?} registered_at={} reason=unknown_provider",
                raw_hint.aaguid, raw_hint.registered_platform, raw_hint.registered_at
            );
        }

        hints.push(hint);
    }

    if found_backup {
        return Ok(InspectedCloudRestoreNamespaces {
            has_backup: true,
            provider_hint: choose_restore_provider_hint(hints),
        });
    }

    if let Some(error) = first_non_not_found_error {
        return Err(error);
    }

    Ok(InspectedCloudRestoreNamespaces { has_backup: false, provider_hint: None })
}

fn record_cloud_restore_download_error(
    first_non_not_found_error: &mut Option<CloudStorageError>,
    error: CloudStorageError,
) {
    if !matches!(error, CloudStorageError::NotFound(_)) {
        first_non_not_found_error.get_or_insert(error);
    }
}

fn choose_restore_provider_hint(
    hints: Vec<CloudRestoreProviderHint>,
) -> Option<CloudRestoreProviderHint> {
    hints.into_iter().max_by_key(|hint| hint.registered_at)
}

fn resolve_provider_hint(hint: &PasskeyProviderHint) -> CloudRestoreProviderHint {
    CloudRestoreProviderHint {
        provider_name: hint.known_provider().map(|provider| provider.display_name().into()),
        registered_at: hint.registered_at,
    }
}

async fn determine_cloud_check_outcome<F, Fut, S>(
    mut inspect_backup: F,
    sleep: S,
) -> CloudCheckOutcome
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<CloudRestoreBackupSnapshot, CloudStorageError>>,
    S: backon::Sleeper,
{
    let max_retries = 6;
    let mut attempt = 0;
    let result = (|| {
        attempt += 1;
        info!("Onboarding: checking cloud backup attempt={attempt}");
        inspect_backup()
    })
    .retry(
        FibonacciBuilder::default()
            .with_max_delay(Duration::from_secs(10))
            .with_max_times(max_retries),
    )
    .sleep(sleep)
    .notify(|error: &CloudStorageError, _| warn!("Onboarding: cloud backup check failed: {error}"))
    .await;

    match result {
        Ok(snapshot) if snapshot.has_backup => {
            log_cloud_restore_provider_hint(snapshot.provider_hint.as_ref());
            CloudCheckOutcome::BackupFound(snapshot.provider_hint)
        }
        Ok(_) => {
            info!("Onboarding: cloud backup check completed backup_found=false");
            CloudCheckOutcome::NoBackupConfirmed
        }
        Err(error) => {
            warn!("Onboarding: final cloud backup check failed: {error}");
            CloudCheckOutcome::Inconclusive(error.into())
        }
    }
}

fn log_cloud_restore_provider_hint(provider_hint: Option<&CloudRestoreProviderHint>) {
    match provider_hint {
        Some(hint) => info!(
            "Onboarding: cloud backup check completed backup_found=true provider_hint=some provider_name={:?} registered_at={}",
            hint.provider_name, hint.registered_at
        ),
        None => {
            info!("Onboarding: cloud backup check completed backup_found=true provider_hint=none")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use bip39::Mnemonic;
    use cove_cspp::backup_data::PasskeyRegistrationPlatform as BackupPasskeyRegistrationPlatform;

    use super::*;

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
        let mut flow = FlowState::SoftwareImport { error_message: None };
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
        );

        assert_eq!(command, TransitionCommand::BeginCloudBackupEnable { backup_found: true });
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
        );

        assert_eq!(command, TransitionCommand::BeginCloudBackupEnable { backup_found: false });
        match flow {
            FlowState::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id: id }) => {
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
    fn welcome_continues_to_bitcoin_choice() {
        let mut flow = FlowState::Welcome { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueFromWelcome,
            CloudRestoreDiscovery::Checking,
            &mut restore_offer_allowed,
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
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
        assert!(restore_offer_allowed);
    }

    #[test]
    fn restoring_failure_returns_to_restore_offer_with_error() {
        let mut flow = FlowState::Restoring { origin: RestoreOrigin::StorageChoice };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::RestoreFailed { error: "passkey verification failed".into() },
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreOffer {
                origin: RestoreOrigin::StorageChoice,
                error_message: Some(message),
            } if message == "passkey verification failed"
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
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(
            flow,
            FlowState::RestoreUnavailable { origin: RestoreOrigin::StorageChoice }
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
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::RestoreOffline { origin: RestoreOrigin::StorageChoice }));
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
        let manager =
            preview_manager(FlowState::HardwareImport, CloudRestoreDiscovery::BackupFound(None));

        manager.dispatch(OnboardingAction::DismissCloudRestoreAlert);

        assert_eq!(manager.state().step, OnboardingStep::HardwareImport);
        assert!(!manager.state().cloud_restore_alert_visible);
    }

    #[test]
    fn opening_restore_from_import_returns_to_import_on_skip() {
        let mut flow = FlowState::SoftwareImport { error_message: None };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::OpenCloudRestore,
            CloudRestoreDiscovery::BackupFound(None),
            &mut restore_offer_allowed,
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
        let mut flow = FlowState::CloudCheck { origin: RestoreOrigin::Startup };
        let mut discovery = CloudRestoreDiscovery::Checking;

        flow.apply_event(
            InternalEvent::CloudCheckFinished(CloudCheckOutcome::Inconclusive(
                CloudCheckIssue::Offline,
            )),
            &mut discovery,
            true,
        );

        assert_eq!(discovery, CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline));
        assert!(matches!(flow, FlowState::RestoreOffline { origin: RestoreOrigin::Startup }));
        assert_eq!(flow.ui_state(&discovery, false, false).step, OnboardingStep::RestoreOffline);
    }

    #[test]
    fn cloud_check_non_offline_inconclusive_keeps_restore_offer_flow() {
        let mut flow = FlowState::CloudCheck { origin: RestoreOrigin::Startup };
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
            FlowState::RestoreOffer { origin: RestoreOrigin::Startup, error_message: None }
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
            CloudRestoreDiscovery::BackupFound(None),
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
    fn continue_without_cloud_restore_from_startup_offline_goes_to_terms() {
        let mut flow = FlowState::RestoreOffline { origin: RestoreOrigin::Startup };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
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
    fn continue_without_cloud_restore_from_import_returns_to_import() {
        let mut flow = FlowState::RestoreUnavailable { origin: RestoreOrigin::SoftwareImport };
        let mut restore_offer_allowed = true;

        let command = flow.apply_user_action(
            OnboardingAction::ContinueWithoutCloudRestore,
            CloudRestoreDiscovery::NoBackupFound,
            &mut restore_offer_allowed,
        );

        assert_eq!(command, TransitionCommand::None);
        assert!(matches!(flow, FlowState::SoftwareImport { error_message: None }));
    }

    #[test]
    fn offline_retry_rechecks_from_restore_offline_screen() {
        let mut state = preview_internal_state(
            FlowState::RestoreOffline { origin: RestoreOrigin::Startup },
            CloudRestoreDiscovery::Inconclusive(CloudCheckIssue::Offline),
        );

        assert!(prepare_offline_cloud_check_retry(&mut state));
        assert_eq!(state.cloud_restore_discovery, CloudRestoreDiscovery::Checking);
        assert!(matches!(state.flow, FlowState::CloudCheck { origin: RestoreOrigin::Startup }));
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
        assert_eq!(state.ui.cloud_restore_message, None);
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
        assert_eq!(manager.state().cloud_restore_message, None);
        assert_no_reconcile_messages(&manager);
    }

    #[test]
    fn late_pending_connectivity_retry_after_offline_finish_is_taken_over() {
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
        assert_eq!(manager.state().cloud_restore_message, None);
    }

    #[test]
    fn startup_restore_retry_after_offline_finish_skips_transient_offline_messages() {
        let manager = preview_manager(
            FlowState::CloudCheck { origin: RestoreOrigin::Startup },
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
        assert_eq!(manager.state().cloud_restore_message, None);
        assert_no_reconcile_messages(&manager);
    }

    #[test]
    fn connectivity_reconnect_while_cloud_check_is_in_flight_does_not_retry_non_offline_finish() {
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
        });

        assert_eq!(
            hint,
            CloudRestoreProviderHint {
                provider_name: Some("Google Password Manager".into()),
                registered_at: 1_777_661_234,
            }
        );
    }

    #[test]
    fn restore_provider_hint_preserves_unknown_provider_date() {
        let hint = resolve_provider_hint(&PasskeyProviderHint {
            aaguid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            registered_platform: BackupPasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_236,
        });

        assert_eq!(
            hint,
            CloudRestoreProviderHint { provider_name: None, registered_at: 1_777_661_236 }
        );
    }

    #[test]
    fn restore_provider_hint_selects_latest_hint() {
        let hint = choose_restore_provider_hint(vec![
            CloudRestoreProviderHint {
                provider_name: Some("Apple Passwords".into()),
                registered_at: 1_777_661_234,
            },
            CloudRestoreProviderHint { provider_name: None, registered_at: 1_777_661_236 },
        ])
        .expect("latest hint should be selected");

        assert_eq!(
            hint,
            CloudRestoreProviderHint { provider_name: None, registered_at: 1_777_661_236 }
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
            ui,
        }
    }

    fn preview_manager(
        flow: FlowState,
        cloud_restore_discovery: CloudRestoreDiscovery,
    ) -> Arc<RustOnboardingManager> {
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
        flow.apply_user_action(action, CloudRestoreDiscovery::Checking, &mut restore_offer_allowed)
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

    fn prepare_offline_cloud_check_retry(state: &mut InternalState) -> bool {
        let (sender, _receiver) = flume::bounded(16);
        let mut deferred = DeferredSender::new(MessageSender::new(sender));
        state.prepare_offline_cloud_check_retry(&mut deferred)
    }
}

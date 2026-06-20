use std::sync::Arc;

use tracing::warn;

use crate::{
    manager::cloud_backup_manager::{CloudBackupRestoreFlow, CloudBackupRestoreReport},
    network::Network,
    wallet::metadata::{WalletId, WalletMode},
    word_validator::WordValidator,
};

use super::{
    CloudCheckIssue, CloudCheckOutcome, CloudRestoreProviderHint, Message, OnboardingAction,
    OnboardingBranch, OnboardingCloudRestoreState, OnboardingError, OnboardingProgress,
    OnboardingRestoreFailure, OnboardingRestoreState, OnboardingState, OnboardingStep,
    OnboardingStorageSelection,
};
use crate::manager::deferred_sender::DeferredSender;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum CompletionTarget {
    SelectLatestOrNew,
    SelectWallet { wallet_id: WalletId, post_onboarding: PostOnboardingDestination },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum PostOnboardingDestination {
    None,
    VerifyWords,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum TermsContext {
    SelectLatestOrNew,
    SelectWallet { wallet_id: WalletId, post_onboarding: PostOnboardingDestination },
}

#[derive(Debug, Clone)]
pub(crate) struct CreatedWalletFlow {
    pub(crate) branch: OnboardingBranch,
    pub(crate) wallet_id: WalletId,
    pub(crate) network: Network,
    pub(crate) wallet_mode: WalletMode,
    pub(crate) created_words: Vec<String>,
    pub(crate) word_validator: Arc<WordValidator>,
    pub(crate) cloud_backup_enabled: bool,
    pub(crate) secret_words_saved: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum CloudBackupFlow {
    CreatedWallet(CreatedWalletFlow),
    SoftwareImport { wallet_id: WalletId },
    HardwareImport { wallet_id: WalletId },
}

#[derive(Debug, Clone)]
pub(crate) enum FlowState {
    CloudCheck {
        origin: RestoreOrigin,
    },
    RestoreOffer {
        origin: RestoreOrigin,
        error: Option<OnboardingError>,
    },
    RestoreOffline {
        origin: RestoreOrigin,
    },
    RestoreUnavailable {
        origin: RestoreOrigin,
    },
    Restoring {
        origin: RestoreOrigin,
        attempt_id: u64,
        flow: CloudBackupRestoreFlow,
    },
    RestoreComplete {
        origin: RestoreOrigin,
        report: CloudBackupRestoreReport,
    },
    RestoreFailed {
        origin: RestoreOrigin,
        failure: OnboardingRestoreFailure,
    },
    Welcome {
        error: Option<OnboardingError>,
    },
    BitcoinChoice {
        error: Option<OnboardingError>,
    },
    StorageChoice {
        error: Option<OnboardingError>,
    },
    CreatingWallet(CreatedWalletFlow),
    BackupWallet(CreatedWalletFlow),
    CloudBackup(CloudBackupFlow),
    CloudBackupSuccess(CloudBackupFlow),
    SecretWords(CreatedWalletFlow),
    ExchangeFunding(CreatedWalletFlow),
    HardwareImport,
    SoftwareImport {
        error: Option<OnboardingError>,
    },
    Terms {
        context: TermsContext,
        error: Option<OnboardingError>,
        progress: Option<OnboardingProgress>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum TransitionCommand {
    None,
    CreateWallet(OnboardingBranch),
    StartRestore { attempt_id: u64 },
    BeginCloudBackupEnable { discovery: CloudRestoreDiscovery },
    CompleteOnboarding(CompletionTarget),
}

#[derive(Debug, Clone)]
pub(crate) enum InternalEvent {
    CloudCheckFinished(CloudCheckOutcome),
    RestoreProgress { attempt_id: u64, flow: CloudBackupRestoreFlow },
    RestoreComplete { attempt_id: u64, report: CloudBackupRestoreReport },
    RestoreNoBackupFound { attempt_id: u64 },
    RestoreFailed { attempt_id: u64, failure: OnboardingRestoreFailure },
    WalletCreated { flow: CreatedWalletFlow },
    WalletCreationFailed { branch: OnboardingBranch, error: String },
    CompletionFailed { error: String },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum CloudRestoreDiscovery {
    Checking,
    BackupFound(Option<CloudRestoreProviderHint>),
    NoBackupFound,
    Inconclusive(CloudCheckIssue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OnboardingCloudBackupEnableStart {
    ConfirmExistingBackup(Option<CloudRestoreProviderHint>),
    CreateNewPasskey,
}

impl OnboardingCloudBackupEnableStart {
    pub(crate) fn from_discovery(discovery: CloudRestoreDiscovery) -> Self {
        match discovery {
            CloudRestoreDiscovery::BackupFound(hint) => Self::ConfirmExistingBackup(hint),
            CloudRestoreDiscovery::Checking
            | CloudRestoreDiscovery::NoBackupFound
            | CloudRestoreDiscovery::Inconclusive(_) => Self::CreateNewPasskey,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum RestoreOrigin {
    Welcome,
    BitcoinChoice,
    StorageChoice,
    HardwareImport,
    SoftwareImport,
}

#[derive(Debug, Clone)]
pub(crate) struct InternalState {
    pub(crate) flow: FlowState,
    pub(crate) cloud_restore_discovery: CloudRestoreDiscovery,
    pub(crate) restore_offer_allowed: bool,
    pub(crate) cloud_restore_alert_dismissed: bool,
    pub(crate) next_restore_attempt_id: u64,
    pub(crate) ui: OnboardingState,
}

impl InternalState {
    pub(crate) fn new(flow: FlowState) -> Self {
        let cloud_restore_discovery = CloudRestoreDiscovery::Checking;
        let restore_offer_allowed = true;
        let cloud_restore_alert_dismissed = false;
        let next_restore_attempt_id = 1;
        let ui = flow.ui_state(&cloud_restore_discovery, false, false);
        Self {
            flow,
            cloud_restore_discovery,
            restore_offer_allowed,
            cloud_restore_alert_dismissed,
            next_restore_attempt_id,
            ui,
        }
    }

    pub(crate) fn prepare_offline_cloud_check_retry(
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

    pub(crate) fn sync_ui(&mut self, deferred: &mut DeferredSender<Message>) {
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
        if self.ui.cloud_restore_issue != next_ui.cloud_restore_issue {
            deferred.queue(Message::CloudRestoreIssueChanged(next_ui.cloud_restore_issue));
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
        if self.ui.restore_state != next_ui.restore_state {
            deferred.queue(Message::RestoreStateChanged(next_ui.restore_state.clone()));
        }
        if self.ui.error != next_ui.error {
            deferred.queue(Message::ErrorChanged(next_ui.error));
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
    pub(crate) fn terms(context: TermsContext, progress: Option<OnboardingProgress>) -> Self {
        Self::Terms { context, error: None, progress }
    }

    pub(crate) fn apply_user_action(
        &mut self,
        action: OnboardingAction,
        cloud_restore_discovery: CloudRestoreDiscovery,
        restore_offer_allowed: &mut bool,
        restore_attempt_id: Option<u64>,
    ) -> TransitionCommand {
        let current = std::mem::replace(self, Self::Welcome { error: None });

        let (next, command) = match (current, action) {
            (Self::Welcome { .. }, OnboardingAction::ContinueFromWelcome) => {
                (Self::BitcoinChoice { error: None }, TransitionCommand::None)
            }
            (
                Self::BitcoinChoice { .. },
                OnboardingAction::SelectHasBitcoin { has_bitcoin: true },
            ) => (Self::StorageChoice { error: None }, TransitionCommand::None),
            (
                Self::BitcoinChoice { .. },
                OnboardingAction::SelectHasBitcoin { has_bitcoin: false },
            ) => {
                *restore_offer_allowed = false;
                (
                    Self::BitcoinChoice { error: None },
                    TransitionCommand::CreateWallet(OnboardingBranch::NewUser),
                )
            }
            (
                Self::StorageChoice { .. },
                OnboardingAction::SelectStorage { selection: OnboardingStorageSelection::Exchange },
            ) => {
                *restore_offer_allowed = false;
                (
                    Self::StorageChoice { error: None },
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
            ) => (Self::SoftwareImport { error: None }, TransitionCommand::None),
            (Self::SoftwareImport { .. }, OnboardingAction::CreateSoftwareWallet) => {
                *restore_offer_allowed = false;
                (
                    Self::SoftwareImport { error: None },
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
                    discovery: cloud_restore_discovery.clone(),
                },
            ),
            (
                Self::CloudBackup(CloudBackupFlow::CreatedWallet(mut flow)),
                OnboardingAction::CloudBackupEnabled,
            ) => {
                flow.cloud_backup_enabled = true;
                (
                    Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow)),
                    TransitionCommand::None,
                )
            }
            (
                Self::CloudBackup(CloudBackupFlow::SoftwareImport { wallet_id }),
                OnboardingAction::CloudBackupEnabled,
            ) => (
                Self::CloudBackupSuccess(CloudBackupFlow::SoftwareImport { wallet_id }),
                TransitionCommand::None,
            ),
            (
                Self::CloudBackup(CloudBackupFlow::HardwareImport { wallet_id }),
                OnboardingAction::CloudBackupEnabled,
            ) => (
                Self::CloudBackupSuccess(CloudBackupFlow::HardwareImport { wallet_id }),
                TransitionCommand::None,
            ),
            (
                Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow)),
                OnboardingAction::ContinueFromCloudBackupSuccess,
            ) => (Self::BackupWallet(flow), TransitionCommand::None),
            (
                Self::CloudBackupSuccess(
                    CloudBackupFlow::SoftwareImport { wallet_id }
                    | CloudBackupFlow::HardwareImport { wallet_id },
                ),
                OnboardingAction::ContinueFromCloudBackupSuccess,
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
                            Some(OnboardingProgress::from(flow)),
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
                        Some(OnboardingProgress::from(flow)),
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
            (Self::BitcoinChoice { .. }, OnboardingAction::OpenCloudRestore) => (
                Self::restore_entry_for(cloud_restore_discovery, RestoreOrigin::BitcoinChoice),
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
                let attempt_id =
                    restore_attempt_id.expect("restore attempt id required for StartRestore");
                (
                    Self::Restoring { origin, attempt_id, flow: CloudBackupRestoreFlow::Finding },
                    TransitionCommand::StartRestore { attempt_id },
                )
            }
            (Self::RestoreOffer { origin, .. }, OnboardingAction::SkipRestore) => {
                *restore_offer_allowed = false;
                (origin.flow_state(), TransitionCommand::None)
            }
            (Self::RestoreOffer { origin, .. }, OnboardingAction::Back) => {
                (origin.flow_state(), TransitionCommand::None)
            }
            (Self::RestoreOffline { origin }, OnboardingAction::ContinueWithoutCloudRestore) => {
                (origin.flow_state_after_restore_unavailable(), TransitionCommand::None)
            }
            (
                Self::RestoreUnavailable { origin },
                OnboardingAction::ContinueWithoutCloudRestore,
            ) => (origin.flow_state_after_restore_unavailable(), TransitionCommand::None),
            (Self::RestoreFailed { origin, .. }, OnboardingAction::RetryRestore) => {
                let attempt_id =
                    restore_attempt_id.expect("restore attempt id required for RetryRestore");
                (
                    Self::Restoring { origin, attempt_id, flow: CloudBackupRestoreFlow::Finding },
                    TransitionCommand::StartRestore { attempt_id },
                )
            }
            (Self::RestoreFailed { origin, .. }, OnboardingAction::SkipRestore) => {
                *restore_offer_allowed = false;
                (origin.flow_state(), TransitionCommand::None)
            }
            (
                Self::RestoreComplete { origin, .. },
                OnboardingAction::ContinueFromRestoreComplete,
            ) => {
                let _ = origin;
                (Self::terms(TermsContext::SelectLatestOrNew, None), TransitionCommand::None)
            }
            (mut terms @ Self::Terms { .. }, OnboardingAction::AcceptTerms) => {
                let command = terms.accept_terms();
                (terms, command)
            }
            (Self::BitcoinChoice { .. }, OnboardingAction::Back) => {
                (Self::Welcome { error: None }, TransitionCommand::None)
            }
            (Self::StorageChoice { .. }, OnboardingAction::Back) => {
                (Self::BitcoinChoice { error: None }, TransitionCommand::None)
            }
            (Self::SoftwareImport { .. }, OnboardingAction::Back) => {
                (Self::StorageChoice { error: None }, TransitionCommand::None)
            }
            (Self::HardwareImport, OnboardingAction::Back) => {
                (Self::StorageChoice { error: None }, TransitionCommand::None)
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

    pub(crate) fn accept_terms(&mut self) -> TransitionCommand {
        let Self::Terms { context, progress, .. } = self else {
            return TransitionCommand::None;
        };

        let context = context.clone();
        let progress = progress.clone();

        let target = context.completion_target();
        *self = Self::Terms { context, error: None, progress };
        TransitionCommand::CompleteOnboarding(target)
    }

    pub(crate) fn apply_event(
        &mut self,
        event: InternalEvent,
        cloud_restore_discovery: &mut CloudRestoreDiscovery,
        restore_offer_allowed: bool,
    ) {
        if let InternalEvent::CloudCheckFinished(outcome) = &event {
            *cloud_restore_discovery = CloudRestoreDiscovery::from(outcome.clone());
        }

        let current = std::mem::replace(self, Self::Welcome { error: None });

        let next = match (current, event) {
            (
                Self::CloudCheck { origin },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) => Self::RestoreOffer { origin, error: None },
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
                Self::RestoreOffer { origin: RestoreOrigin::Welcome, error: None }
            }
            (
                Self::BitcoinChoice { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::BitcoinChoice, error: None }
            }
            (
                Self::StorageChoice { .. },
                InternalEvent::CloudCheckFinished(CloudCheckOutcome::BackupFound(_)),
            ) if restore_offer_allowed => {
                Self::RestoreOffer { origin: RestoreOrigin::StorageChoice, error: None }
            }
            (state, InternalEvent::CloudCheckFinished(_)) => state,
            (
                Self::Restoring { origin, attempt_id, .. },
                InternalEvent::RestoreProgress { attempt_id: event_attempt_id, flow },
            ) if attempt_id == event_attempt_id => Self::Restoring { origin, attempt_id, flow },
            (
                Self::Restoring { origin, attempt_id, .. },
                InternalEvent::RestoreComplete { attempt_id: event_attempt_id, report },
            ) if attempt_id == event_attempt_id => Self::RestoreComplete { origin, report },
            (
                Self::Restoring { origin, attempt_id, .. },
                InternalEvent::RestoreNoBackupFound { attempt_id: event_attempt_id },
            ) if attempt_id == event_attempt_id => {
                *cloud_restore_discovery = CloudRestoreDiscovery::NoBackupFound;
                Self::RestoreUnavailable { origin }
            }
            (
                Self::Restoring { origin, attempt_id, .. },
                InternalEvent::RestoreFailed { attempt_id: event_attempt_id, failure },
            ) if attempt_id == event_attempt_id => Self::RestoreFailed { origin, failure },
            (state, InternalEvent::RestoreProgress { .. }) => state,
            (state, InternalEvent::RestoreComplete { .. }) => state,
            (state, InternalEvent::RestoreNoBackupFound { .. }) => state,
            (state, InternalEvent::RestoreFailed { .. }) => state,
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
            ) => {
                warn!("Onboarding wallet creation failed for new user: {error}");
                Self::BitcoinChoice { error: Some(OnboardingError::WalletCreationFailed) }
            }
            (
                Self::StorageChoice { .. },
                InternalEvent::WalletCreationFailed { branch: OnboardingBranch::Exchange, error },
            ) => {
                warn!("Onboarding wallet creation failed for exchange flow: {error}");
                Self::StorageChoice { error: Some(OnboardingError::WalletCreationFailed) }
            }
            (
                Self::SoftwareImport { .. },
                InternalEvent::WalletCreationFailed {
                    branch: OnboardingBranch::SoftwareCreate,
                    error,
                },
            ) => {
                warn!("Onboarding software wallet creation failed: {error}");
                Self::SoftwareImport { error: Some(OnboardingError::WalletCreationFailed) }
            }
            (Self::Terms { context, progress, .. }, InternalEvent::CompletionFailed { error }) => {
                warn!("Onboarding completion failed: {error}");
                Self::Terms { context, error: Some(OnboardingError::CompletionFailed), progress }
            }
            (state, event) => {
                warn!("Onboarding: invalid event={event:?} flow={state:?}");
                state
            }
        };

        *self = next;
    }

    pub(crate) fn is_restore_event_current(&self, event: &InternalEvent) -> bool {
        let event_attempt_id = match event {
            InternalEvent::RestoreProgress { attempt_id, .. }
            | InternalEvent::RestoreComplete { attempt_id, .. }
            | InternalEvent::RestoreNoBackupFound { attempt_id }
            | InternalEvent::RestoreFailed { attempt_id, .. } => *attempt_id,
            _ => return false,
        };

        self.is_restore_attempt_current(event_attempt_id)
    }

    pub(crate) fn is_restore_attempt_current(&self, event_attempt_id: u64) -> bool {
        matches!(self, Self::Restoring { attempt_id, .. } if *attempt_id == event_attempt_id)
    }

    pub(crate) fn ui_state(
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
            Self::RestoreOffer { error, .. } => {
                state.step = OnboardingStep::RestoreOffer;
                state.error = *error;
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
            Self::Restoring { flow, .. } => {
                state.step = OnboardingStep::Restoring;
                state.restore_state = OnboardingRestoreState::Restoring(flow.clone());
                state
            }
            Self::RestoreComplete { report, .. } => {
                state.step = OnboardingStep::RestoreComplete;
                state.restore_state = OnboardingRestoreState::Complete(report.clone());
                state
            }
            Self::RestoreFailed { failure, .. } => {
                state.step = OnboardingStep::RestoreFailed;
                state.restore_state = OnboardingRestoreState::Failed { failure: *failure };
                state
            }
            Self::Welcome { error } => {
                state.step = OnboardingStep::Welcome;
                state.error = *error;
                state
            }
            Self::BitcoinChoice { error } => {
                state.step = OnboardingStep::BitcoinChoice;
                state.error = *error;
                state
            }
            Self::StorageChoice { error } => {
                state.step = OnboardingStep::StorageChoice;
                state.error = *error;
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
            Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow)) => {
                Self::project_created_wallet(
                    OnboardingStep::CloudBackupSuccess,
                    flow,
                    cloud_restore_discovery,
                    should_offer_cloud_restore,
                    cloud_restore_alert_visible,
                )
            }
            Self::CloudBackupSuccess(CloudBackupFlow::SoftwareImport { .. }) => {
                state.step = OnboardingStep::CloudBackupSuccess;
                state.branch = Some(OnboardingBranch::SoftwareImport);
                state
            }
            Self::CloudBackupSuccess(CloudBackupFlow::HardwareImport { .. }) => {
                state.step = OnboardingStep::CloudBackupSuccess;
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
            Self::SoftwareImport { error } => {
                state.step = OnboardingStep::SoftwareImport;
                state.branch = Some(OnboardingBranch::SoftwareImport);
                state.error = *error;
                state
            }
            Self::Terms { error, .. } => {
                state.step = OnboardingStep::Terms;
                state.error = *error;
                state
            }
        }
    }

    pub(crate) fn current_wallet_id(&self) -> Option<WalletId> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::SecretWords(flow)
            | Self::ExchangeFunding(flow) => Some(flow.wallet_id.clone()),
            Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow)) => Some(flow.wallet_id.clone()),
            Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow)) => {
                Some(flow.wallet_id.clone())
            }
            Self::CloudBackup(
                CloudBackupFlow::SoftwareImport { wallet_id }
                | CloudBackupFlow::HardwareImport { wallet_id },
            ) => Some(wallet_id.clone()),
            Self::CloudBackupSuccess(
                CloudBackupFlow::SoftwareImport { wallet_id }
                | CloudBackupFlow::HardwareImport { wallet_id },
            ) => Some(wallet_id.clone()),
            Self::Terms { context: TermsContext::SelectWallet { wallet_id, .. }, .. } => {
                Some(wallet_id.clone())
            }
            _ => None,
        }
    }

    pub(crate) fn word_validator(&self) -> Option<Arc<WordValidator>> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow))
            | Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow))
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
            CloudRestoreDiscovery::BackupFound(_) => Self::RestoreOffer { origin, error: None },
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
                Self::RestoreOffer { origin, error: None }
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
            cloud_restore_issue: cloud_restore_discovery.issue(),
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
            cloud_restore_issue: cloud_restore_discovery.issue(),
            cloud_restore_provider_hint: cloud_restore_discovery.provider_hint(),
            should_offer_cloud_restore,
            cloud_restore_alert_visible,
            restore_state: OnboardingRestoreState::Idle,
            error: None,
        }
    }

    pub(crate) fn persisted_progress(&self) -> Option<OnboardingProgress> {
        match self {
            Self::CreatingWallet(flow)
            | Self::BackupWallet(flow)
            | Self::CloudBackup(CloudBackupFlow::CreatedWallet(flow))
            | Self::CloudBackupSuccess(CloudBackupFlow::CreatedWallet(flow))
            | Self::SecretWords(flow)
            | Self::ExchangeFunding(flow) => Some(OnboardingProgress::from(flow.clone())),
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
                | Self::HardwareImport
                | Self::SoftwareImport { .. }
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
    fn completion_target(&self) -> CompletionTarget {
        match self {
            Self::SelectLatestOrNew => CompletionTarget::SelectLatestOrNew,
            Self::SelectWallet { wallet_id, post_onboarding } => CompletionTarget::SelectWallet {
                wallet_id: wallet_id.clone(),
                post_onboarding: *post_onboarding,
            },
        }
    }
}

impl CloudRestoreDiscovery {
    pub(crate) fn ui_state(&self) -> OnboardingCloudRestoreState {
        match self {
            Self::Checking => OnboardingCloudRestoreState::Checking,
            Self::BackupFound(_) => OnboardingCloudRestoreState::BackupFound,
            Self::NoBackupFound => OnboardingCloudRestoreState::NoBackupFound,
            Self::Inconclusive(_) => OnboardingCloudRestoreState::Inconclusive,
        }
    }

    fn issue(&self) -> Option<CloudCheckIssue> {
        match self {
            Self::Inconclusive(issue) => Some(*issue),
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
            Self::Welcome => FlowState::Welcome { error: None },
            Self::BitcoinChoice => FlowState::BitcoinChoice { error: None },
            Self::StorageChoice => FlowState::StorageChoice { error: None },
            Self::HardwareImport => FlowState::HardwareImport,
            Self::SoftwareImport => FlowState::SoftwareImport { error: None },
        }
    }

    fn flow_state_after_restore_unavailable(self) -> FlowState {
        self.flow_state()
    }
}

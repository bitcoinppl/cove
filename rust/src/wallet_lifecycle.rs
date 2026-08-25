//! Process-wide ownership for persistent wallet actors and destructive operations

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};

use act_zero::{Actor, Addr, AddrLike as _, call};
use futures::{FutureExt as _, StreamExt as _, channel::oneshot, future::BoxFuture, stream};
use parking_lot::Mutex;
use rand::RngExt as _;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    app::reconcile::{AppStateReconcileMessage, Updater},
    discovery_scanner::WalletDiscoveryScanner,
    manager::wallet_manager::{WalletManagerError, actor::WalletActor},
    wallet::metadata::WalletId,
};

const INITIAL_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);
const RETRY_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(20);
const ORDINARY_CLOSE_RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// Opaque authorization for one blocked shutdown retry
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ShutdownAttemptId {
    /// Random process-local attempt identifier
    pub value: String,
}

impl ShutdownAttemptId {
    fn new() -> Self {
        Self { value: hex::encode(rand::rng().random::<[u8; 16]>()) }
    }
}

/// Deadline selected for a terminal wallet actor request
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ShutdownDeadlineTier {
    /// Five-second first attempt
    Initial,
    /// Twenty-second authorized retry
    Retry,
}

impl ShutdownDeadlineTier {
    const fn duration(self) -> Duration {
        match self {
            Self::Initial => INITIAL_SHUTDOWN_DEADLINE,
            Self::Retry => RETRY_SHUTDOWN_DEADLINE,
        }
    }
}

/// Persistent actor kind that did not reach terminal shutdown
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletActorKind {
    /// Main wallet actor
    Wallet,
    /// Alternate-address discovery actor
    Discovery,
}

/// Typed failure from the process wallet lifecycle owner
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum, thiserror::Error)]
#[uniffi::export(Display)]
pub enum WalletLifecycleFailure {
    /// The shared Tokio runtime is not initialized
    #[error("runtime is unavailable")]
    RuntimeUnavailable,
    /// A synchronous runtime bridge was called from a Tokio runtime thread
    #[error("synchronous lifecycle call cannot run on a Tokio runtime thread")]
    RuntimeThreadCall,
    /// Another destructive operation currently owns the lifecycle coordinator
    #[error("wallet lifecycle coordinator is busy")]
    CoordinatorBusy,
    /// Another construction is already loading this wallet
    #[error("wallet construction is already in progress")]
    ConstructionInProgress {
        /// Wallet whose construction is already running
        wallet_id: WalletId,
    },
    /// A released manager still owns actors that are reaching terminal shutdown
    #[error("wallet manager is closing")]
    ManagerClosing {
        /// Wallet whose previous manager is closing
        wallet_id: WalletId,
    },
    /// One or more actors can still write wallet state
    #[error("wallet actor shutdown is blocked")]
    ShutdownBlocked {
        /// Identifier required by Retry or Cancel
        attempt_id: ShutdownAttemptId,
        /// Actors that did not reach terminal shutdown
        actors: Vec<WalletActorKind>,
        /// Deadline used by this attempt
        deadline_tier: ShutdownDeadlineTier,
    },
    /// A manager could not resume after a cancelled or failed quiescence
    #[error("wallet manager requires process restart")]
    ManagerRecoveryRequired {
        /// Wallet whose manager cannot safely resume
        wallet_id: WalletId,
    },
    /// Cloud Backup local writers could not return to a safe runtime state
    #[error("cloud backup requires process restart")]
    CloudBackupRecoveryRequired,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum CoordinatorPhase {
    Available,
    PreparingDeletion(WalletId),
    PreparingFullWipe,
    Deleting(WalletId),
    Wiping,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum DestructiveIntent {
    Delete(WalletId),
    FullWipe,
}

#[derive(Debug)]
struct RegisteredActors {
    wallet_id: WalletId,
    wallet: Addr<WalletActor>,
    discovery: Option<Addr<WalletDiscoveryScanner>>,
    lifecycle: WalletManagerLifecycleToken,
    state: RegistrationState,
    ordinary_close_retry: Option<u64>,
    next_ordinary_close_retry_id: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RegistrationState {
    Active,
    OrdinaryClosePending,
    OrdinaryClosing,
    DestructiveClosing { resume: ResumedRegistrationState },
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ResumedRegistrationState {
    Active,
    OrdinaryClosePending,
}

impl From<ResumedRegistrationState> for RegistrationState {
    fn from(state: ResumedRegistrationState) -> Self {
        match state {
            ResumedRegistrationState::Active => Self::Active,
            ResumedRegistrationState::OrdinaryClosePending => Self::OrdinaryClosePending,
        }
    }
}

#[derive(Debug)]
struct OrdinaryCloseTarget {
    registration_id: u64,
    wallet: Addr<WalletActor>,
    discovery: Option<Addr<WalletDiscoveryScanner>>,
    deadline_tier: ShutdownDeadlineTier,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum OrdinaryCloseOutcome {
    Closed,
    Retryable,
    RecoveryRequired,
}

#[derive(Debug)]
enum OrdinaryCloseRetryClaim {
    Wait,
    Claimed(OrdinaryCloseTarget),
    Finished,
}

fn cache_clear_after_ordinary_close(
    outcome: OrdinaryCloseOutcome,
    wallet_id: &WalletId,
) -> Option<WalletId> {
    match outcome {
        OrdinaryCloseOutcome::RecoveryRequired => Some(wallet_id.clone()),
        OrdinaryCloseOutcome::Closed | OrdinaryCloseOutcome::Retryable => None,
    }
}

#[derive(Debug)]
struct DestructiveCloseTarget {
    registration_id: u64,
    wallet_id: WalletId,
    wallet: Addr<WalletActor>,
    discovery: Option<Addr<WalletDiscoveryScanner>>,
    had_discovery: bool,
    lifecycle: WalletManagerLifecycleToken,
}

#[derive(Debug)]
enum DestructiveClaim {
    Wait,
    Claimed(Vec<DestructiveCloseTarget>),
}

#[derive(Debug)]
struct CoordinatorData {
    phase: CoordinatorPhase,
    pending_preparation: Option<PendingPreparation>,
    active_constructions: usize,
    constructing_wallets: HashSet<WalletId>,
    active_persistence_operations: usize,
    next_registration_id: u64,
    actors: HashMap<u64, RegisteredActors>,
    retries: HashMap<ShutdownAttemptId, DestructiveIntent>,
    next_preparation_id: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PendingPreparation {
    id: u64,
    intent: DestructiveIntent,
    tier: ShutdownDeadlineTier,
    retry: Option<ShutdownAttemptId>,
}

#[derive(Debug)]
enum ShutdownActorsFailure {
    Blocked(Vec<WalletActorKind>),
    RecoveryRequired(WalletId),
}

impl Default for CoordinatorData {
    fn default() -> Self {
        Self {
            phase: CoordinatorPhase::Available,
            pending_preparation: None,
            active_constructions: 0,
            constructing_wallets: HashSet::new(),
            active_persistence_operations: 0,
            next_registration_id: 0,
            actors: HashMap::new(),
            retries: HashMap::new(),
            next_preparation_id: 0,
        }
    }
}

/// Process-wide persistent wallet lifecycle owner
#[derive(Debug, Default)]
pub(crate) struct WalletLifecycleCoordinator {
    data: Mutex<CoordinatorData>,
    changed: Notify,
}

static COORDINATOR: LazyLock<WalletLifecycleCoordinator> =
    LazyLock::new(WalletLifecycleCoordinator::default);

#[derive(Debug, Clone)]
enum OperationScope {
    Wallet(WalletId),
    Unscoped,
}

thread_local! {
    static ACTIVE_OPERATION_SCOPES: RefCell<Vec<(u64, OperationScope)>> = const { RefCell::new(Vec::new()) };
}

static NEXT_OPERATION_SCOPE_ID: AtomicU64 = AtomicU64::new(0);

fn operation_scope_allows(wallet_id: Option<&WalletId>) -> bool {
    ACTIVE_OPERATION_SCOPES.with(|scopes| {
        scopes.borrow().iter().any(|(_, scope)| match (scope, wallet_id) {
            (OperationScope::Unscoped, _) | (OperationScope::Wallet(_), None) => true,
            (OperationScope::Wallet(active), Some(requested)) => active == requested,
        })
    })
}

fn register_operation_scope(scope: OperationScope) -> u64 {
    let id = NEXT_OPERATION_SCOPE_ID.fetch_add(1, Ordering::Relaxed);
    ACTIVE_OPERATION_SCOPES.with(|scopes| scopes.borrow_mut().push((id, scope)));
    id
}

fn release_operation_scope(id: u64) {
    ACTIVE_OPERATION_SCOPES.with(|scopes| scopes.borrow_mut().retain(|(active, _)| *active != id));
}

impl WalletLifecycleCoordinator {
    /// Return the process lifecycle owner
    pub(crate) fn global() -> &'static Self {
        &COORDINATOR
    }

    /// Reserve construction before a persistent wallet store is opened
    pub(crate) fn begin_construction(
        &'static self,
        wallet_id: WalletId,
    ) -> Result<WalletConstructionPermit, WalletLifecycleFailure> {
        let retry_target = {
            let mut data = self.data.lock();
            let nested_operation = operation_scope_allows(Some(&wallet_id));
            match &data.phase {
                CoordinatorPhase::Available => {}
                _ if nested_operation => {}
                CoordinatorPhase::PreparingDeletion(id) | CoordinatorPhase::Deleting(id)
                    if id != &wallet_id => {}
                _ => return Err(WalletLifecycleFailure::CoordinatorBusy),
            }

            let registration = data
                .actors
                .iter()
                .find(|(_, actors)| actors.wallet_id == wallet_id)
                .map(|(registration_id, actors)| (*registration_id, actors.state));

            match registration {
                Some((
                    _,
                    RegistrationState::Active | RegistrationState::DestructiveClosing { .. },
                )) => return Err(WalletLifecycleFailure::CoordinatorBusy),
                Some((_, RegistrationState::RecoveryRequired)) => {
                    return Err(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id });
                }
                Some((_, RegistrationState::OrdinaryClosing)) => {
                    return Err(WalletLifecycleFailure::ManagerClosing { wallet_id });
                }
                Some((registration_id, RegistrationState::OrdinaryClosePending)) => {
                    if !cove_tokio::is_tokio_initialized() {
                        return Err(WalletLifecycleFailure::RuntimeUnavailable);
                    }

                    let actors = data
                        .actors
                        .get_mut(&registration_id)
                        .expect("ordinary close registration remains present");
                    actors.ordinary_close_retry = None;
                    actors.state = RegistrationState::OrdinaryClosing;
                    actors.lifecycle.mark_closing();
                    Self::ordinary_close_target(
                        registration_id,
                        actors,
                        ShutdownDeadlineTier::Retry,
                    )
                }
                None => {
                    if !data.constructing_wallets.insert(wallet_id.clone()) {
                        return Err(WalletLifecycleFailure::ConstructionInProgress { wallet_id });
                    }

                    data.active_constructions += 1;
                    let scope_id =
                        register_operation_scope(OperationScope::Wallet(wallet_id.clone()));

                    return Ok(WalletConstructionPermit {
                        coordinator: self,
                        expected_wallet_id: Some(wallet_id),
                        scope_id,
                        active: true,
                        _not_send: PhantomData,
                    });
                }
            }
        };

        self.spawn_ordinary_close(retry_target);

        Err(WalletLifecycleFailure::ManagerClosing { wallet_id })
    }

    /// Reserve construction when the wallet ID is derived by the persistent builder
    pub(crate) fn begin_unscoped_construction(
        &'static self,
    ) -> Result<WalletConstructionPermit, WalletLifecycleFailure> {
        let mut data = self.data.lock();
        if data.phase != CoordinatorPhase::Available && !operation_scope_allows(None) {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        data.active_constructions += 1;
        let scope_id = register_operation_scope(OperationScope::Unscoped);
        Ok(WalletConstructionPermit {
            coordinator: self,
            expected_wallet_id: None,
            scope_id,
            active: true,
            _not_send: PhantomData,
        })
    }

    /// Reserve one persistent operation before it can open or write wallet storage
    pub(crate) fn begin_persistence_operation(
        &'static self,
        wallet_id: WalletId,
    ) -> Result<WalletPersistenceOperation, WalletLifecycleFailure> {
        let mut data = self.data.lock();
        let nested_operation = operation_scope_allows(Some(&wallet_id));
        match &data.phase {
            CoordinatorPhase::Available => {}
            _ if nested_operation => {}
            CoordinatorPhase::PreparingDeletion(id) | CoordinatorPhase::Deleting(id)
                if id != &wallet_id => {}
            _ => return Err(WalletLifecycleFailure::CoordinatorBusy),
        }

        data.active_persistence_operations += 1;
        let scope_id = register_operation_scope(OperationScope::Wallet(wallet_id));
        Ok(WalletPersistenceOperation {
            coordinator: self,
            scope_id: Some(scope_id),
            active: true,
            _not_send: PhantomData,
        })
    }

    fn terminal_payjoin_authority(
        &'static self,
        wallet_id: WalletId,
    ) -> TerminalPayjoinPersistenceAuthority {
        TerminalPayjoinPersistenceAuthority { coordinator: self, wallet_id }
    }

    fn begin_terminal_payjoin_persistence(
        &'static self,
        authority: &TerminalPayjoinPersistenceAuthority,
        wallet_id: &WalletId,
    ) -> Result<WalletPersistenceOperation, WalletLifecycleFailure> {
        if !std::ptr::eq(self, authority.coordinator) || wallet_id != &authority.wallet_id {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        let mut data = self.data.lock();
        let authorized = match &data.phase {
            CoordinatorPhase::PreparingDeletion(quiescing_id) => quiescing_id == wallet_id,
            CoordinatorPhase::PreparingFullWipe => true,
            _ => false,
        };
        if !authorized {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        data.active_persistence_operations += 1;
        Ok(WalletPersistenceOperation {
            coordinator: self,
            scope_id: None,
            active: true,
            _not_send: PhantomData,
        })
    }

    /// Reserve a persistent operation whose wallet set is not known before its first read
    pub(crate) fn begin_unscoped_persistence_operation(
        &'static self,
    ) -> Result<WalletPersistenceOperation, WalletLifecycleFailure> {
        let mut data = self.data.lock();
        if data.phase != CoordinatorPhase::Available && !operation_scope_allows(None) {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        data.active_persistence_operations += 1;
        let scope_id = register_operation_scope(OperationScope::Unscoped);
        Ok(WalletPersistenceOperation {
            coordinator: self,
            scope_id: Some(scope_id),
            active: true,
            _not_send: PhantomData,
        })
    }

    /// Reserve database-unavailable recovery cleanup when no wallet writer exists
    pub(crate) fn begin_recovery_cleanup(
        &'static self,
    ) -> Result<RecoveryCleanupPermit, WalletLifecycleFailure> {
        let mut data = self.data.lock();
        if data.phase != CoordinatorPhase::Available
            || data.pending_preparation.is_some()
            || data.active_constructions != 0
            || data.active_persistence_operations != 0
            || !data.actors.is_empty()
        {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        data.phase = CoordinatorPhase::Wiping;
        Ok(RecoveryCleanupPermit { coordinator: self, active: true })
    }

    async fn wait_for_operations_to_drain(&self) {
        loop {
            let notified = self.changed.notified();
            let drained = {
                let data = self.data.lock();
                data.active_constructions == 0 && data.active_persistence_operations == 0
            };
            if drained {
                return;
            }

            notified.await;
        }
    }

    /// Enter deletion preparation and terminate every registered actor for the wallet
    pub(crate) async fn prepare_wallet_deletion(
        &'static self,
        wallet_id: WalletId,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<PreparedWalletLifecycle, WalletLifecycleFailure> {
        // keep destructive preparation alive after the request future is cancelled. The
        // coordinator-owned task retains the reservation until shutdown either produces the
        // prepared capability or releases the phase
        let retry = retry.cloned();
        cove_tokio::task::spawn(async move {
            self.prepare_wallet_deletion_owned(wallet_id, tier, retry.as_ref()).await
        })
        .await
        .expect("wallet deletion preparation task must not panic")
    }

    async fn prepare_wallet_deletion_owned(
        &'static self,
        wallet_id: WalletId,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<PreparedWalletLifecycle, WalletLifecycleFailure> {
        let reservation =
            self.reserve_preparation(DestructiveIntent::Delete(wallet_id.clone()), tier, retry)?;
        let mut reservation = reservation;
        reservation.enter_when_ordinary_closes_resolve().await?;
        self.wait_for_operations_to_drain().await;

        match self.shutdown_registered_actors(Some(&wallet_id), tier).await {
            Ok(()) => {
                self.wait_for_operations_to_drain().await;
                self.data.lock().phase = CoordinatorPhase::Deleting(wallet_id.clone());
                Ok(reservation.into_prepared(CoordinatorPhase::Deleting(wallet_id)))
            }
            Err(ShutdownActorsFailure::Blocked(actors)) => Err(self.finish_blocked_attempt(
                DestructiveIntent::Delete(wallet_id),
                actors,
                tier,
                retry,
            )),
            Err(ShutdownActorsFailure::RecoveryRequired(wallet_id)) => {
                self.release_failed_preparation();
                Err(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id })
            }
        }
    }

    /// Enter full-wipe preparation and terminate every registered wallet actor
    pub(crate) async fn prepare_full_wipe(
        &'static self,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<PreparedWalletLifecycle, WalletLifecycleFailure> {
        // keep destructive preparation alive after the request future is cancelled. The
        // coordinator-owned task retains the reservation until shutdown either produces the
        // prepared capability or releases the phase
        let retry = retry.cloned();
        cove_tokio::task::spawn(
            async move { self.prepare_full_wipe_owned(tier, retry.as_ref()).await },
        )
        .await
        .expect("full-wipe preparation task must not panic")
    }

    async fn prepare_full_wipe_owned(
        &'static self,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<PreparedWalletLifecycle, WalletLifecycleFailure> {
        let reservation = self.reserve_preparation(DestructiveIntent::FullWipe, tier, retry)?;
        let mut reservation = reservation;
        reservation.enter_when_ordinary_closes_resolve().await?;
        self.wait_for_operations_to_drain().await;

        match self.shutdown_registered_actors(None, tier).await {
            Ok(()) => {
                self.wait_for_operations_to_drain().await;
                self.data.lock().phase = CoordinatorPhase::Wiping;
                Ok(reservation.into_prepared(CoordinatorPhase::Wiping))
            }
            Err(ShutdownActorsFailure::Blocked(actors)) => {
                Err(self.finish_blocked_attempt(DestructiveIntent::FullWipe, actors, tier, retry))
            }
            Err(ShutdownActorsFailure::RecoveryRequired(wallet_id)) => {
                self.release_failed_preparation();
                Err(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id })
            }
        }
    }

    fn reserve_preparation(
        &'static self,
        intent: DestructiveIntent,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<PreparationReservation, WalletLifecycleFailure> {
        let mut data = self.data.lock();
        if data.phase != CoordinatorPhase::Available || data.pending_preparation.is_some() {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        Self::validate_preparation_attempt(&data, &intent, tier, retry)?;

        let id = data.next_preparation_id;
        data.next_preparation_id = data.next_preparation_id.wrapping_add(1);
        data.pending_preparation =
            Some(PendingPreparation { id, intent, tier, retry: retry.cloned() });

        Ok(PreparationReservation {
            coordinator: self,
            ownership: PreparationOwnership::Reserved { id },
        })
    }

    fn validate_preparation_attempt(
        data: &CoordinatorData,
        intent: &DestructiveIntent,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<(), WalletLifecycleFailure> {
        match (tier, retry) {
            (ShutdownDeadlineTier::Initial, None) => Ok(()),
            (ShutdownDeadlineTier::Retry, Some(attempt_id))
                if data.retries.get(attempt_id) == Some(intent) =>
            {
                Ok(())
            }
            _ => Err(WalletLifecycleFailure::CoordinatorBusy),
        }
    }

    async fn enter_reserved_preparation(
        &self,
        reservation_id: u64,
    ) -> Result<CoordinatorPhase, WalletLifecycleFailure> {
        loop {
            let notified = self.changed.notified();
            let entered_phase = {
                let mut data = self.data.lock();
                let Some(pending) = data
                    .pending_preparation
                    .as_ref()
                    .filter(|pending| pending.id == reservation_id)
                    .cloned()
                else {
                    return Err(WalletLifecycleFailure::CoordinatorBusy);
                };

                if data.phase != CoordinatorPhase::Available {
                    return Err(WalletLifecycleFailure::CoordinatorBusy);
                }

                let wallet_id = match &pending.intent {
                    DestructiveIntent::Delete(wallet_id) => Some(wallet_id),
                    DestructiveIntent::FullWipe => None,
                };
                let ordinary_close_active = data.actors.values().any(|actors| {
                    wallet_id.is_none_or(|id| id == &actors.wallet_id)
                        && actors.state == RegistrationState::OrdinaryClosing
                });
                if ordinary_close_active {
                    None
                } else {
                    Self::validate_preparation_attempt(
                        &data,
                        &pending.intent,
                        pending.tier,
                        pending.retry.as_ref(),
                    )?;
                    if let Some(attempt_id) = &pending.retry {
                        data.retries.remove(attempt_id);
                    }

                    let phase = match pending.intent {
                        DestructiveIntent::Delete(wallet_id) => {
                            CoordinatorPhase::PreparingDeletion(wallet_id)
                        }
                        DestructiveIntent::FullWipe => CoordinatorPhase::PreparingFullWipe,
                    };
                    data.phase = phase.clone();
                    data.pending_preparation = None;
                    Some(phase)
                }
            };

            if let Some(phase) = entered_phase {
                self.changed.notify_waiters();
                return Ok(phase);
            }

            notified.await;
        }
    }

    fn release_preparation_reservation(&self, reservation_id: u64) {
        let mut data = self.data.lock();
        if data.pending_preparation.as_ref().is_some_and(|pending| pending.id == reservation_id) {
            data.pending_preparation = None;
            self.changed.notify_waiters();
        }
    }

    fn finish_blocked_attempt(
        &self,
        intent: DestructiveIntent,
        actors: Vec<WalletActorKind>,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> WalletLifecycleFailure {
        let attempt_id = retry.cloned().unwrap_or_else(ShutdownAttemptId::new);
        {
            let mut data = self.data.lock();
            data.phase = CoordinatorPhase::Available;
            data.retries.insert(attempt_id.clone(), intent);
        }
        self.changed.notify_waiters();

        WalletLifecycleFailure::ShutdownBlocked { attempt_id, actors, deadline_tier: tier }
    }

    fn release_failed_preparation(&self) {
        let mut data = self.data.lock();
        data.phase = CoordinatorPhase::Available;
        self.changed.notify_waiters();
    }

    /// Revoke one retry authorization without changing wallet data
    pub(crate) fn cancel_attempt(&self, attempt_id: &ShutdownAttemptId) {
        self.data.lock().retries.remove(attempt_id);
    }

    fn try_claim_destructive_registrations(
        &self,
        wallet_id: Option<&WalletId>,
    ) -> Result<DestructiveClaim, ShutdownActorsFailure> {
        let mut data = self.data.lock();
        let targeted =
            |actors: &&RegisteredActors| wallet_id.is_none_or(|id| id == &actors.wallet_id);
        if data
            .actors
            .values()
            .filter(targeted)
            .any(|actors| actors.state == RegistrationState::OrdinaryClosing)
        {
            return Ok(DestructiveClaim::Wait);
        }

        let recovery_required = data
            .actors
            .values()
            .filter(targeted)
            .find(|actors| {
                matches!(
                    actors.state,
                    RegistrationState::RecoveryRequired
                        | RegistrationState::DestructiveClosing { .. }
                )
            })
            .map(|actors| actors.wallet_id.clone());
        if let Some(wallet_id) = recovery_required {
            return Err(ShutdownActorsFailure::RecoveryRequired(wallet_id));
        }

        let mut registrations = Vec::new();
        for (registration_id, actors) in data
            .actors
            .iter_mut()
            .filter(|(_, actors)| wallet_id.is_none_or(|id| id == &actors.wallet_id))
        {
            let resume = match actors.state {
                RegistrationState::Active => ResumedRegistrationState::Active,
                RegistrationState::OrdinaryClosePending => {
                    ResumedRegistrationState::OrdinaryClosePending
                }
                RegistrationState::OrdinaryClosing
                | RegistrationState::DestructiveClosing { .. }
                | RegistrationState::RecoveryRequired => {
                    return Err(ShutdownActorsFailure::RecoveryRequired(actors.wallet_id.clone()));
                }
            };
            actors.state = RegistrationState::DestructiveClosing { resume };
            actors.lifecycle.mark_closing();
            registrations.push(DestructiveCloseTarget {
                registration_id: *registration_id,
                wallet_id: actors.wallet_id.clone(),
                wallet: actors.wallet.clone(),
                discovery: actors.discovery.clone(),
                had_discovery: actors.discovery.is_some(),
                lifecycle: actors.lifecycle.clone(),
            });
        }

        Ok(DestructiveClaim::Claimed(registrations))
    }

    async fn shutdown_registered_actors(
        &'static self,
        wallet_id: Option<&WalletId>,
        tier: ShutdownDeadlineTier,
    ) -> Result<(), ShutdownActorsFailure> {
        let registrations = loop {
            let notified = self.changed.notified();
            match self.try_claim_destructive_registrations(wallet_id)? {
                DestructiveClaim::Wait => notified.await,
                DestructiveClaim::Claimed(registrations) => break registrations,
            }
        };

        let deadline = tokio::time::Instant::now() + tier.duration();
        let results = stream::iter(registrations)
            .map(|registration| async move {
                let (discovery_result, wallet_result) = tokio::join!(
                    async {
                        match registration.discovery {
                            Some(discovery) => {
                                terminal_discovery_shutdown(discovery, deadline).await
                            }
                            None => Ok(()),
                        }
                    },
                    terminal_wallet_shutdown(
                        Some(self.terminal_payjoin_authority(registration.wallet_id.clone())),
                        registration.wallet.clone(),
                        deadline,
                    ),
                );

                (
                    registration.registration_id,
                    registration.wallet_id,
                    registration.wallet,
                    registration.had_discovery,
                    registration.lifecycle,
                    discovery_result,
                    wallet_result,
                )
            })
            .buffer_unordered(usize::MAX)
            .collect::<Vec<_>>()
            .await;

        let mut failures = Vec::new();
        let mut recovery_required = None;
        let mut terminated = Vec::new();
        for (
            registration_id,
            wallet_id,
            wallet,
            had_discovery,
            lifecycle,
            discovery,
            wallet_result,
        ) in results
        {
            if discovery.is_err() {
                failures.push(WalletActorKind::Discovery);
            }
            if wallet_result.is_err() {
                failures.push(WalletActorKind::Wallet);
            }

            if discovery.is_ok() && wallet_result.is_ok() {
                lifecycle.mark_closed();
                terminated.push((registration_id, wallet_id));
                continue;
            }

            // discovery shutdown cancels its one-shot worker set, so any partial discovery
            // shutdown cannot return to the previous active manager state
            if had_discovery || wallet_result.is_ok() {
                recovery_required.get_or_insert(wallet_id.clone());
                if let Some(actors) = self.data.lock().actors.get_mut(&registration_id) {
                    actors.state = RegistrationState::RecoveryRequired;
                }
                Updater::send_update(AppStateReconcileMessage::ClearCachedWalletManager(wallet_id));
                continue;
            }

            if call!(wallet.resume_after_failed_quiesce()).await.is_ok() {
                let Some(resume) = self.restore_after_failed_destructive_quiesce(registration_id)
                else {
                    continue;
                };

                if resume == ResumedRegistrationState::OrdinaryClosePending {
                    self.arm_ordinary_close_retry(registration_id);
                }
            } else {
                recovery_required.get_or_insert(wallet_id.clone());
                if let Some(actors) = self.data.lock().actors.get_mut(&registration_id) {
                    actors.state = RegistrationState::RecoveryRequired;
                }
                Updater::send_update(AppStateReconcileMessage::ClearCachedWalletManager(wallet_id));
            }
        }

        if !terminated.is_empty() {
            let mut data = self.data.lock();
            for (registration_id, wallet_id) in terminated {
                data.actors.remove(&registration_id);
                Updater::send_update(AppStateReconcileMessage::ClearCachedWalletManager(wallet_id));
            }
        }
        self.changed.notify_waiters();

        failures.sort_unstable_by_key(|kind| match kind {
            WalletActorKind::Wallet => 0,
            WalletActorKind::Discovery => 1,
        });
        failures.dedup();
        if let Some(wallet_id) = recovery_required {
            return Err(ShutdownActorsFailure::RecoveryRequired(wallet_id));
        }

        if failures.is_empty() { Ok(()) } else { Err(ShutdownActorsFailure::Blocked(failures)) }
    }

    fn restore_after_failed_destructive_quiesce(
        &self,
        registration_id: u64,
    ) -> Option<ResumedRegistrationState> {
        let mut data = self.data.lock();
        let actors = data.actors.get_mut(&registration_id)?;
        let RegistrationState::DestructiveClosing { resume } = actors.state else {
            return None;
        };
        actors.state = resume.into();
        match resume {
            ResumedRegistrationState::Active => actors.lifecycle.mark_active(),
            ResumedRegistrationState::OrdinaryClosePending => actors.lifecycle.mark_closing(),
        }
        Some(resume)
    }

    fn release_construction(&self, wallet_id: Option<&WalletId>) {
        let mut data = self.data.lock();
        if let Some(wallet_id) = wallet_id {
            data.constructing_wallets.remove(wallet_id);
        }
        data.active_constructions = data.active_constructions.saturating_sub(1);
        self.changed.notify_waiters();
    }

    fn release_persistence_operation(&self) {
        let mut data = self.data.lock();
        data.active_persistence_operations = data.active_persistence_operations.saturating_sub(1);
        self.changed.notify_waiters();
    }

    fn release_preparation_phase(&self, expected: &CoordinatorPhase) {
        let mut data = self.data.lock();
        if &data.phase == expected {
            data.phase = CoordinatorPhase::Available;
        }
        self.changed.notify_waiters();
    }

    fn release_prepared(&self, expected: &CoordinatorPhase) {
        self.release_preparation_phase(expected);
    }

    fn ordinary_close_target(
        registration_id: u64,
        actors: &RegisteredActors,
        deadline_tier: ShutdownDeadlineTier,
    ) -> OrdinaryCloseTarget {
        OrdinaryCloseTarget {
            registration_id,
            wallet: actors.wallet.clone(),
            discovery: actors.discovery.clone(),
            deadline_tier,
        }
    }

    fn close_registration(&'static self, registration_id: u64) {
        let (target, arm_retry) = {
            let mut data = self.data.lock();
            let phase = data.phase.clone();
            let Some(actors) = data.actors.get_mut(&registration_id) else {
                return;
            };
            if Self::phase_owns_destructive_shutdown(&phase, &actors.wallet_id) {
                let arm_retry = match actors.state {
                    RegistrationState::Active => {
                        actors.state = RegistrationState::OrdinaryClosePending;
                        true
                    }
                    RegistrationState::DestructiveClosing {
                        resume: ResumedRegistrationState::Active,
                    } => {
                        actors.state = RegistrationState::DestructiveClosing {
                            resume: ResumedRegistrationState::OrdinaryClosePending,
                        };
                        true
                    }
                    RegistrationState::OrdinaryClosePending
                    | RegistrationState::DestructiveClosing {
                        resume: ResumedRegistrationState::OrdinaryClosePending,
                    } => actors.ordinary_close_retry.is_none(),
                    RegistrationState::OrdinaryClosing | RegistrationState::RecoveryRequired => {
                        false
                    }
                };
                if arm_retry {
                    actors.lifecycle.mark_closing();
                }
                self.changed.notify_waiters();
                (None, arm_retry)
            } else {
                if actors.state != RegistrationState::Active {
                    return;
                }

                actors.lifecycle.mark_closing();
                if !cove_tokio::is_tokio_initialized() {
                    actors.state = RegistrationState::OrdinaryClosePending;
                    self.changed.notify_waiters();
                    return;
                }

                actors.state = RegistrationState::OrdinaryClosing;
                (
                    Some(Self::ordinary_close_target(
                        registration_id,
                        actors,
                        ShutdownDeadlineTier::Initial,
                    )),
                    false,
                )
            }
        };

        if arm_retry {
            self.arm_ordinary_close_retry(registration_id);
        }
        if let Some(target) = target {
            self.spawn_ordinary_close(target);
        }
    }

    fn phase_owns_destructive_shutdown(phase: &CoordinatorPhase, wallet_id: &WalletId) -> bool {
        match phase {
            CoordinatorPhase::PreparingDeletion(target) | CoordinatorPhase::Deleting(target) => {
                target == wallet_id
            }
            CoordinatorPhase::PreparingFullWipe | CoordinatorPhase::Wiping => true,
            CoordinatorPhase::Available => false,
        }
    }

    fn spawn_ordinary_close(&'static self, target: OrdinaryCloseTarget) {
        cove_tokio::task::spawn(async move {
            let deadline = tokio::time::Instant::now() + target.deadline_tier.duration();
            let had_discovery = target.discovery.is_some();
            let (discovery_result, wallet_result) = tokio::join!(
                async {
                    match target.discovery {
                        Some(discovery) => terminal_discovery_shutdown(discovery, deadline).await,
                        None => Ok(()),
                    }
                },
                terminal_wallet_shutdown(None, target.wallet.clone(), deadline),
            );

            let outcome = if discovery_result.is_ok() && wallet_result.is_ok() {
                OrdinaryCloseOutcome::Closed
            } else if had_discovery || wallet_result.is_ok() {
                OrdinaryCloseOutcome::RecoveryRequired
            } else if call!(target.wallet.resume_after_failed_quiesce()).await.is_ok()
                && target.deadline_tier == ShutdownDeadlineTier::Initial
            {
                OrdinaryCloseOutcome::Retryable
            } else {
                OrdinaryCloseOutcome::RecoveryRequired
            };

            self.finish_ordinary_close(target.registration_id, outcome);
        });
    }

    fn arm_ordinary_close_retry(&'static self, registration_id: u64) {
        let Some(retry_id) = self.reserve_ordinary_close_retry(registration_id) else {
            return;
        };

        self.spawn_ordinary_close_retry(registration_id, retry_id);
    }

    fn reserve_ordinary_close_retry(&self, registration_id: u64) -> Option<u64> {
        let mut data = self.data.lock();
        let actors = data.actors.get_mut(&registration_id)?;
        if !matches!(
            actors.state,
            RegistrationState::OrdinaryClosePending
                | RegistrationState::DestructiveClosing {
                    resume: ResumedRegistrationState::OrdinaryClosePending,
                }
        ) || actors.ordinary_close_retry.is_some()
        {
            return None;
        }

        let retry_id = actors.next_ordinary_close_retry_id;
        actors.next_ordinary_close_retry_id = actors.next_ordinary_close_retry_id.wrapping_add(1);
        actors.ordinary_close_retry = Some(retry_id);
        Some(retry_id)
    }

    fn try_claim_ordinary_close_retry(
        &self,
        registration_id: u64,
        retry_id: u64,
    ) -> OrdinaryCloseRetryClaim {
        let mut data = self.data.lock();
        let phase = data.phase.clone();
        let Some(actors) = data.actors.get_mut(&registration_id) else {
            return OrdinaryCloseRetryClaim::Finished;
        };
        if actors.ordinary_close_retry != Some(retry_id) {
            return OrdinaryCloseRetryClaim::Finished;
        }

        match actors.state {
            RegistrationState::OrdinaryClosePending
                if Self::phase_owns_destructive_shutdown(&phase, &actors.wallet_id) =>
            {
                OrdinaryCloseRetryClaim::Wait
            }
            RegistrationState::DestructiveClosing {
                resume: ResumedRegistrationState::OrdinaryClosePending,
            } => OrdinaryCloseRetryClaim::Wait,
            RegistrationState::OrdinaryClosePending => {
                actors.ordinary_close_retry = None;
                actors.state = RegistrationState::OrdinaryClosing;
                actors.lifecycle.mark_closing();
                OrdinaryCloseRetryClaim::Claimed(Self::ordinary_close_target(
                    registration_id,
                    actors,
                    ShutdownDeadlineTier::Retry,
                ))
            }
            RegistrationState::Active
            | RegistrationState::OrdinaryClosing
            | RegistrationState::DestructiveClosing { resume: ResumedRegistrationState::Active }
            | RegistrationState::RecoveryRequired => {
                actors.ordinary_close_retry = None;
                OrdinaryCloseRetryClaim::Finished
            }
        }
    }

    fn spawn_ordinary_close_retry(&'static self, registration_id: u64, retry_id: u64) {
        cove_tokio::task::spawn(async move {
            tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF).await;

            loop {
                let notified = self.changed.notified();
                match self.try_claim_ordinary_close_retry(registration_id, retry_id) {
                    OrdinaryCloseRetryClaim::Wait => notified.await,
                    OrdinaryCloseRetryClaim::Claimed(target) => {
                        self.spawn_ordinary_close(target);
                        return;
                    }
                    OrdinaryCloseRetryClaim::Finished => return,
                }
            }
        });
    }

    fn finish_ordinary_close(&'static self, registration_id: u64, outcome: OrdinaryCloseOutcome) {
        let (clear_wallet, schedule_retry) = {
            let mut data = self.data.lock();
            let Some(actors) = data.actors.get_mut(&registration_id) else {
                return;
            };
            if actors.state != RegistrationState::OrdinaryClosing {
                return;
            }

            match outcome {
                OrdinaryCloseOutcome::Closed => {
                    actors.lifecycle.mark_closed();
                    let clear_wallet = cache_clear_after_ordinary_close(outcome, &actors.wallet_id);
                    data.actors.remove(&registration_id);
                    (clear_wallet, false)
                }
                OrdinaryCloseOutcome::Retryable => {
                    actors.state = RegistrationState::OrdinaryClosePending;
                    (None, true)
                }
                OrdinaryCloseOutcome::RecoveryRequired => {
                    actors.state = RegistrationState::RecoveryRequired;
                    (cache_clear_after_ordinary_close(outcome, &actors.wallet_id), false)
                }
            }
        };
        self.changed.notify_waiters();

        if schedule_retry {
            self.arm_ordinary_close_retry(registration_id);
        }

        if let Some(wallet_id) = clear_wallet {
            Updater::send_update(AppStateReconcileMessage::ClearCachedWalletManager(wallet_id));
        }
    }
}

/// Capability held while a persistent wallet manager is being built
#[derive(Debug)]
pub(crate) struct WalletConstructionPermit {
    coordinator: &'static WalletLifecycleCoordinator,
    expected_wallet_id: Option<WalletId>,
    scope_id: u64,
    active: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl WalletConstructionPermit {
    /// Atomically register the built actors before allowing destructive inventory
    pub(crate) fn register(
        mut self,
        wallet_id: WalletId,
        wallet: Addr<WalletActor>,
        discovery: Option<Addr<WalletDiscoveryScanner>>,
    ) -> (WalletActorRegistration, WalletManagerLifecycleToken) {
        debug_assert!(
            self.expected_wallet_id.as_ref().is_none_or(|expected| expected == &wallet_id),
            "construction registered a different wallet ID"
        );
        let lifecycle = WalletManagerLifecycleToken::new();
        let registration_id = {
            let mut data = self.coordinator.data.lock();
            let registration_id = data.next_registration_id;
            data.next_registration_id = data.next_registration_id.wrapping_add(1);
            data.actors.insert(
                registration_id,
                RegisteredActors {
                    wallet_id,
                    wallet,
                    discovery,
                    lifecycle: lifecycle.clone(),
                    state: RegistrationState::Active,
                    ordinary_close_retry: None,
                    next_ordinary_close_retry_id: 0,
                },
            );
            if let Some(wallet_id) = &self.expected_wallet_id {
                data.constructing_wallets.remove(wallet_id);
            }
            data.active_constructions = data.active_constructions.saturating_sub(1);
            registration_id
        };
        release_operation_scope(self.scope_id);
        self.active = false;
        self.coordinator.changed.notify_waiters();

        (
            WalletActorRegistration {
                _lease: Arc::new(WalletActorRegistrationInner { registration_id }),
            },
            lifecycle,
        )
    }
}

impl Drop for WalletConstructionPermit {
    fn drop(&mut self) {
        if self.active {
            release_operation_scope(self.scope_id);
            self.coordinator.release_construction(self.expected_wallet_id.as_ref());
        }
    }
}

/// Capability held by one external persistent operation
#[derive(Debug)]
pub(crate) struct WalletPersistenceOperation {
    coordinator: &'static WalletLifecycleCoordinator,
    scope_id: Option<u64>,
    active: bool,
    _not_send: PhantomData<Rc<()>>,
}

/// Capability for a Payjoin terminal marker written while one wallet is quiescing
#[derive(Debug, Clone)]
pub(crate) struct TerminalPayjoinPersistenceAuthority {
    coordinator: &'static WalletLifecycleCoordinator,
    wallet_id: WalletId,
}

impl TerminalPayjoinPersistenceAuthority {
    pub(crate) fn begin(
        &self,
        wallet_id: &WalletId,
    ) -> Result<WalletPersistenceOperation, WalletLifecycleFailure> {
        self.coordinator.begin_terminal_payjoin_persistence(self, wallet_id)
    }
}

/// Capability proving database-unavailable cleanup has no active wallet writer
#[derive(Debug)]
pub(crate) struct RecoveryCleanupPermit {
    coordinator: &'static WalletLifecycleCoordinator,
    active: bool,
}

impl Drop for RecoveryCleanupPermit {
    fn drop(&mut self) {
        if self.active {
            self.coordinator.release_prepared(&CoordinatorPhase::Wiping);
        }
    }
}

impl Drop for WalletPersistenceOperation {
    fn drop(&mut self) {
        if self.active {
            if let Some(scope_id) = self.scope_id {
                release_operation_scope(scope_id);
            }
            self.coordinator.release_persistence_operation();
        }
    }
}

#[derive(Debug)]
enum PreparationOwnership {
    Reserved { id: u64 },
    Entered { phase: CoordinatorPhase },
    Transferred,
}

#[derive(Debug)]
struct PreparationReservation {
    coordinator: &'static WalletLifecycleCoordinator,
    ownership: PreparationOwnership,
}

impl PreparationReservation {
    async fn enter_when_ordinary_closes_resolve(&mut self) -> Result<(), WalletLifecycleFailure> {
        let id = match &self.ownership {
            PreparationOwnership::Reserved { id } => *id,
            PreparationOwnership::Entered { .. } | PreparationOwnership::Transferred => {
                return Err(WalletLifecycleFailure::CoordinatorBusy);
            }
        };
        let phase = self.coordinator.enter_reserved_preparation(id).await?;
        self.ownership = PreparationOwnership::Entered { phase };
        Ok(())
    }

    fn into_prepared(mut self, final_phase: CoordinatorPhase) -> PreparedWalletLifecycle {
        debug_assert!(matches!(self.ownership, PreparationOwnership::Entered { .. }));
        self.ownership = PreparationOwnership::Transferred;
        PreparedWalletLifecycle { coordinator: self.coordinator, final_phase, active: true }
    }
}

impl Drop for PreparationReservation {
    fn drop(&mut self) {
        match &self.ownership {
            PreparationOwnership::Reserved { id } => {
                self.coordinator.release_preparation_reservation(*id)
            }
            PreparationOwnership::Entered { phase } => {
                self.coordinator.release_preparation_phase(phase)
            }
            PreparationOwnership::Transferred => {}
        }
    }
}

#[derive(Debug)]
struct WalletActorRegistrationInner {
    registration_id: u64,
}

impl Drop for WalletActorRegistrationInner {
    fn drop(&mut self) {
        WalletLifecycleCoordinator::global().close_registration(self.registration_id);
    }
}

/// RAII ownership attached to every persistent wallet manager clone
#[derive(Debug, Clone)]
pub(crate) struct WalletActorRegistration {
    _lease: Arc<WalletActorRegistrationInner>,
}

const MANAGER_ACTIVE: u8 = 0;
const MANAGER_CLOSING: u8 = 1;
const MANAGER_CLOSED: u8 = 2;

/// Shared closed-state gate held by the manager and the actor registry
#[derive(Debug, Clone)]
pub(crate) struct WalletManagerLifecycleToken {
    state: Arc<AtomicU8>,
}

impl WalletManagerLifecycleToken {
    fn new() -> Self {
        Self { state: Arc::new(AtomicU8::new(MANAGER_ACTIVE)) }
    }

    /// Reject actor or persistent-state access after terminal shutdown starts
    pub(crate) fn ensure_active(&self) -> Result<(), WalletManagerError> {
        if self.state.load(Ordering::Acquire) == MANAGER_ACTIVE {
            return Ok(());
        }

        Err(WalletManagerError::ManagerClosed)
    }

    fn mark_active(&self) {
        self.state.store(MANAGER_ACTIVE, Ordering::Release);
    }

    fn mark_closing(&self) {
        self.state.store(MANAGER_CLOSING, Ordering::Release);
    }

    fn mark_closed(&self) {
        self.state.store(MANAGER_CLOSED, Ordering::Release);
    }
}

/// Capability proving wallet actors and current persistent operations are stopped
#[derive(Debug)]
pub(crate) struct PreparedWalletLifecycle {
    coordinator: &'static WalletLifecycleCoordinator,
    final_phase: CoordinatorPhase,
    active: bool,
}

impl Drop for PreparedWalletLifecycle {
    fn drop(&mut self) {
        if self.active {
            self.coordinator.release_prepared(&self.final_phase);
        }
    }
}

enum TerminalReply {
    Completed(Result<(), String>),
    Cancelled,
}

async fn terminal_wallet_shutdown(
    authority: Option<TerminalPayjoinPersistenceAuthority>,
    actor: Addr<WalletActor>,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    terminal_shutdown(actor, deadline, move |actor| {
        async move {
            match authority {
                Some(authority) => actor.quiesce_for_terminal_shutdown(authority).await,
                None => actor.shutdown().await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        .boxed()
    })
    .await
}

async fn terminal_discovery_shutdown(
    actor: Addr<WalletDiscoveryScanner>,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    terminal_shutdown(actor, deadline, |actor| {
        async move { actor.shutdown().await.map(|_| ()).map_err(|error| error.to_string()) }.boxed()
    })
    .await
}

async fn terminal_shutdown<T>(
    actor: Addr<T>,
    deadline: tokio::time::Instant,
    quiesce: impl for<'a> FnOnce(&'a mut T) -> BoxFuture<'a, Result<(), String>> + Send + 'static,
) -> Result<(), ()>
where
    T: Actor,
{
    if actor.termination().now_or_never().is_some() {
        return Ok(());
    }

    let cancellation = CancellationToken::new();
    let request_cancellation = cancellation.clone();
    let (started_sender, started_receiver) = oneshot::channel();
    let (reply_sender, reply_receiver) = oneshot::channel();
    actor.send_mut(Box::new(move |actor| {
        async move {
            let _ = started_sender.send(());
            let result = tokio::select! {
                biased;
                () = request_cancellation.cancelled() => TerminalReply::Cancelled,
                result = quiesce(actor) => {
                    if request_cancellation.is_cancelled() {
                        TerminalReply::Cancelled
                    } else {
                        TerminalReply::Completed(result)
                    }
                }
            };
            let terminate = matches!(result, TerminalReply::Completed(Ok(())));
            let _ = reply_sender.send(result);
            terminate
        }
        .boxed()
    }));

    let started = tokio::select! {
        result = started_receiver => result.is_ok(),
        () = tokio::time::sleep_until(deadline) => false,
    };
    if !started {
        cancellation.cancel();
        return Err(());
    }

    let mut reply_receiver = reply_receiver;
    let reply = tokio::select! {
        result = &mut reply_receiver => result.ok(),
        () = tokio::time::sleep_until(deadline) => {
            cancellation.cancel();
            reply_receiver.await.ok()
        }
    };

    match reply {
        Some(TerminalReply::Completed(Ok(()))) => {
            actor.termination().await;
            Ok(())
        }
        Some(TerminalReply::Completed(Err(_))) | Some(TerminalReply::Cancelled) | None => Err(()),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{
        CoordinatorPhase, DestructiveIntent, ShutdownAttemptId, ShutdownDeadlineTier,
        TerminalPayjoinPersistenceAuthority, WalletLifecycleCoordinator, WalletLifecycleFailure,
    };
    use crate::wallet::metadata::WalletId;

    pub(crate) struct TerminalPreparation {
        coordinator: &'static WalletLifecycleCoordinator,
    }

    impl Drop for TerminalPreparation {
        fn drop(&mut self) {
            self.coordinator.release_failed_preparation();
        }
    }

    pub(crate) fn begin_wallet_deletion(
        wallet_id: WalletId,
    ) -> (TerminalPayjoinPersistenceAuthority, TerminalPreparation) {
        begin(DestructiveIntent::Delete(wallet_id.clone()), wallet_id)
    }

    pub(crate) fn begin_full_wipe(
        wallet_id: WalletId,
    ) -> (TerminalPayjoinPersistenceAuthority, TerminalPreparation) {
        begin(DestructiveIntent::FullWipe, wallet_id)
    }

    pub(crate) fn enter_wallet_deletion(
        coordinator: &'static WalletLifecycleCoordinator,
        wallet_id: WalletId,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<(), WalletLifecycleFailure> {
        enter(coordinator, DestructiveIntent::Delete(wallet_id), tier, retry)
    }

    fn begin(
        intent: DestructiveIntent,
        wallet_id: WalletId,
    ) -> (TerminalPayjoinPersistenceAuthority, TerminalPreparation) {
        let coordinator = WalletLifecycleCoordinator::global();
        enter(coordinator, intent, ShutdownDeadlineTier::Initial, None)
            .expect("terminal test preparation starts");

        (coordinator.terminal_payjoin_authority(wallet_id), TerminalPreparation { coordinator })
    }

    fn enter(
        coordinator: &'static WalletLifecycleCoordinator,
        intent: DestructiveIntent,
        tier: ShutdownDeadlineTier,
        retry: Option<&ShutdownAttemptId>,
    ) -> Result<(), WalletLifecycleFailure> {
        let mut data = coordinator.data.lock();
        if data.phase != CoordinatorPhase::Available || data.pending_preparation.is_some() {
            return Err(WalletLifecycleFailure::CoordinatorBusy);
        }

        WalletLifecycleCoordinator::validate_preparation_attempt(&data, &intent, tier, retry)?;
        if let Some(attempt_id) = retry {
            data.retries.remove(attempt_id);
        }

        data.phase = match intent {
            DestructiveIntent::Delete(wallet_id) => CoordinatorPhase::PreparingDeletion(wallet_id),
            DestructiveIntent::FullWipe => CoordinatorPhase::PreparingFullWipe,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use act_zero::{Actor, ActorResult, AddrLike as _, call};
    use futures::FutureExt as _;
    use tokio::sync::{Notify, oneshot};

    use super::{
        CoordinatorPhase, DestructiveClaim, DestructiveIntent, ORDINARY_CLOSE_RETRY_BACKOFF,
        OrdinaryCloseOutcome, RegistrationState, ResumedRegistrationState, ShutdownDeadlineTier,
        WalletActorKind, WalletLifecycleCoordinator, WalletLifecycleFailure,
        WalletManagerLifecycleToken, cache_clear_after_ordinary_close, terminal_shutdown,
        test_support::enter_wallet_deletion,
    };

    #[derive(Debug)]
    struct TestActor {
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl Actor for TestActor {
        async fn error(&mut self, _error: act_zero::ActorError) -> bool {
            false
        }
    }

    impl TestActor {
        async fn fail(&mut self) -> ActorResult<()> {
            Err("expected failure".into())
        }

        async fn probe(&mut self) -> ActorResult<()> {
            Ok(act_zero::Produces::Value(()))
        }

        async fn block(&mut self, started: oneshot::Sender<()>) -> ActorResult<()> {
            let _ = started.send(());
            self.release.notified().await;
            Ok(act_zero::Produces::Value(()))
        }
    }

    fn spawn_actor(release: Arc<Notify>) -> act_zero::Addr<TestActor> {
        crate::test_support::ensure_tokio_runtime();
        cove_tokio::task::spawn_actor(TestActor { release })
    }

    fn register_preview_wallet(
        coordinator: &'static WalletLifecycleCoordinator,
    ) -> (
        crate::wallet::metadata::WalletId,
        u64,
        crate::manager::wallet_manager::RustWalletManager,
        super::WalletActorRegistration,
    ) {
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        let manager = crate::manager::wallet_manager::RustWalletManager::preview_new_wallet();
        let wallet_id = manager.id.clone();
        let construction =
            coordinator.begin_construction(wallet_id.clone()).expect("construction starts");
        let (registration, _) =
            construction.register(wallet_id.clone(), manager.actor.clone(), None);
        let registration_id = registration._lease.registration_id;

        (wallet_id, registration_id, manager, registration)
    }

    fn mark_ordinary_closing(coordinator: &WalletLifecycleCoordinator, registration_id: u64) {
        let mut data = coordinator.data.lock();
        let actors = data.actors.get_mut(&registration_id).expect("registration exists");
        actors.state = RegistrationState::OrdinaryClosing;
        actors.lifecycle.mark_closing();
    }

    #[tokio::test]
    async fn ordinary_failed_call_does_not_terminate_actor() {
        let actor = spawn_actor(Arc::new(Notify::new()));

        assert!(call!(actor.fail()).await.is_err());
        call!(actor.probe()).await.expect("actor remains active");
    }

    #[tokio::test]
    async fn successful_terminal_request_awaits_termination() {
        let actor = spawn_actor(Arc::new(Notify::new()));
        let result = terminal_shutdown(
            actor.clone(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
            |_actor| Box::pin(async { Ok(()) }),
        )
        .await;

        assert_eq!(result, Ok(()));
        assert!(actor.termination().now_or_never().is_some());
    }

    #[tokio::test]
    async fn already_terminated_actor_is_idempotent_success() {
        let actor = spawn_actor(Arc::new(Notify::new()));
        let quiesce_calls = Arc::new(AtomicUsize::new(0));
        let first_calls = quiesce_calls.clone();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);

        terminal_shutdown(actor.clone(), deadline, move |_actor| {
            first_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })
        .await
        .expect("first terminal request succeeds");

        let second_calls = quiesce_calls.clone();
        terminal_shutdown(actor, deadline, move |_actor| {
            second_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })
        .await
        .expect("terminated actor is already safe");

        assert_eq!(quiesce_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_terminal_request_leaves_actor_active() {
        let actor = spawn_actor(Arc::new(Notify::new()));
        let result = terminal_shutdown(
            actor.clone(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
            |_actor| Box::pin(async { Err("blocked".to_string()) }),
        )
        .await;

        assert_eq!(result, Err(()));
        call!(actor.probe()).await.expect("actor remains active");
    }

    #[tokio::test]
    async fn cancelled_queued_terminal_request_cannot_terminate_actor_later() {
        let release = Arc::new(Notify::new());
        let actor = spawn_actor(release.clone());
        let (started_sender, started_receiver) = oneshot::channel();
        actor.send_mut(Box::new(move |actor| {
            Box::pin(async move {
                let _ = actor.block(started_sender).await;
                false
            })
        }));
        started_receiver.await.expect("blocking call starts");

        let result = terminal_shutdown(
            actor.clone(),
            tokio::time::Instant::now() + std::time::Duration::from_millis(10),
            |_actor| Box::pin(async { Ok(()) }),
        )
        .await;
        assert_eq!(result, Err(()));

        release.notify_one();
        call!(actor.probe()).await.expect("actor remains active");
    }

    #[tokio::test]
    async fn cancelled_running_terminal_request_cannot_terminate_actor_later() {
        let actor = spawn_actor(Arc::new(Notify::new()));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10);

        let result = terminal_shutdown(actor.clone(), deadline, |_actor| {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                Ok(())
            })
        })
        .await;

        assert_eq!(result, Err(()));
        call!(actor.probe()).await.expect("actor remains active");
    }

    #[test]
    fn same_wallet_construction_returns_typed_closing_state_without_waiting() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        mark_ordinary_closing(coordinator, registration_id);

        let started = std::time::Instant::now();
        assert_eq!(
            coordinator.begin_construction(wallet_id.clone()).err(),
            Some(WalletLifecycleFailure::ManagerClosing { wallet_id })
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "synchronous construction must not wait for actor shutdown"
        );

        coordinator.data.lock().actors.remove(&registration_id);
        std::mem::forget(registration);
        drop(manager);
    }

    #[tokio::test]
    async fn retryable_ordinary_close_is_retried_without_a_new_construction() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        mark_ordinary_closing(coordinator, registration_id);

        coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::Retryable);
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::OrdinaryClosePending)
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let notified = coordinator.changed.notified();
                if !coordinator.data.lock().actors.contains_key(&registration_id) {
                    break;
                }

                notified.await;
            }
        })
        .await
        .expect("the coordinator-owned retry closes the actors");

        assert!(coordinator.begin_construction(wallet_id).is_ok());

        std::mem::forget(registration);
        drop(manager);
    }

    #[tokio::test]
    async fn ordinary_close_retry_waits_for_failed_destructive_phase_to_release() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        {
            let mut data = coordinator.data.lock();
            let actors = data.actors.get_mut(&registration_id).expect("registration exists");
            actors.state = RegistrationState::OrdinaryClosePending;
            actors.lifecycle.mark_closing();
        }

        coordinator.arm_ordinary_close_retry(registration_id);
        let retry_id = coordinator
            .data
            .lock()
            .actors
            .get(&registration_id)
            .and_then(|actors| actors.ordinary_close_retry)
            .expect("ordinary close retry is armed");

        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");
        let DestructiveClaim::Claimed(targets) = coordinator
            .try_claim_destructive_registrations(Some(&wallet_id))
            .expect("destructive shutdown claims the pending registration")
        else {
            panic!("destructive shutdown must claim the registration")
        };
        assert_eq!(
            coordinator.restore_after_failed_destructive_quiesce(registration_id),
            Some(ResumedRegistrationState::OrdinaryClosePending)
        );
        tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
        {
            let data = coordinator.data.lock();
            let actors = data.actors.get(&registration_id).expect("registration remains pending");
            assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
            assert_eq!(actors.ordinary_close_retry, Some(retry_id));
            assert_eq!(actors.next_ordinary_close_retry_id, 1);
        }

        drop(targets);
        coordinator.release_failed_preparation();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let notified = coordinator.changed.notified();
                if !coordinator.data.lock().actors.contains_key(&registration_id) {
                    break;
                }

                notified.await;
            }
        })
        .await
        .expect("the retained retry closes after destructive ownership releases");

        assert!(coordinator.begin_construction(wallet_id).is_ok());

        std::mem::forget(registration);
        drop(manager);
    }

    async fn wait_for_registration_removal(
        coordinator: &'static WalletLifecycleCoordinator,
        registration_id: u64,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let notified = coordinator.changed.notified();
                if !coordinator.data.lock().actors.contains_key(&registration_id) {
                    break;
                }

                notified.await;
            }
        })
        .await
        .expect("ordinary close retry removes the registration");
    }

    #[tokio::test]
    async fn lease_drop_before_destructive_claim_survives_blocked_shutdown() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        let lifecycle = coordinator
            .data
            .lock()
            .actors
            .get(&registration_id)
            .expect("registration exists")
            .lifecycle
            .clone();
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");

        coordinator.close_registration(registration_id);
        std::mem::forget(registration);
        {
            let data = coordinator.data.lock();
            let actors = data.actors.get(&registration_id).expect("registration remains owned");
            assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
            assert_eq!(actors.ordinary_close_retry, Some(0));
            assert_eq!(actors.next_ordinary_close_retry_id, 1);
        }
        assert!(lifecycle.ensure_active().is_err());

        tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
        {
            let data = coordinator.data.lock();
            let actors = data.actors.get(&registration_id).expect("registration remains owned");
            assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
            assert_eq!(actors.ordinary_close_retry, Some(0));
        }

        let DestructiveClaim::Claimed(targets) = coordinator
            .try_claim_destructive_registrations(Some(&wallet_id))
            .expect("destructive shutdown claims the registration")
        else {
            panic!("destructive shutdown must claim the registration");
        };
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::DestructiveClosing {
                resume: ResumedRegistrationState::OrdinaryClosePending,
            })
        );
        drop(targets);

        assert_eq!(
            coordinator.restore_after_failed_destructive_quiesce(registration_id),
            Some(ResumedRegistrationState::OrdinaryClosePending)
        );
        assert!(lifecycle.ensure_active().is_err());
        let blocked = coordinator.finish_blocked_attempt(
            DestructiveIntent::Delete(wallet_id.clone()),
            vec![WalletActorKind::Wallet],
            ShutdownDeadlineTier::Initial,
            None,
        );
        assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

        wait_for_registration_removal(coordinator, registration_id).await;
        let construction = coordinator
            .begin_construction(wallet_id)
            .expect("wallet reconstruction follows ordinary close");
        drop(construction);
        drop(manager);
    }

    #[tokio::test]
    async fn lease_drop_after_destructive_claim_updates_failed_resume_target() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        let lifecycle = coordinator
            .data
            .lock()
            .actors
            .get(&registration_id)
            .expect("registration exists")
            .lifecycle
            .clone();
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");
        let DestructiveClaim::Claimed(targets) = coordinator
            .try_claim_destructive_registrations(Some(&wallet_id))
            .expect("destructive shutdown claims the registration")
        else {
            panic!("destructive shutdown must claim the registration");
        };
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::DestructiveClosing {
                resume: ResumedRegistrationState::Active,
            })
        );

        coordinator.close_registration(registration_id);
        std::mem::forget(registration);
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| (
                actors.state,
                actors.ordinary_close_retry,
                actors.next_ordinary_close_retry_id
            )),
            Some((
                RegistrationState::DestructiveClosing {
                    resume: ResumedRegistrationState::OrdinaryClosePending,
                },
                Some(0),
                1,
            ))
        );

        tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::DestructiveClosing {
                resume: ResumedRegistrationState::OrdinaryClosePending,
            })
        );
        drop(targets);

        assert_eq!(
            coordinator.restore_after_failed_destructive_quiesce(registration_id),
            Some(ResumedRegistrationState::OrdinaryClosePending)
        );
        assert!(lifecycle.ensure_active().is_err());
        let blocked = coordinator.finish_blocked_attempt(
            DestructiveIntent::Delete(wallet_id.clone()),
            vec![WalletActorKind::Wallet],
            ShutdownDeadlineTier::Initial,
            None,
        );
        assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

        wait_for_registration_removal(coordinator, registration_id).await;
        let construction = coordinator
            .begin_construction(wallet_id)
            .expect("wallet reconstruction follows ordinary close");
        drop(construction);
        drop(manager);
    }

    #[tokio::test]
    async fn lease_drop_after_failed_restore_marks_lifecycle_closing() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        let lifecycle = coordinator
            .data
            .lock()
            .actors
            .get(&registration_id)
            .expect("registration exists")
            .lifecycle
            .clone();
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");
        let DestructiveClaim::Claimed(targets) = coordinator
            .try_claim_destructive_registrations(Some(&wallet_id))
            .expect("destructive shutdown claims the registration")
        else {
            panic!("destructive shutdown must claim the registration");
        };

        assert_eq!(
            coordinator.restore_after_failed_destructive_quiesce(registration_id),
            Some(ResumedRegistrationState::Active)
        );
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::Active)
        );
        assert!(lifecycle.ensure_active().is_ok());

        coordinator.close_registration(registration_id);
        std::mem::forget(registration);
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::OrdinaryClosePending)
        );
        assert!(lifecycle.ensure_active().is_err());

        drop(targets);
        let blocked = coordinator.finish_blocked_attempt(
            DestructiveIntent::Delete(wallet_id.clone()),
            vec![WalletActorKind::Wallet],
            ShutdownDeadlineTier::Initial,
            None,
        );
        assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

        wait_for_registration_removal(coordinator, registration_id).await;
        let construction = coordinator
            .begin_construction(wallet_id)
            .expect("wallet reconstruction follows ordinary close");
        drop(construction);
        drop(manager);
    }

    #[test]
    fn successful_ordinary_close_does_not_clear_a_same_wallet_replacement() {
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();

        assert_eq!(
            cache_clear_after_ordinary_close(OrdinaryCloseOutcome::Closed, &wallet_id),
            None,
            "ordinary close was requested by the cache owner and must not send a stale clear"
        );
        assert_eq!(
            cache_clear_after_ordinary_close(OrdinaryCloseOutcome::RecoveryRequired, &wallet_id,),
            Some(wallet_id),
            "recovery-required state still invalidates the unsafe cached manager"
        );
    }

    #[test]
    fn unrecoverable_ordinary_close_requires_process_restart() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        mark_ordinary_closing(coordinator, registration_id);

        coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::RecoveryRequired);

        assert_eq!(
            coordinator.begin_construction(wallet_id.clone()).err(),
            Some(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id })
        );

        coordinator.data.lock().actors.remove(&registration_id);
        std::mem::forget(registration);
        drop(manager);
    }

    #[test]
    fn destructive_preparation_keeps_normal_close_persistence_available() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        mark_ordinary_closing(coordinator, registration_id);

        let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
        let deletion_wallet_id = wallet_id.clone();
        std::thread::spawn(move || {
            let result = cove_tokio::try_block_on(coordinator.prepare_wallet_deletion(
                deletion_wallet_id,
                ShutdownDeadlineTier::Initial,
                None,
            ))
            .expect("runtime bridge is available");
            result_sender.send(result.is_ok()).expect("send preparation result");
        });

        let reservation_started = (0..100).any(|_| {
            if coordinator.data.lock().pending_preparation.is_some() {
                return true;
            }

            std::thread::sleep(std::time::Duration::from_millis(1));
            false
        });
        assert!(reservation_started, "destructive preparation reserves ownership");
        assert_eq!(coordinator.data.lock().phase, CoordinatorPhase::Available);
        assert!(matches!(
            coordinator.reserve_preparation(
                DestructiveIntent::FullWipe,
                ShutdownDeadlineTier::Initial,
                None,
            ),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));
        assert!(matches!(
            coordinator.begin_recovery_cleanup(),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));

        let persistence = coordinator
            .begin_persistence_operation(wallet_id.clone())
            .expect("ordinary close persistence remains available");
        drop(persistence);

        coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::Retryable);
        assert!(
            result_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("destructive preparation finishes after ordinary close")
        );

        std::mem::forget(registration);
        drop(manager);
    }

    #[test]
    fn ordinary_close_defers_to_claimed_destructive_phase() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");

        coordinator.close_registration(registration_id);
        assert_eq!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::OrdinaryClosePending)
        );

        let super::DestructiveClaim::Claimed(targets) = coordinator
            .try_claim_destructive_registrations(Some(&wallet_id))
            .expect("destructive shutdown claims the actors")
        else {
            panic!("destructive shutdown must not wait for ordinary close");
        };
        assert_eq!(targets.len(), 1);
        assert!(matches!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::DestructiveClosing { .. })
        ));

        coordinator.release_failed_preparation();
        coordinator.data.lock().actors.remove(&registration_id);
        drop(targets);
        std::mem::forget(registration);
        drop(manager);
    }

    #[test]
    fn deadline_tiers_escalate_from_five_to_twenty_seconds() {
        assert_eq!(ShutdownDeadlineTier::Initial.duration(), std::time::Duration::from_secs(5));
        assert_eq!(ShutdownDeadlineTier::Retry.duration(), std::time::Duration::from_secs(20));
    }

    #[test]
    fn cancelled_attempt_cannot_authorize_retry() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        let intent = DestructiveIntent::Delete(wallet_id.clone());
        let blocked = coordinator.finish_blocked_attempt(
            intent.clone(),
            vec![],
            ShutdownDeadlineTier::Initial,
            None,
        );
        let WalletLifecycleFailure::ShutdownBlocked { attempt_id, .. } = blocked else {
            panic!("expected blocked attempt");
        };

        coordinator.cancel_attempt(&attempt_id);

        assert_eq!(
            enter_wallet_deletion(
                coordinator,
                wallet_id,
                ShutdownDeadlineTier::Retry,
                Some(&attempt_id),
            ),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        );
        assert_eq!(coordinator.data.lock().phase, CoordinatorPhase::Available);
    }

    #[test]
    fn recovery_cleanup_blocks_new_wallet_work_until_release() {
        let coordinator = Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let cleanup = coordinator.begin_recovery_cleanup().expect("cleanup is reserved");
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();

        assert!(matches!(
            coordinator.begin_construction(wallet_id.clone()),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));
        assert!(matches!(
            coordinator.begin_persistence_operation(wallet_id.clone()),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));

        drop(cleanup);

        assert!(coordinator.begin_persistence_operation(wallet_id).is_ok());
    }

    #[test]
    fn same_wallet_constructions_are_serialized() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        let first =
            coordinator.begin_construction(wallet_id.clone()).expect("first construction starts");

        assert!(matches!(
            coordinator.begin_construction(wallet_id.clone()),
            Err(WalletLifecycleFailure::ConstructionInProgress { wallet_id: id }) if id == wallet_id
        ));

        drop(first);
        assert!(coordinator.begin_construction(wallet_id).is_ok());
    }

    #[tokio::test]
    async fn cancelled_preparation_finishes_in_the_coordinator_owned_task() {
        crate::test_support::ensure_tokio_runtime();
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        let persistence =
            coordinator.begin_persistence_operation(wallet_id.clone()).expect("operation starts");
        let task = tokio::spawn(async move {
            coordinator
                .prepare_wallet_deletion(wallet_id, ShutdownDeadlineTier::Initial, None)
                .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let notified = coordinator.changed.notified();
                if coordinator.data.lock().phase != CoordinatorPhase::Available {
                    break;
                }

                notified.await;
            }
        })
        .await
        .expect("preparation reserves ownership");

        task.abort();
        drop(persistence);

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let notified = coordinator.changed.notified();
                let data = coordinator.data.lock();
                if data.phase == CoordinatorPhase::Available && data.pending_preparation.is_none() {
                    break;
                }

                drop(data);
                notified.await;
            }
        })
        .await
        .expect("coordinator-owned preparation releases after cancellation");
    }

    #[tokio::test]
    async fn cancelled_preparation_converges_a_real_destructive_registration() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let (wallet_id, registration_id, manager, registration) =
            register_preview_wallet(coordinator);
        let release_blocked_call = Arc::new(Notify::new());
        let release_blocked_call_for_actor = release_blocked_call.clone();
        let (blocked_call_started_sender, blocked_call_started_receiver) = oneshot::channel();
        manager.actor.send_mut(Box::new(move |_actor| {
            Box::pin(async move {
                let _ = blocked_call_started_sender.send(());
                release_blocked_call_for_actor.notified().await;
                false
            })
        }));
        blocked_call_started_receiver.await.expect("blocking wallet call starts");

        let preparation = tokio::spawn(async move {
            coordinator
                .prepare_wallet_deletion(wallet_id, ShutdownDeadlineTier::Initial, None)
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let data = coordinator.data.lock();
                if matches!(
                    data.actors.get(&registration_id).map(|actors| actors.state),
                    Some(RegistrationState::DestructiveClosing { .. })
                ) {
                    break;
                }

                drop(data);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("real registration enters destructive closing");

        preparation.abort();
        assert!(matches!(
            coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
            Some(RegistrationState::DestructiveClosing { .. })
        ));

        release_blocked_call.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let notified = coordinator.changed.notified();
                let data = coordinator.data.lock();
                if data.phase == CoordinatorPhase::Available
                    && !data.actors.contains_key(&registration_id)
                {
                    break;
                }

                drop(data);
                notified.await;
            }
        })
        .await
        .expect("detached preparation converges phase and registration");

        std::mem::forget(registration);
        drop(manager);
    }

    #[test]
    fn preparation_allows_owned_nested_work_but_rejects_new_external_work() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        let construction =
            coordinator.begin_construction(wallet_id.clone()).expect("construction starts");
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");

        let nested = coordinator
            .begin_persistence_operation(wallet_id.clone())
            .expect("owned nested write can finish");
        let external_coordinator = coordinator;
        let external = std::thread::spawn(move || {
            matches!(
                external_coordinator.begin_persistence_operation(wallet_id),
                Err(WalletLifecycleFailure::CoordinatorBusy)
            )
        })
        .join()
        .expect("external operation thread joins");

        assert!(external);

        drop(nested);
        drop(construction);
        coordinator.release_failed_preparation();
    }

    #[test]
    fn terminal_payjoin_authority_allows_only_its_wallet_and_phase() {
        let coordinator: &'static WalletLifecycleCoordinator =
            Box::leak(Box::new(WalletLifecycleCoordinator::default()));
        let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        let other_wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
        enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .expect("deletion preparation starts");
        let authority = coordinator.terminal_payjoin_authority(wallet_id.clone());

        let persistence = authority.begin(&wallet_id).expect("terminal fallback write is allowed");
        assert!(matches!(
            authority.begin(&other_wallet_id),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));
        assert!(matches!(
            coordinator.begin_persistence_operation(wallet_id.clone()),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));
        drop(persistence);

        coordinator.release_failed_preparation();
        assert!(matches!(
            authority.begin(&wallet_id),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        ));
    }

    #[test]
    fn stale_label_manager_rejects_storage_access() {
        let id = crate::wallet::metadata::WalletId::preview_new_random();
        let db = crate::database::wallet_data::WalletDataDb::new_in_memory(id)
            .expect("in-memory wallet data opens");
        let lifecycle = WalletManagerLifecycleToken::new();
        let labels =
            crate::label_manager::LabelManager::new_with_db(db).with_lifecycle(lifecycle.clone());

        assert!(!labels.has_labels().expect("active manager reads labels"));

        lifecycle.mark_closing();

        assert!(matches!(
            labels.has_labels(),
            Err(crate::label_manager::LabelManagerError::ManagerClosed)
        ));
    }
}

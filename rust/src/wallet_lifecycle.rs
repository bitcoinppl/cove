//! Process-wide ownership for persistent wallet actors and destructive operations

mod shutdown;

#[cfg(test)]
use shutdown::terminal_shutdown;
use shutdown::{terminal_discovery_shutdown, terminal_wallet_shutdown};

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

use act_zero::{Addr, call};
use futures::{StreamExt as _, stream};
use parking_lot::Mutex;
use rand::RngExt as _;
use tokio::sync::Notify;

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

impl DestructiveIntent {
    fn wallet_id(&self) -> Option<&WalletId> {
        match self {
            Self::Delete(wallet_id) => Some(wallet_id),
            Self::FullWipe => None,
        }
    }

    fn preparing_phase(self) -> CoordinatorPhase {
        match self {
            Self::Delete(wallet_id) => CoordinatorPhase::PreparingDeletion(wallet_id),
            Self::FullWipe => CoordinatorPhase::PreparingFullWipe,
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

enum ConstructionTransition {
    Permit { scope_id: u64 },
    Retry(OrdinaryCloseTarget),
}

enum ReservedPreparationTransition {
    Wait,
    Entered(CoordinatorPhase),
}

enum RegistrationCloseAction {
    Noop,
    Notify { arm_retry: bool },
    Spawn(OrdinaryCloseTarget),
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
        let transition = {
            let mut data = self.data.lock();
            Self::begin_construction_locked(&mut data, wallet_id.clone())?
        };

        match transition {
            ConstructionTransition::Permit { scope_id } => Ok(WalletConstructionPermit {
                coordinator: self,
                expected_wallet_id: Some(wallet_id),
                scope_id,
                active: true,
                _not_send: PhantomData,
            }),

            ConstructionTransition::Retry(retry_target) => {
                self.spawn_ordinary_close(retry_target);
                Err(WalletLifecycleFailure::ManagerClosing { wallet_id })
            }
        }
    }

    fn begin_construction_locked(
        data: &mut CoordinatorData,
        wallet_id: WalletId,
    ) -> Result<ConstructionTransition, WalletLifecycleFailure> {
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
            Some((_, RegistrationState::Active | RegistrationState::DestructiveClosing { .. })) => {
                Err(WalletLifecycleFailure::CoordinatorBusy)
            }

            Some((_, RegistrationState::RecoveryRequired)) => {
                Err(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id })
            }

            Some((_, RegistrationState::OrdinaryClosing)) => {
                Err(WalletLifecycleFailure::ManagerClosing { wallet_id })
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
                Ok(ConstructionTransition::Retry(Self::ordinary_close_target(
                    registration_id,
                    actors,
                    ShutdownDeadlineTier::Retry,
                )))
            }

            None => {
                if !data.constructing_wallets.insert(wallet_id.clone()) {
                    return Err(WalletLifecycleFailure::ConstructionInProgress { wallet_id });
                }

                data.active_constructions += 1;
                let scope_id = register_operation_scope(OperationScope::Wallet(wallet_id));
                Ok(ConstructionTransition::Permit { scope_id })
            }
        }
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
            let transition = {
                let mut data = self.data.lock();
                Self::try_enter_reserved_preparation(&mut data, reservation_id)?
            };

            match transition {
                ReservedPreparationTransition::Wait => notified.await,

                ReservedPreparationTransition::Entered(phase) => {
                    self.changed.notify_waiters();
                    return Ok(phase);
                }
            }
        }
    }

    fn try_enter_reserved_preparation(
        data: &mut CoordinatorData,
        reservation_id: u64,
    ) -> Result<ReservedPreparationTransition, WalletLifecycleFailure> {
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

        let ordinary_close_active = data.actors.values().any(|actors| {
            actors.state == RegistrationState::OrdinaryClosing
                && pending.intent.wallet_id().is_none_or(|id| id == &actors.wallet_id)
        });
        if ordinary_close_active {
            return Ok(ReservedPreparationTransition::Wait);
        }

        Self::validate_preparation_attempt(
            data,
            &pending.intent,
            pending.tier,
            pending.retry.as_ref(),
        )?;

        if let Some(attempt_id) = &pending.retry {
            data.retries.remove(attempt_id);
        }

        let phase = pending.intent.preparing_phase();
        data.phase = phase.clone();
        data.pending_preparation = None;
        Ok(ReservedPreparationTransition::Entered(phase))
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
        let action = {
            let mut data = self.data.lock();
            Self::close_registration_locked(&mut data, registration_id)
        };

        match action {
            RegistrationCloseAction::Noop => {}

            RegistrationCloseAction::Notify { arm_retry } => {
                self.changed.notify_waiters();
                if arm_retry {
                    self.arm_ordinary_close_retry(registration_id);
                }
            }

            RegistrationCloseAction::Spawn(target) => self.spawn_ordinary_close(target),
        }
    }

    fn close_registration_locked(
        data: &mut CoordinatorData,
        registration_id: u64,
    ) -> RegistrationCloseAction {
        let phase = data.phase.clone();
        let Some(actors) = data.actors.get_mut(&registration_id) else {
            return RegistrationCloseAction::Noop;
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

                RegistrationState::OrdinaryClosing | RegistrationState::RecoveryRequired => false,
            };

            if arm_retry {
                actors.lifecycle.mark_closing();
            }
            return RegistrationCloseAction::Notify { arm_retry };
        }

        if actors.state != RegistrationState::Active {
            return RegistrationCloseAction::Noop;
        }

        actors.lifecycle.mark_closing();
        if !cove_tokio::is_tokio_initialized() {
            actors.state = RegistrationState::OrdinaryClosePending;
            return RegistrationCloseAction::Notify { arm_retry: false };
        }

        actors.state = RegistrationState::OrdinaryClosing;
        RegistrationCloseAction::Spawn(Self::ordinary_close_target(
            registration_id,
            actors,
            ShutdownDeadlineTier::Initial,
        ))
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
mod tests;

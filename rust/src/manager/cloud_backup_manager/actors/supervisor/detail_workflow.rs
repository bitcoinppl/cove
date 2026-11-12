use std::time::Duration;

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use tokio::time::Instant;

use crate::manager::cloud_backup_manager::owner::inventory::InventoryScope;

pub(crate) const DETAIL_REFRESH_MINIMUM_INTERVAL: Duration = Duration::from_secs(5);

/// Who asked for a detail refresh, which decides whether its provider calls may ask the user
/// for consent
///
/// Ordered so the stronger origin wins when requests join: a tap that joins a waiting or
/// running refresh gives the refresh after it the tap's authority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DetailRefreshOrigin {
    /// Nobody tapped: launch discovery, a provider or wallet change, a new configuration
    Automatic,
    /// The user opened Cloud Backup or tapped Check Again
    Tap,
}

impl DetailRefreshOrigin {
    /// The provider client the refresh lists with
    ///
    /// A refresh nobody tapped for only gets a silent client, so the provider can't show its
    /// consent sheet: one that needs consent fails with `AuthorizationRequired`, which the screen
    /// shows as an incomplete inventory offering Reconnect
    pub(crate) fn cloud_client(self) -> CloudStorageClient {
        match self {
            Self::Automatic => CloudStorage::global_silent_client(),
            Self::Tap => CloudStorage::global_explicit_client(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailRefreshClaim {
    owner: u64,
    generation: u64,
    origin: DetailRefreshOrigin,
}

impl DetailRefreshClaim {
    pub(crate) fn origin(self) -> DetailRefreshOrigin {
        self.origin
    }
}

/// One operation's claim to apply the detail it reads, carrying the inventory scope its read
/// started in so the inventory owner can drop it after a configuration change
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailResultClaim {
    generation: u64,
    scope: InventoryScope,
}

impl DetailResultClaim {
    pub(crate) fn scope(self) -> InventoryScope {
        self.scope
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailRefreshPlan {
    Start(DetailRefreshClaim),
    Wait { owner: u64, delay: Duration },
    Queued,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailRefreshCompletion {
    pub(crate) apply: bool,
    pub(crate) next: DetailRefreshPlan,
}

/// Where the detail refresh cycle is; the variants replace the flags that used to be kept
/// consistent by hand
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum RefreshPhase {
    #[default]
    Closed,
    /// A local reset owns the runtime, so nothing refreshes until it ends; `reopen` says whether
    /// the cycle opens again when the reset is abandoned and the backup resumes
    Suspended {
        reopen: bool,
    },
    Idle,
    InFlight {
        claim: DetailRefreshClaim,
        /// The strongest origin among the requests that arrived while it ran
        trailing: Option<DetailRefreshOrigin>,
    },
    /// A rate-limit timer is scheduled and a trailing refresh is waiting on it
    WaitingTimer {
        origin: DetailRefreshOrigin,
    },
}

#[derive(Debug, Default)]
struct DetailRefreshCoordinator {
    owner: u64,
    next_generation: u64,
    phase: RefreshPhase,
    last_started_at: Option<Duration>,
}

impl DetailRefreshCoordinator {
    pub(crate) fn open(&mut self) {
        match &mut self.phase {
            RefreshPhase::Closed => self.reopen(),

            // the reset owns the runtime, so the open waits for the reset to end
            RefreshPhase::Suspended { reopen } => *reopen = true,
            RefreshPhase::Idle
            | RefreshPhase::InFlight { .. }
            | RefreshPhase::WaitingTimer { .. } => {}
        }
    }

    /// Drops the waiting or in-flight refresh until the reset ends, remembering whether the cycle
    /// was open; the resume takes a new owner, so the dropped one's timer and claim match nothing
    fn suspend(&mut self) {
        let reopen = match self.phase {
            RefreshPhase::Closed => false,
            RefreshPhase::Suspended { reopen } => reopen,
            RefreshPhase::Idle
            | RefreshPhase::InFlight { .. }
            | RefreshPhase::WaitingTimer { .. } => true,
        };

        self.phase = RefreshPhase::Suspended { reopen };
    }

    /// Ends the reset's suspension; a cycle that was open reopens when the backup `resumed`,
    /// with nothing queued, and returns whether it did
    fn end_suspension(&mut self, resumed: bool) -> bool {
        match self.phase {
            RefreshPhase::Suspended { reopen: true } if resumed => {
                self.reopen();
                true
            }

            RefreshPhase::Suspended { .. } => {
                self.phase = RefreshPhase::Closed;
                false
            }

            RefreshPhase::Closed
            | RefreshPhase::Idle
            | RefreshPhase::InFlight { .. }
            | RefreshPhase::WaitingTimer { .. } => false,
        }
    }

    fn reopen(&mut self) {
        self.owner = self.owner.wrapping_add(1);
        self.phase = RefreshPhase::Idle;
    }

    /// A request that joins the load the inventory shows: it raises the authority of the refresh
    /// after that load, and never starts a second load beside it
    fn join(&mut self, origin: DetailRefreshOrigin) {
        match &mut self.phase {
            RefreshPhase::InFlight { trailing, .. } => *trailing = (*trailing).max(Some(origin)),
            RefreshPhase::WaitingTimer { origin: waiting } => *waiting = (*waiting).max(origin),
            RefreshPhase::Closed | RefreshPhase::Suspended { .. } | RefreshPhase::Idle => {}
        }
    }

    pub(crate) fn request(
        &mut self,
        now: Duration,
        origin: DetailRefreshOrigin,
    ) -> DetailRefreshPlan {
        match self.phase {
            RefreshPhase::Closed | RefreshPhase::Suspended { .. } => DetailRefreshPlan::Ignored,
            RefreshPhase::InFlight { .. } | RefreshPhase::WaitingTimer { .. } => {
                self.join(origin);
                DetailRefreshPlan::Queued
            }

            RefreshPhase::Idle => match self.rate_limit_delay(now) {
                Some(delay) => {
                    self.phase = RefreshPhase::WaitingTimer { origin };
                    DetailRefreshPlan::Wait { owner: self.owner, delay }
                }
                None => self.start(now, origin),
            },
        }
    }

    pub(crate) fn timer_elapsed(&mut self, owner: u64, now: Duration) -> DetailRefreshPlan {
        let RefreshPhase::WaitingTimer { origin } = self.phase else {
            return DetailRefreshPlan::Ignored;
        };
        if self.owner != owner {
            return DetailRefreshPlan::Ignored;
        }

        self.phase = RefreshPhase::Idle;
        self.request(now, origin)
    }

    pub(crate) fn complete(
        &mut self,
        claim: DetailRefreshClaim,
        now: Duration,
    ) -> DetailRefreshCompletion {
        if !self.is_active(claim) {
            return DetailRefreshCompletion { apply: false, next: DetailRefreshPlan::Ignored };
        }

        let trailing = match self.phase {
            RefreshPhase::InFlight { trailing, .. } => trailing,
            _ => None,
        };
        self.phase = RefreshPhase::Idle;
        let next = match trailing {
            Some(origin) => self.request(now, origin),
            None => DetailRefreshPlan::Ignored,
        };

        DetailRefreshCompletion { apply: true, next }
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(
            self.phase,
            RefreshPhase::Idle | RefreshPhase::InFlight { .. } | RefreshPhase::WaitingTimer { .. }
        )
    }

    /// Re-key the in-flight refresh to the claim that carries the shared result generation
    fn replace_in_flight_claim(&mut self, claim: DetailRefreshClaim) {
        if let RefreshPhase::InFlight { claim: in_flight, .. } = &mut self.phase {
            *in_flight = claim;
        }
    }

    pub(crate) fn is_active(&self, claim: DetailRefreshClaim) -> bool {
        claim.owner == self.owner
            && matches!(self.phase, RefreshPhase::InFlight { claim: in_flight, .. } if in_flight == claim)
    }

    fn start(&mut self, now: Duration, origin: DetailRefreshOrigin) -> DetailRefreshPlan {
        let claim =
            DetailRefreshClaim { owner: self.owner, generation: self.next_generation, origin };
        self.next_generation = self.next_generation.wrapping_add(1);
        self.phase = RefreshPhase::InFlight { claim, trailing: None };
        self.last_started_at = Some(now);

        DetailRefreshPlan::Start(claim)
    }

    fn rate_limit_delay(&self, now: Duration) -> Option<Duration> {
        let earliest = self.last_started_at?.saturating_add(DETAIL_REFRESH_MINIMUM_INTERVAL);
        (now < earliest).then(|| earliest - now)
    }
}

#[derive(Debug)]
pub(crate) struct DetailWorkflow {
    refresh: DetailRefreshCoordinator,
    /// Tokio's clock, the one the rate limit's wait sleeps on, so a paused test clock moves both
    clock: Instant,
    next_result_generation: u64,
    newest_result_generation: Option<u64>,
}

impl Default for DetailWorkflow {
    fn default() -> Self {
        Self {
            refresh: DetailRefreshCoordinator::default(),
            clock: Instant::now(),
            next_result_generation: 0,
            newest_result_generation: None,
        }
    }
}

impl DetailWorkflow {
    pub(crate) fn open(&mut self) {
        self.refresh.open();
    }

    pub(crate) fn is_open(&self) -> bool {
        self.refresh.is_open()
    }

    /// Cancels the refresh cycle while a local reset owns the runtime; nothing opens it until
    /// [`Self::end_suspension`]
    pub(crate) fn suspend(&mut self) {
        self.refresh.suspend();
    }

    /// Ends a local reset's suspension and returns whether the cycle reopened, which it does
    /// only when it was open and the backup `resumed`; the caller loads the inventory again
    pub(crate) fn end_suspension(&mut self, resumed: bool) -> bool {
        self.refresh.end_suspension(resumed)
    }

    /// A tap that joins the load the inventory shows loading, which gives the refresh after that
    /// load the tap's authority
    pub(crate) fn join_refresh(&mut self, origin: DetailRefreshOrigin) {
        self.refresh.join(origin);
    }

    pub(crate) fn request_refresh(&mut self, origin: DetailRefreshOrigin) -> DetailRefreshPlan {
        let plan = self.refresh.request(self.now(), origin);
        self.admit_refresh_plan(plan)
    }

    pub(crate) fn timer_elapsed(&mut self, owner: u64) -> DetailRefreshPlan {
        let plan = self.refresh.timer_elapsed(owner, self.now());
        self.admit_refresh_plan(plan)
    }

    pub(crate) fn complete_refresh(
        &mut self,
        claim: DetailRefreshClaim,
    ) -> DetailRefreshCompletion {
        let mut completion = self.refresh.complete(claim, self.now());
        completion.apply &= self.is_latest_generation(claim.generation);
        completion.next = self.admit_refresh_plan(completion.next);
        completion
    }

    pub(crate) fn is_refresh_active(&self, claim: DetailRefreshClaim) -> bool {
        self.refresh.is_active(claim)
    }

    pub(crate) fn is_latest_refresh(&self, claim: DetailRefreshClaim) -> bool {
        self.is_latest_generation(claim.generation)
    }

    pub(crate) fn start_operation_result(&mut self, scope: InventoryScope) -> DetailResultClaim {
        DetailResultClaim { generation: self.start_result_generation(), scope }
    }

    pub(crate) fn is_latest_result(&self, claim: DetailResultClaim) -> bool {
        self.is_latest_generation(claim.generation)
    }

    fn start_result_generation(&mut self) -> u64 {
        let generation = self.next_result_generation;
        self.next_result_generation = self.next_result_generation.wrapping_add(1);
        self.newest_result_generation = Some(generation);
        generation
    }

    fn is_latest_generation(&self, generation: u64) -> bool {
        self.newest_result_generation == Some(generation)
    }

    fn now(&self) -> Duration {
        self.clock.elapsed()
    }

    fn admit_refresh_plan(&mut self, plan: DetailRefreshPlan) -> DetailRefreshPlan {
        let DetailRefreshPlan::Start(claim) = plan else { return plan };

        let generation = self.start_result_generation();
        let claim = DetailRefreshClaim { generation, ..claim };
        self.refresh.replace_in_flight_claim(claim);

        DetailRefreshPlan::Start(claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_allow_one_in_flight_and_one_trailing_refresh() {
        let mut coordinator = DetailRefreshCoordinator::default();
        coordinator.open();

        let DetailRefreshPlan::Start(first) =
            coordinator.request(Duration::ZERO, DetailRefreshOrigin::Automatic)
        else {
            panic!("expected first refresh to start");
        };
        assert_eq!(
            coordinator.request(Duration::from_secs(1), DetailRefreshOrigin::Automatic),
            DetailRefreshPlan::Queued
        );
        assert_eq!(
            coordinator.request(Duration::from_secs(2), DetailRefreshOrigin::Automatic),
            DetailRefreshPlan::Queued
        );

        let completion = coordinator.complete(first, Duration::from_secs(2));
        assert!(completion.apply);
        assert_eq!(
            completion.next,
            DetailRefreshPlan::Wait { owner: first.owner, delay: Duration::from_secs(3) }
        );
        assert_eq!(
            coordinator.request(Duration::from_secs(3), DetailRefreshOrigin::Automatic),
            DetailRefreshPlan::Queued
        );

        let DetailRefreshPlan::Start(second) =
            coordinator.timer_elapsed(first.owner, Duration::from_secs(5))
        else {
            panic!("expected trailing refresh to start at rate limit");
        };
        assert_ne!(first, second);
    }

    #[test]
    fn newest_started_detail_result_wins() {
        let mut workflow = DetailWorkflow::default();
        let older = workflow.start_operation_result(InventoryScope::default());
        let newer = workflow.start_operation_result(InventoryScope::default());

        assert!(!workflow.is_latest_result(older));
        assert!(workflow.is_latest_result(newer));
    }

    #[test]
    fn newer_operation_supersedes_screen_result_without_dropping_trailing_refresh() {
        let mut workflow = DetailWorkflow::default();
        workflow.open();
        let DetailRefreshPlan::Start(refresh) =
            workflow.request_refresh(DetailRefreshOrigin::Automatic)
        else {
            panic!("expected refresh to start");
        };
        assert_eq!(
            workflow.request_refresh(DetailRefreshOrigin::Automatic),
            DetailRefreshPlan::Queued
        );

        workflow.start_operation_result(InventoryScope::default());
        let completion = workflow.complete_refresh(refresh);

        assert!(!completion.apply);
        assert!(matches!(completion.next, DetailRefreshPlan::Wait { .. }));
    }

    #[test]
    fn a_tap_that_joins_an_automatic_refresh_gives_the_next_one_its_authority() {
        let mut coordinator = DetailRefreshCoordinator::default();
        coordinator.open();

        let DetailRefreshPlan::Start(automatic) =
            coordinator.request(Duration::ZERO, DetailRefreshOrigin::Automatic)
        else {
            panic!("expected the automatic refresh to start");
        };
        assert_eq!(automatic.origin(), DetailRefreshOrigin::Automatic);
        assert_eq!(
            coordinator.request(Duration::from_secs(1), DetailRefreshOrigin::Tap),
            DetailRefreshPlan::Queued
        );
        assert_eq!(
            coordinator.request(Duration::from_secs(1), DetailRefreshOrigin::Automatic),
            DetailRefreshPlan::Queued
        );

        let completion = coordinator.complete(automatic, Duration::from_secs(10));
        let DetailRefreshPlan::Start(trailing) = completion.next else {
            panic!("expected the trailing refresh to start");
        };
        assert_eq!(trailing.origin(), DetailRefreshOrigin::Tap);
    }

    #[test]
    fn an_abandoned_reset_reopens_the_cycle_it_suspended_without_its_queued_work() {
        let mut coordinator = DetailRefreshCoordinator::default();
        coordinator.open();
        let DetailRefreshPlan::Start(before) =
            coordinator.request(Duration::ZERO, DetailRefreshOrigin::Automatic)
        else {
            panic!("expected the refresh to start");
        };
        assert_eq!(
            coordinator.request(Duration::ZERO, DetailRefreshOrigin::Tap),
            DetailRefreshPlan::Queued
        );

        coordinator.suspend();
        coordinator.open();
        assert!(!coordinator.is_open(), "an open during the reset waits for it to end");
        assert_eq!(
            coordinator.request(Duration::from_secs(1), DetailRefreshOrigin::Tap),
            DetailRefreshPlan::Ignored
        );

        assert!(coordinator.end_suspension(true));
        assert!(coordinator.is_open());
        assert_eq!(
            coordinator.complete(before, Duration::from_secs(10)),
            DetailRefreshCompletion { apply: false, next: DetailRefreshPlan::Ignored },
            "the refresh from before the reset, and the tap that joined it, belong to no cycle"
        );

        let DetailRefreshPlan::Start(after) =
            coordinator.request(Duration::from_secs(10), DetailRefreshOrigin::Automatic)
        else {
            panic!("expected the reopened cycle to start a refresh");
        };
        assert_ne!(after.owner, before.owner);
        assert_eq!(after.origin(), DetailRefreshOrigin::Automatic);
    }

    #[test]
    fn a_reset_reopens_nothing_unless_it_resumes_a_cycle_that_was_open() {
        let mut completed = DetailRefreshCoordinator::default();
        completed.open();
        completed.suspend();
        assert!(!completed.end_suspension(false), "a completed reset keeps the cycle closed");
        assert!(!completed.is_open());

        let mut never_opened = DetailRefreshCoordinator::default();
        never_opened.suspend();
        assert!(!never_opened.end_suspension(true), "a resume reopens only an open cycle");
        assert!(!never_opened.is_open());
    }
}

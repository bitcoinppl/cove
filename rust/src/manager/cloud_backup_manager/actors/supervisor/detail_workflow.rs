use std::time::{Duration, Instant};

use crate::manager::cloud_backup_manager::owner::inventory::InventoryScope;

pub(crate) const DETAIL_REFRESH_MINIMUM_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailRefreshClaim {
    owner: u64,
    generation: u64,
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
    Idle,
    InFlight {
        claim: DetailRefreshClaim,
        trailing_requested: bool,
    },
    /// A rate-limit timer is scheduled and a trailing refresh is waiting on it
    WaitingTimer,
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
        if self.phase != RefreshPhase::Closed {
            return;
        }

        self.owner = self.owner.wrapping_add(1);
        self.phase = RefreshPhase::Idle;
    }

    pub(crate) fn request(&mut self, now: Duration) -> DetailRefreshPlan {
        match &mut self.phase {
            RefreshPhase::Closed => DetailRefreshPlan::Ignored,
            RefreshPhase::InFlight { trailing_requested, .. } => {
                *trailing_requested = true;
                DetailRefreshPlan::Queued
            }
            RefreshPhase::WaitingTimer => DetailRefreshPlan::Queued,
            RefreshPhase::Idle => match self.rate_limit_delay(now) {
                Some(delay) => {
                    self.phase = RefreshPhase::WaitingTimer;
                    DetailRefreshPlan::Wait { owner: self.owner, delay }
                }
                None => self.start(now),
            },
        }
    }

    pub(crate) fn timer_elapsed(&mut self, owner: u64, now: Duration) -> DetailRefreshPlan {
        if self.phase != RefreshPhase::WaitingTimer || self.owner != owner {
            return DetailRefreshPlan::Ignored;
        }

        self.phase = RefreshPhase::Idle;
        self.request(now)
    }

    pub(crate) fn complete(
        &mut self,
        claim: DetailRefreshClaim,
        now: Duration,
    ) -> DetailRefreshCompletion {
        if !self.is_active(claim) {
            return DetailRefreshCompletion { apply: false, next: DetailRefreshPlan::Ignored };
        }

        let trailing_requested =
            matches!(self.phase, RefreshPhase::InFlight { trailing_requested: true, .. });
        self.phase = RefreshPhase::Idle;
        let next = if trailing_requested { self.request(now) } else { DetailRefreshPlan::Ignored };

        DetailRefreshCompletion { apply: true, next }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.phase != RefreshPhase::Closed
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

    fn start(&mut self, now: Duration) -> DetailRefreshPlan {
        let claim = DetailRefreshClaim { owner: self.owner, generation: self.next_generation };
        self.next_generation = self.next_generation.wrapping_add(1);
        self.phase = RefreshPhase::InFlight { claim, trailing_requested: false };
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

    pub(crate) fn request_refresh(&mut self) -> DetailRefreshPlan {
        let plan = self.refresh.request(self.now());
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
        let claim = DetailRefreshClaim { owner: claim.owner, generation };
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

        let DetailRefreshPlan::Start(first) = coordinator.request(Duration::ZERO) else {
            panic!("expected first refresh to start");
        };
        assert_eq!(coordinator.request(Duration::from_secs(1)), DetailRefreshPlan::Queued);
        assert_eq!(coordinator.request(Duration::from_secs(2)), DetailRefreshPlan::Queued);

        let completion = coordinator.complete(first, Duration::from_secs(2));
        assert!(completion.apply);
        assert_eq!(
            completion.next,
            DetailRefreshPlan::Wait { owner: first.owner, delay: Duration::from_secs(3) }
        );
        assert_eq!(coordinator.request(Duration::from_secs(3)), DetailRefreshPlan::Queued);

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
        let DetailRefreshPlan::Start(refresh) = workflow.request_refresh() else {
            panic!("expected refresh to start");
        };
        assert_eq!(workflow.request_refresh(), DetailRefreshPlan::Queued);

        workflow.start_operation_result(InventoryScope::default());
        let completion = workflow.complete_refresh(refresh);

        assert!(!completion.apply);
        assert!(matches!(completion.next, DetailRefreshPlan::Wait { .. }));
    }
}

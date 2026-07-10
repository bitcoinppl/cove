use std::time::Duration;

pub(super) const DETAIL_REFRESH_MINIMUM_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailRefreshClaim {
    owner: u64,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailRefreshPlan {
    Start(DetailRefreshClaim),
    Wait { owner: u64, delay: Duration },
    Queued,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DetailRefreshCompletion {
    pub(super) apply: bool,
    pub(super) next: DetailRefreshPlan,
}

#[derive(Debug, Default)]
pub(super) struct DetailRefreshCoordinator {
    owner: u64,
    is_open: bool,
    next_generation: u64,
    in_flight: Option<DetailRefreshClaim>,
    trailing_requested: bool,
    timer_scheduled: bool,
    last_started_at: Option<Duration>,
}

impl DetailRefreshCoordinator {
    pub(super) fn open(&mut self) {
        if self.is_open {
            return;
        }

        self.owner = self.owner.wrapping_add(1);
        self.is_open = true;
        self.in_flight = None;
        self.trailing_requested = false;
        self.timer_scheduled = false;
    }

    pub(super) fn close(&mut self) {
        self.owner = self.owner.wrapping_add(1);
        self.is_open = false;
        self.in_flight = None;
        self.trailing_requested = false;
        self.timer_scheduled = false;
    }

    pub(super) fn request(&mut self, now: Duration) -> DetailRefreshPlan {
        if !self.is_open {
            return DetailRefreshPlan::Ignored;
        }

        if self.in_flight.is_some() {
            self.trailing_requested = true;
            return DetailRefreshPlan::Queued;
        }

        if let Some(delay) = self.rate_limit_delay(now) {
            self.trailing_requested = true;
            if self.timer_scheduled {
                return DetailRefreshPlan::Queued;
            }

            self.timer_scheduled = true;
            return DetailRefreshPlan::Wait { owner: self.owner, delay };
        }

        self.start(now)
    }

    pub(super) fn timer_elapsed(&mut self, owner: u64, now: Duration) -> DetailRefreshPlan {
        if !self.is_open || self.owner != owner || !self.timer_scheduled {
            return DetailRefreshPlan::Ignored;
        }

        self.timer_scheduled = false;
        if !self.trailing_requested {
            return DetailRefreshPlan::Ignored;
        }

        self.request(now)
    }

    pub(super) fn complete(
        &mut self,
        claim: DetailRefreshClaim,
        now: Duration,
    ) -> DetailRefreshCompletion {
        if !self.is_open || self.in_flight != Some(claim) || claim.owner != self.owner {
            return DetailRefreshCompletion { apply: false, next: DetailRefreshPlan::Ignored };
        }

        self.in_flight = None;
        let next = if self.trailing_requested {
            self.trailing_requested = false;
            self.request(now)
        } else {
            DetailRefreshPlan::Ignored
        };

        DetailRefreshCompletion { apply: true, next }
    }

    pub(super) fn is_active(&self, claim: DetailRefreshClaim) -> bool {
        self.is_open && claim.owner == self.owner && self.in_flight == Some(claim)
    }

    fn start(&mut self, now: Duration) -> DetailRefreshPlan {
        let claim = DetailRefreshClaim { owner: self.owner, generation: self.next_generation };
        self.next_generation = self.next_generation.wrapping_add(1);
        self.in_flight = Some(claim);
        self.trailing_requested = false;
        self.last_started_at = Some(now);

        DetailRefreshPlan::Start(claim)
    }

    fn rate_limit_delay(&self, now: Duration) -> Option<Duration> {
        let earliest = self.last_started_at?.saturating_add(DETAIL_REFRESH_MINIMUM_INTERVAL);
        (now < earliest).then(|| earliest - now)
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
    fn stale_and_closed_owner_completions_are_ignored() {
        let mut coordinator = DetailRefreshCoordinator::default();
        coordinator.open();
        let DetailRefreshPlan::Start(stale) = coordinator.request(Duration::ZERO) else {
            panic!("expected refresh to start");
        };

        coordinator.close();
        assert!(!coordinator.is_active(stale));
        assert!(!coordinator.complete(stale, Duration::from_secs(1)).apply);

        coordinator.open();
        assert!(!coordinator.is_active(stale));
        assert!(!coordinator.complete(stale, Duration::from_secs(2)).apply);
    }

    #[test]
    fn closing_invalidates_a_scheduled_trailing_refresh() {
        let mut coordinator = DetailRefreshCoordinator::default();
        coordinator.open();
        let DetailRefreshPlan::Start(first) = coordinator.request(Duration::ZERO) else {
            panic!("expected refresh to start");
        };
        let owner = first.owner;
        assert_eq!(coordinator.request(Duration::from_secs(1)), DetailRefreshPlan::Queued);
        assert!(matches!(
            coordinator.complete(first, Duration::from_secs(1)).next,
            DetailRefreshPlan::Wait { .. }
        ));

        coordinator.close();

        assert_eq!(
            coordinator.timer_elapsed(owner, Duration::from_secs(5)),
            DetailRefreshPlan::Ignored
        );
    }
}

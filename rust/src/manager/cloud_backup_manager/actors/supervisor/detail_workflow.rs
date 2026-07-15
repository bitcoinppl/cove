use std::time::{Duration, Instant};

use super::{DetailEntryPlan, RuntimePasskeyAuthorization};
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupStatus, PendingVerificationCompletion, RustCloudBackupManager,
    VerificationState,
};

pub(super) const DETAIL_REFRESH_MINIMUM_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailRefreshClaim {
    owner: u64,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailResultClaim(u64);

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
struct DetailRefreshCoordinator {
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

#[derive(Debug)]
pub(super) struct DetailWorkflow {
    refresh: DetailRefreshCoordinator,
    clock: Instant,
    pending_verification_completion: Option<PendingVerificationCompletion>,
    // clearing session-only proof with this workflow makes detail entry re-check passkey availability
    runtime_passkey_authorization: Option<RuntimePasskeyAuthorization>,
    next_result_generation: u64,
    newest_result_generation: Option<u64>,
}

impl Default for DetailWorkflow {
    fn default() -> Self {
        Self {
            refresh: DetailRefreshCoordinator::default(),
            clock: Instant::now(),
            pending_verification_completion: None,
            runtime_passkey_authorization: None,
            next_result_generation: 0,
            newest_result_generation: None,
        }
    }
}

impl DetailWorkflow {
    pub(super) fn open(&mut self) {
        self.refresh.open();
    }

    pub(super) fn close(&mut self) {
        self.refresh.close();
    }

    pub(super) fn request_refresh(&mut self) -> DetailRefreshPlan {
        let plan = self.refresh.request(self.now());
        self.admit_refresh_plan(plan)
    }

    pub(super) fn timer_elapsed(&mut self, owner: u64) -> DetailRefreshPlan {
        let plan = self.refresh.timer_elapsed(owner, self.now());
        self.admit_refresh_plan(plan)
    }

    pub(super) fn complete_refresh(
        &mut self,
        claim: DetailRefreshClaim,
    ) -> DetailRefreshCompletion {
        let mut completion = self.refresh.complete(claim, self.now());
        completion.apply &= self.is_latest_result(DetailResultClaim(claim.generation));
        completion.next = self.admit_refresh_plan(completion.next);
        completion
    }

    pub(super) fn is_refresh_active(&self, claim: DetailRefreshClaim) -> bool {
        self.refresh.is_active(claim)
    }

    pub(super) fn is_latest_refresh(&self, claim: DetailRefreshClaim) -> bool {
        self.is_latest_result(DetailResultClaim(claim.generation))
    }

    pub(super) fn start_operation_result(&mut self) -> DetailResultClaim {
        let claim = DetailResultClaim(self.next_result_generation);
        self.next_result_generation = self.next_result_generation.wrapping_add(1);
        self.newest_result_generation = Some(claim.0);
        claim
    }

    pub(super) fn is_latest_result(&self, claim: DetailResultClaim) -> bool {
        self.newest_result_generation == Some(claim.0)
    }

    pub(super) fn entry_plan(&self, manager: &RustCloudBackupManager) -> DetailEntryPlan {
        let state = manager.state.read();
        if !matches!(state.status(), CloudBackupStatus::Enabled) {
            return DetailEntryPlan::RefreshOnly;
        }

        if super::restore_all_marker_matches_active_namespace(manager) {
            return DetailEntryPlan::RefreshOnly;
        }

        if matches!(
            state.verification(),
            VerificationState::Verifying
                | VerificationState::Verified(_)
                | VerificationState::PasskeyConfirmed
        ) {
            return DetailEntryPlan::ContinueRustOwnedVerification;
        }

        if let Some(completion) = self.pending_verification_completion.clone() {
            return DetailEntryPlan::ResumePendingUploadConfirmation(completion);
        }

        if let Some(authorization) = self.authorization_for_current_manager(manager) {
            return DetailEntryPlan::UseFreshEnableProof(authorization);
        }

        DetailEntryPlan::StartPasskeyVerification { force_discoverable: true }
    }

    pub(super) fn cache_pending_completion(&mut self, completion: PendingVerificationCompletion) {
        self.pending_verification_completion = Some(completion);
    }

    pub(super) fn clear_pending_completion(&mut self) {
        self.pending_verification_completion = None;
    }

    pub(super) fn set_authorization(&mut self, authorization: RuntimePasskeyAuthorization) {
        self.runtime_passkey_authorization = Some(authorization);
    }

    pub(super) fn clear_authorization(&mut self) {
        self.runtime_passkey_authorization = None;
    }

    fn authorization_for_current_manager(
        &self,
        manager: &RustCloudBackupManager,
    ) -> Option<RuntimePasskeyAuthorization> {
        let authorization = self.runtime_passkey_authorization.as_ref()?;
        let Ok(namespace_id) = manager.current_namespace_id() else {
            return None;
        };

        let cloud_keychain = CloudBackupKeychain::global();
        let credential_id = cloud_keychain.load_credential_id()?;
        let prf_salt = cloud_keychain.load_prf_salt()?;

        (authorization.namespace_id == namespace_id
            && authorization.credential_id == credential_id
            && authorization.prf_salt == prf_salt)
            .then(|| authorization.clone())
    }

    fn now(&self) -> Duration {
        self.clock.elapsed()
    }

    fn admit_refresh_plan(&mut self, plan: DetailRefreshPlan) -> DetailRefreshPlan {
        let DetailRefreshPlan::Start(claim) = plan else { return plan };

        let result_claim = self.start_operation_result();
        let claim = DetailRefreshClaim { owner: claim.owner, generation: result_claim.0 };
        self.refresh.in_flight = Some(claim);

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

    #[test]
    fn newest_started_detail_result_wins() {
        let mut workflow = DetailWorkflow::default();
        let older = workflow.start_operation_result();
        let newer = workflow.start_operation_result();

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

        workflow.start_operation_result();
        let completion = workflow.complete_refresh(refresh);

        assert!(!completion.apply);
        assert!(matches!(completion.next, DetailRefreshPlan::Wait { .. }));
    }
}

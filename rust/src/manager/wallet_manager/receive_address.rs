use std::sync::Arc;
use std::time::Duration;

use cove_types::address::AddressInfoWithDerivation;

pub const CACHE_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ReceiveAddressStatus {
    Fresh,
    CachedUnused,
    PaymentReceived,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ReceiveAddressCopyPolicy {
    Copy,
    ConfirmPaidAddress,
}

#[derive(Debug, Default, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ReceiveAddressRefreshState {
    #[default]
    Idle,
    Refreshing,
    Failed,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ReceiveAddressPresentation {
    pub copy_policy: ReceiveAddressCopyPolicy,
    pub refresh_state: ReceiveAddressRefreshState,
}

impl Default for ReceiveAddressPresentation {
    fn default() -> Self {
        Self {
            copy_policy: ReceiveAddressCopyPolicy::Copy,
            refresh_state: ReceiveAddressRefreshState::Idle,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ReceiveAddressState {
    pub request_id: u64,
    pub address: Arc<AddressInfoWithDerivation>,
    pub status: ReceiveAddressStatus,
    pub first_shown_at_secs: u64,
    pub expires_at_secs: Option<u64>,
}

impl ReceiveAddressState {
    pub fn cached(
        request_id: u64,
        address: AddressInfoWithDerivation,
        status: ReceiveAddressStatus,
        first_shown_at_secs: u64,
    ) -> Self {
        Self {
            request_id,
            address: Arc::new(address),
            status,
            first_shown_at_secs,
            expires_at_secs: Some(first_shown_at_secs + CACHE_WINDOW.as_secs()),
        }
    }

    pub fn payment_received(&self) -> Self {
        Self {
            status: ReceiveAddressStatus::PaymentReceived,
            expires_at_secs: None,
            ..self.clone()
        }
    }

    pub fn refresh_delay(&self, now_secs: u64) -> Option<Duration> {
        if self.status == ReceiveAddressStatus::PaymentReceived {
            return None;
        }

        let expires_at_secs = self.expires_at_secs?;
        let delay_secs = expires_at_secs.saturating_sub(now_secs);

        Some(Duration::from_secs(delay_secs))
    }

    fn should_refresh_at(&self, now_secs: u64) -> bool {
        self.status != ReceiveAddressStatus::PaymentReceived
            && self.expires_at_secs.is_some_and(|expires_at_secs| now_secs >= expires_at_secs)
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum RefreshExpiredAddressDecision {
    Rotate,
    ReturnVisible(ReceiveAddressState),
    MissingVisibleState,
}

#[derive(Debug, Default)]
pub struct ReceiveAddressSession {
    active_request_id: Option<u64>,
    next_request_id: u64,
    visible_state: Option<ReceiveAddressState>,
    refresh_state: ReceiveAddressRefreshState,
}

impl ReceiveAddressSession {
    pub fn next_request_id(&mut self) -> u64 {
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.active_request_id = Some(self.next_request_id);
        self.next_request_id
    }

    pub fn set_visible(&mut self, state: ReceiveAddressState) {
        self.active_request_id = Some(state.request_id);
        self.visible_state = Some(state);
    }

    pub fn visible_state(&self) -> Option<ReceiveAddressState> {
        self.visible_state.clone()
    }

    pub fn set_refresh_state(&mut self, refresh_state: ReceiveAddressRefreshState) {
        self.refresh_state = refresh_state;
    }

    pub fn presentation(&self) -> ReceiveAddressPresentation {
        let copy_policy = if self
            .visible_state
            .as_ref()
            .is_some_and(|state| state.status == ReceiveAddressStatus::PaymentReceived)
        {
            ReceiveAddressCopyPolicy::ConfirmPaidAddress
        } else {
            ReceiveAddressCopyPolicy::Copy
        };

        ReceiveAddressPresentation { copy_policy, refresh_state: self.refresh_state }
    }

    pub fn is_current(&self, request_id: u64) -> bool {
        self.active_request_id == Some(request_id)
    }

    pub fn can_mark_payment_received(&self, request_id: u64, derivation_index: u32) -> bool {
        let Some(visible_state) = &self.visible_state else {
            return false;
        };

        self.is_current(request_id)
            && visible_state.address.info.index == derivation_index
            && visible_state.status != ReceiveAddressStatus::PaymentReceived
    }

    pub fn mark_payment_received(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) -> Option<ReceiveAddressState> {
        if !self.can_mark_payment_received(request_id, derivation_index) {
            return None;
        }

        let state = self.visible_state.as_ref()?.payment_received();
        self.set_visible(state.clone());

        Some(state)
    }

    pub fn close(&mut self, request_id: u64) -> bool {
        if !self.is_current(request_id) {
            return false;
        }

        self.active_request_id = None;
        self.visible_state = None;
        self.refresh_state = ReceiveAddressRefreshState::Idle;

        true
    }

    pub fn refresh_expired_decision(
        &self,
        request_id: u64,
        now_secs: u64,
    ) -> RefreshExpiredAddressDecision {
        let Some(visible_state) = self.visible_state() else {
            return RefreshExpiredAddressDecision::MissingVisibleState;
        };

        if !self.is_current(request_id)
            || self.refresh_state != ReceiveAddressRefreshState::Idle
            || !visible_state.should_refresh_at(now_secs)
        {
            return RefreshExpiredAddressDecision::ReturnVisible(visible_state);
        }

        RefreshExpiredAddressDecision::Rotate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::{AddressInfo, KeychainKind};
    use bitcoin::{Address, Network};
    use std::str::FromStr as _;

    fn address(index: u32) -> AddressInfoWithDerivation {
        let address = Address::from_str("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
            .unwrap()
            .require_network(Network::Bitcoin)
            .unwrap();

        AddressInfoWithDerivation::new(
            cove_types::address::AddressInfo::from(AddressInfo {
                address,
                index,
                keychain: KeychainKind::External,
            }),
            None,
        )
    }

    #[test]
    fn new_request_supersedes_previous_request() {
        let mut session = ReceiveAddressSession::default();

        let first = session.next_request_id();
        let second = session.next_request_id();

        assert!(!session.is_current(first));
        assert!(session.is_current(second));
    }

    #[test]
    fn close_only_clears_current_request() {
        let mut session = ReceiveAddressSession::default();

        let first = session.next_request_id();
        let second = session.next_request_id();
        let state = ReceiveAddressState::cached(
            second,
            address(0),
            ReceiveAddressStatus::CachedUnused,
            100,
        );
        session.set_visible(state);

        session.close(first);
        assert!(session.visible_state().is_some());

        assert!(session.close(second));
        assert!(session.visible_state().is_none());
    }

    #[test]
    fn cached_state_expires_exactly_five_minutes_after_visible_window_start() {
        let state = ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::Fresh, 1_000);

        assert_eq!(state.expires_at_secs, Some(1_000 + CACHE_WINDOW.as_secs()));
    }

    #[test]
    fn refresh_delay_uses_visible_window_expiry() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 2_000);

        assert_eq!(state.refresh_delay(2_000), Some(Duration::from_secs(300)));
        assert_eq!(state.refresh_delay(2_299), Some(Duration::from_secs(1)));
        assert_eq!(state.refresh_delay(2_300), Some(Duration::from_secs(0)));
    }

    #[test]
    fn payment_received_has_no_refresh_delay() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 2_000)
                .payment_received();

        assert_eq!(state.refresh_delay(2_000), None);
    }

    #[test]
    fn payment_received_keeps_address() {
        let state =
            ReceiveAddressState::cached(1, address(7), ReceiveAddressStatus::CachedUnused, 1_000);

        let paid = state.payment_received();

        assert_eq!(paid.request_id, state.request_id);
        assert_eq!(paid.address.info.index, state.address.info.index);
        assert_eq!(paid.status, ReceiveAddressStatus::PaymentReceived);
        assert_eq!(paid.expires_at_secs, None);
    }

    #[test]
    fn default_presentation_copies_with_idle_refresh_state() {
        let session = ReceiveAddressSession::default();

        let presentation = session.presentation();

        assert_eq!(presentation.copy_policy, ReceiveAddressCopyPolicy::Copy);
        assert_eq!(presentation.refresh_state, ReceiveAddressRefreshState::Idle);
    }

    #[test]
    fn presentation_payment_received_confirms_copy() {
        let mut session = ReceiveAddressSession::default();
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000)
                .payment_received();
        session.set_visible(state);

        let presentation = session.presentation();

        assert_eq!(presentation.copy_policy, ReceiveAddressCopyPolicy::ConfirmPaidAddress);
        assert_eq!(presentation.refresh_state, ReceiveAddressRefreshState::Idle);
    }

    #[test]
    fn presentation_uses_session_refresh_state() {
        let mut session = ReceiveAddressSession::default();

        session.set_refresh_state(ReceiveAddressRefreshState::Refreshing);
        assert_eq!(session.presentation().refresh_state, ReceiveAddressRefreshState::Refreshing);

        session.set_refresh_state(ReceiveAddressRefreshState::Failed);
        assert_eq!(session.presentation().refresh_state, ReceiveAddressRefreshState::Failed);
    }

    #[test]
    fn not_expired_refresh_request_does_not_rotate() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state =
            ReceiveAddressState::cached(request_id, address(0), ReceiveAddressStatus::Fresh, 100);
        session.set_visible(state.clone());

        let decision = session.refresh_expired_decision(request_id, 399);

        assert_eq!(decision, RefreshExpiredAddressDecision::ReturnVisible(state));
    }

    #[test]
    fn payment_received_refresh_request_does_not_rotate() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            request_id,
            address(0),
            ReceiveAddressStatus::CachedUnused,
            100,
        )
        .payment_received();
        session.set_visible(state.clone());

        let decision = session.refresh_expired_decision(request_id, 500);

        assert_eq!(decision, RefreshExpiredAddressDecision::ReturnVisible(state));
    }

    #[test]
    fn failed_refresh_request_does_not_rotate() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            request_id,
            address(0),
            ReceiveAddressStatus::CachedUnused,
            100,
        );
        session.set_visible(state.clone());
        session.set_refresh_state(ReceiveAddressRefreshState::Failed);

        let decision = session.refresh_expired_decision(request_id, 500);

        assert_eq!(decision, RefreshExpiredAddressDecision::ReturnVisible(state));
    }

    #[test]
    fn stale_refresh_request_returns_current_visible_state() {
        let mut session = ReceiveAddressSession::default();
        let stale_request_id = session.next_request_id();
        let current_request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            current_request_id,
            address(0),
            ReceiveAddressStatus::Fresh,
            100,
        );
        session.set_visible(state.clone());

        let decision = session.refresh_expired_decision(stale_request_id, 500);

        assert_eq!(decision, RefreshExpiredAddressDecision::ReturnVisible(state));
    }

    #[test]
    fn expired_current_refresh_request_rotates() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            request_id,
            address(0),
            ReceiveAddressStatus::CachedUnused,
            100,
        );
        session.set_visible(state);

        let decision = session.refresh_expired_decision(request_id, 400);

        assert_eq!(decision, RefreshExpiredAddressDecision::Rotate);
    }

    #[test]
    fn current_visible_address_can_be_marked_payment_received() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state =
            ReceiveAddressState::cached(request_id, address(7), ReceiveAddressStatus::Fresh, 100);
        session.set_visible(state);

        let state = session.mark_payment_received(request_id, 7).unwrap();

        assert_eq!(state.status, ReceiveAddressStatus::PaymentReceived);
        assert_eq!(state.expires_at_secs, None);
        assert_eq!(session.visible_state().unwrap().status, ReceiveAddressStatus::PaymentReceived);
    }

    #[test]
    fn stale_request_cannot_be_marked_payment_received() {
        let mut session = ReceiveAddressSession::default();
        let stale_request_id = session.next_request_id();
        let current_request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            current_request_id,
            address(7),
            ReceiveAddressStatus::Fresh,
            100,
        );
        session.set_visible(state);

        let state = session.mark_payment_received(stale_request_id, 7);

        assert_eq!(state, None);
        assert_eq!(session.visible_state().unwrap().status, ReceiveAddressStatus::Fresh);
    }

    #[test]
    fn wrong_derivation_index_cannot_be_marked_payment_received() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state =
            ReceiveAddressState::cached(request_id, address(7), ReceiveAddressStatus::Fresh, 100);
        session.set_visible(state);

        let state = session.mark_payment_received(request_id, 8);

        assert_eq!(state, None);
        assert_eq!(session.visible_state().unwrap().status, ReceiveAddressStatus::Fresh);
    }

    #[test]
    fn already_paid_address_is_not_marked_again() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state =
            ReceiveAddressState::cached(request_id, address(7), ReceiveAddressStatus::Fresh, 100)
                .payment_received();
        session.set_visible(state);

        let state = session.mark_payment_received(request_id, 7);

        assert_eq!(state, None);
        assert_eq!(session.visible_state().unwrap().status, ReceiveAddressStatus::PaymentReceived);
    }
}

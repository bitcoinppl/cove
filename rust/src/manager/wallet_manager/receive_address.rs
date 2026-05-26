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

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ReceiveAddressPresentation {
    pub copy_policy: ReceiveAddressCopyPolicy,
    pub countdown_remaining_secs: Option<u64>,
    pub should_refresh_now: bool,
    pub show_refreshing: bool,
    pub show_refresh_error: bool,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct ReceiveAddressState {
    pub request_id: u64,
    pub address: Arc<AddressInfoWithDerivation>,
    pub status: ReceiveAddressStatus,
    pub first_shown_at_secs: u64,
    pub expires_at_secs: Option<u64>,
    pub refresh_error: Option<String>,
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
            refresh_error: None,
        }
    }

    pub fn payment_received(&self) -> Self {
        Self {
            status: ReceiveAddressStatus::PaymentReceived,
            expires_at_secs: None,
            refresh_error: None,
            ..self.clone()
        }
    }

    pub fn refresh_failed(&self, error: String) -> Self {
        Self { refresh_error: Some(error), ..self.clone() }
    }

    fn should_refresh_at(&self, now_secs: u64) -> bool {
        self.status != ReceiveAddressStatus::PaymentReceived
            && self.refresh_error.is_none()
            && self.expires_at_secs.is_some_and(|expires_at_secs| now_secs >= expires_at_secs)
    }
}

#[uniffi::export]
pub fn receive_address_presentation(
    state: Option<ReceiveAddressState>,
    now_secs: u64,
    is_refreshing: bool,
) -> ReceiveAddressPresentation {
    let Some(state) = state else {
        return ReceiveAddressPresentation {
            copy_policy: ReceiveAddressCopyPolicy::Copy,
            countdown_remaining_secs: None,
            should_refresh_now: false,
            show_refreshing: false,
            show_refresh_error: false,
        };
    };

    let is_paid = state.status == ReceiveAddressStatus::PaymentReceived;
    let should_refresh = state.should_refresh_at(now_secs);
    let countdown_remaining_secs = if is_paid || should_refresh {
        None
    } else {
        state
            .expires_at_secs
            .and_then(|expires_at_secs| expires_at_secs.checked_sub(now_secs))
            .filter(|remaining_secs| (1..=60).contains(remaining_secs))
    };

    ReceiveAddressPresentation {
        copy_policy: if is_paid {
            ReceiveAddressCopyPolicy::ConfirmPaidAddress
        } else {
            ReceiveAddressCopyPolicy::Copy
        },
        countdown_remaining_secs,
        should_refresh_now: should_refresh && !is_refreshing,
        show_refreshing: should_refresh && is_refreshing,
        show_refresh_error: state.refresh_error.is_some(),
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

    pub fn close(&mut self, request_id: u64) {
        if self.is_current(request_id) {
            self.active_request_id = None;
            self.visible_state = None;
        }
    }

    pub fn refresh_expired_decision(
        &self,
        request_id: u64,
        now_secs: u64,
    ) -> RefreshExpiredAddressDecision {
        let Some(visible_state) = self.visible_state() else {
            return RefreshExpiredAddressDecision::MissingVisibleState;
        };

        if !self.is_current(request_id) || !visible_state.should_refresh_at(now_secs) {
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

        session.close(second);
        assert!(session.visible_state().is_none());
    }

    #[test]
    fn cached_state_expires_exactly_five_minutes_after_visible_window_start() {
        let state = ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::Fresh, 1_000);

        assert_eq!(state.expires_at_secs, Some(1_000 + CACHE_WINDOW.as_secs()));
    }

    #[test]
    fn reopened_cached_state_gets_fresh_visible_window() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 2_000);

        let presentation = receive_address_presentation(Some(state), 2_000, false);

        assert_eq!(presentation.countdown_remaining_secs, None);
        assert!(!presentation.should_refresh_now);
    }

    #[test]
    fn payment_received_stops_countdown_and_keeps_address() {
        let state =
            ReceiveAddressState::cached(1, address(7), ReceiveAddressStatus::CachedUnused, 1_000);

        let paid = state.payment_received();

        assert_eq!(paid.request_id, state.request_id);
        assert_eq!(paid.address.info.index, state.address.info.index);
        assert_eq!(paid.status, ReceiveAddressStatus::PaymentReceived);
        assert_eq!(paid.expires_at_secs, None);
    }

    #[test]
    fn refresh_failure_keeps_visible_address() {
        let state =
            ReceiveAddressState::cached(1, address(3), ReceiveAddressStatus::CachedUnused, 1_000);

        let failed = state.refresh_failed("node unavailable".to_string());

        assert_eq!(failed.request_id, state.request_id);
        assert_eq!(failed.address.info.index, state.address.info.index);
        assert_eq!(failed.refresh_error.as_deref(), Some("node unavailable"));
    }

    #[test]
    fn presentation_hides_countdown_before_final_minute() {
        let state = ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::Fresh, 1_000);

        let presentation = receive_address_presentation(Some(state), 1_000, false);

        assert_eq!(presentation.countdown_remaining_secs, None);
        assert!(!presentation.should_refresh_now);
    }

    #[test]
    fn presentation_shows_countdown_from_sixty_to_one_seconds() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000);

        let at_sixty = receive_address_presentation(Some(state.clone()), 1_240, false);
        let at_one = receive_address_presentation(Some(state), 1_299, false);

        assert_eq!(at_sixty.countdown_remaining_secs, Some(60));
        assert_eq!(at_one.countdown_remaining_secs, Some(1));
    }

    #[test]
    fn presentation_expired_cached_state_should_refresh_now() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000);

        let presentation = receive_address_presentation(Some(state), 1_300, false);

        assert!(presentation.should_refresh_now);
        assert!(!presentation.show_refreshing);
    }

    #[test]
    fn presentation_expired_refreshing_state_shows_refreshing() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000);

        let presentation = receive_address_presentation(Some(state), 1_300, true);

        assert!(!presentation.should_refresh_now);
        assert!(presentation.show_refreshing);
    }

    #[test]
    fn presentation_payment_received_hides_countdown_and_confirms_copy() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000)
                .payment_received();

        let presentation = receive_address_presentation(Some(state), 1_240, false);

        assert_eq!(presentation.copy_policy, ReceiveAddressCopyPolicy::ConfirmPaidAddress);
        assert_eq!(presentation.countdown_remaining_secs, None);
        assert!(!presentation.should_refresh_now);
    }

    #[test]
    fn presentation_refresh_error_maps_to_show_refresh_error() {
        let state =
            ReceiveAddressState::cached(1, address(0), ReceiveAddressStatus::CachedUnused, 1_000)
                .refresh_failed("node unavailable".to_string());

        let presentation = receive_address_presentation(Some(state), 1_300, false);

        assert!(presentation.show_refresh_error);
        assert!(!presentation.should_refresh_now);
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
    fn refresh_error_refresh_request_does_not_rotate() {
        let mut session = ReceiveAddressSession::default();
        let request_id = session.next_request_id();
        let state = ReceiveAddressState::cached(
            request_id,
            address(0),
            ReceiveAddressStatus::CachedUnused,
            100,
        )
        .refresh_failed("node unavailable".to_string());
        session.set_visible(state.clone());

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

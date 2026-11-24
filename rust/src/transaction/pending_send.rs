//! How a pending payment this wallet sent is doing, for its list row and details
//!
//! Rust decides the status, so both apps show the same thing: a send pending for over half an
//! hour shows how long it has waited, and one pending for over an hour at a rate below what the
//! network asks now, which the wallet can bump, shows as taking longer than usual

use std::sync::Arc;

use bdk_wallet::{
    Wallet as BdkWallet, WalletTx,
    chain::{ChainPosition, ConfirmationBlockTime},
};
use bitcoin::{FeeRate as BdkFeeRate, Txid};

use crate::{
    fee_client::FEE_CLIENT,
    transaction::{
        FeeRate, Transaction, TransactionDirection,
        fee_bump::{self, FeeBumpWallet},
    },
};

/// How long a send stays pending before its row says how long it has waited
const WAITING_AFTER_SECS: u64 = 30 * 60;

/// How long a send stays pending before it can count as delayed
const DELAYED_AFTER_SECS: u64 = 60 * 60;

/// Set by UI tests to reach the delayed state without waiting an hour, to the delay in seconds;
/// a send then counts as waiting after half of it. It changes what the app tells the user, so
/// it is read only in builds with debug assertions, which no release profile enables
#[cfg(debug_assertions)]
const DELAY_OVERRIDE_ENV: &str = "COVE_UI_TEST_PENDING_SEND_DELAY_SECS";

/// How a pending payment this wallet sent is doing
///
/// A received or confirmed transaction has none. A send the wallet can't bump, such as one
/// paying several recipients, is never delayed, since nothing could be done about it
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PendingSendStatus {
    /// Pending for under 30 minutes, which needs no mention
    Recent { bumpable: bool },

    /// Pending for over 30 minutes without being delayed, shown as "Unconfirmed · 32m"
    Waiting {
        /// When the wallet first saw it, in unix seconds
        since: u64,
        bumpable: bool,
    },

    /// Pending for over an hour, at a rate below the network's current Medium rate, and the
    /// wallet can bump its fee, so it shows as taking longer than usual
    Delayed {
        /// When the wallet first saw it, in unix seconds
        since: u64,

        /// The rate it pays
        fee_rate: Arc<FeeRate>,
    },
}

impl PendingSendStatus {
    /// Whether the wallet can replace it with one paying a higher fee
    pub const fn is_bumpable(&self) -> bool {
        match self {
            Self::Recent { bumpable } | Self::Waiting { bumpable, .. } => *bumpable,
            Self::Delayed { .. } => true,
        }
    }

    const fn since(&self) -> Option<u64> {
        match self {
            Self::Recent { .. } => None,
            Self::Waiting { since, .. } | Self::Delayed { since, .. } => Some(*since),
        }
    }

    /// How long it has been pending at `now`, as "1h 32m", or `None` while it is recent
    pub fn elapsed_fmt_at(&self, now: u64) -> Option<String> {
        let since = self.since()?;
        Some(elapsed_fmt(now.saturating_sub(since)))
    }
}

#[uniffi::export]
impl PendingSendStatus {
    #[uniffi::method(name = "isBumpable")]
    fn ffi_is_bumpable(&self) -> bool {
        self.is_bumpable()
    }

    #[uniffi::method(name = "isDelayed")]
    fn ffi_is_delayed(&self) -> bool {
        matches!(self, Self::Delayed { .. })
    }

    /// How long it has been pending, as "1h 32m" or "45m", or `None` while it is recent
    ///
    /// Counts to the current time, so a screen calls it again every minute to keep it current
    #[uniffi::method(name = "elapsedFmt")]
    fn ffi_elapsed_fmt(&self) -> Option<String> {
        self.elapsed_fmt_at(cove_util::time::unix_timestamp_secs_or_zero())
    }
}

/// When a send's status moves from recent to waiting, and from waiting to delayed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSendThresholds {
    waiting_after: u64,
    delayed_after: u64,
}

impl PendingSendThresholds {
    pub const DEFAULT: Self =
        Self { waiting_after: WAITING_AFTER_SECS, delayed_after: DELAYED_AFTER_SECS };

    /// The thresholds in use, which a UI test can shorten in a debug build
    pub fn current() -> Self {
        // read on every list build, not once, so an Android instrumentation test can set it
        // in the app process after launch
        #[cfg(debug_assertions)]
        if let Some(delayed_after) = std::env::var(DELAY_OVERRIDE_ENV)
            .ok()
            .and_then(|secs| secs.trim().parse::<u64>().ok())
            .filter(|secs| *secs > 0)
        {
            return Self::delayed_after(delayed_after);
        }

        Self::DEFAULT
    }

    /// Counts as delayed after `delayed_after` seconds, and as waiting after half of it
    pub const fn delayed_after(delayed_after: u64) -> Self {
        Self { waiting_after: delayed_after / 2, delayed_after }
    }

    /// The status at `now` of a send pending since `since`
    ///
    /// `replaceable_at` is the rate it pays when the wallet can bump it, and `medium` the
    /// network's current Medium rate when one is cached, without which no send counts as
    /// delayed
    pub fn classify(
        self,
        since: u64,
        now: u64,
        replaceable_at: Option<BdkFeeRate>,
        medium: Option<FeeRate>,
    ) -> PendingSendStatus {
        let pending_for = now.saturating_sub(since);
        let bumpable = replaceable_at.is_some();
        if pending_for < self.waiting_after {
            return PendingSendStatus::Recent { bumpable };
        }

        let fee_rate = replaceable_at.map(FeeRate::from);
        match (fee_rate, medium) {
            (Some(fee_rate), Some(medium))
                if pending_for >= self.delayed_after && fee_rate < medium =>
            {
                PendingSendStatus::Delayed { since, fee_rate: Arc::new(fee_rate) }
            }

            _ => PendingSendStatus::Waiting { since, bumpable },
        }
    }

    /// When the status of a send pending since `since`, `status` at `now`, next changes
    /// with time alone
    ///
    /// A send whose rate is at or above Medium an hour in stays waiting until the rates or
    /// the transaction change, which rebuild the list anyway
    pub fn next_change_at(self, since: u64, now: u64, status: &PendingSendStatus) -> Option<u64> {
        let delayed_at = since.saturating_add(self.delayed_after);

        match status {
            PendingSendStatus::Recent { .. } => Some(since.saturating_add(self.waiting_after)),
            PendingSendStatus::Waiting { bumpable: true, .. } if now < delayed_at => {
                Some(delayed_at)
            }

            PendingSendStatus::Waiting { .. } | PendingSendStatus::Delayed { .. } => None,
        }
    }
}

/// What a send's status depends on beyond the transaction, read once for a whole list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSendContext {
    wallet: FeeBumpWallet,
    medium: Option<FeeRate>,
    now: u64,
    thresholds: PendingSendThresholds,
}

impl PendingSendContext {
    /// The context right now, with the cached network rates
    pub fn current(wallet: FeeBumpWallet) -> Self {
        let medium = FEE_CLIENT.cached_fee_rate_options().map(|options| options.medium.fee_rate);
        Self::with_medium(wallet, medium)
    }

    /// The context right now without network rates, for a wallet that keeps nothing on disk
    ///
    /// The cached rates can load from the app database, which a preview wallet must not open.
    /// With no Medium rate no send counts as delayed
    pub fn without_rates(wallet: FeeBumpWallet) -> Self {
        Self::with_medium(wallet, None)
    }

    fn with_medium(wallet: FeeBumpWallet, medium: Option<FeeRate>) -> Self {
        Self {
            wallet,
            medium,
            now: cove_util::time::unix_timestamp_secs_or_zero(),
            thresholds: PendingSendThresholds::current(),
        }
    }

    /// When `tx` was first seen, falling back to when it was last seen or to now
    ///
    /// The canonical view only lists a pending transaction it saw in a mempool, so the
    /// fallbacks are for a graph that lost its timestamps
    pub fn since(&self, chain_position: &ChainPosition<ConfirmationBlockTime>) -> u64 {
        match chain_position {
            ChainPosition::Unconfirmed { first_seen, last_seen } => {
                first_seen.or(*last_seen).unwrap_or(self.now)
            }
            ChainPosition::Confirmed { .. } => self.now,
        }
    }

    /// The status of `tx` when it is a pending payment this wallet sent
    ///
    /// `is_canonical` says whether another transaction is in the same canonical view as `tx`,
    /// see [`fee_bump::check_replaceable`]
    pub fn status(
        &self,
        bdk: &BdkWallet,
        tx: &WalletTx<'_>,
        direction: TransactionDirection,
        is_canonical: impl Fn(Txid) -> bool,
    ) -> Option<PendingSendStatus> {
        if tx.chain_position.is_confirmed() || direction != TransactionDirection::Outgoing {
            return None;
        }

        let replaceable_at = fee_bump::check_replaceable(bdk, self.wallet, tx, is_canonical)
            .ok()
            .map(|replaceable| replaceable.fee_rate());

        let since = self.since(&tx.chain_position);
        Some(self.thresholds.classify(since, self.now, replaceable_at, self.medium))
    }

    /// The soonest time any row in `transactions` changes status with time alone
    pub fn next_change_at(&self, transactions: &[Transaction]) -> Option<u64> {
        transactions
            .iter()
            .filter_map(|transaction| match transaction {
                Transaction::Unconfirmed(unconfirmed) => Some(unconfirmed),
                Transaction::Confirmed(_) => None,
            })
            .filter_map(|unconfirmed| {
                let status = unconfirmed.pending_send.as_ref()?;
                self.thresholds.next_change_at(unconfirmed.first_seen, self.now, status)
            })
            // a change already due was applied in this build, so it never schedules a rebuild
            .filter(|at| *at > self.now)
            .min()
    }

    pub const fn now(&self) -> u64 {
        self.now
    }
}

/// "45m", "1h 32m" or "2d 3h", and "<1m" for the first minute, which only a UI test's short
/// thresholds show
fn elapsed_fmt(secs: u64) -> String {
    let minutes = secs / 60;
    let (days, hours, minutes) = (minutes / (24 * 60), minutes / 60 % 24, minutes % 60);

    match (days, hours, minutes) {
        (0, 0, 0) => "<1m".to_string(),
        (0, 0, minutes) => format!("{minutes}m"),
        (0, hours, minutes) => format!("{hours}h {minutes}m"),
        (days, hours, _) => format!("{days}d {hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINCE: u64 = 1_700_000_000;
    const MINUTE: u64 = 60;

    fn sat_per_vb(rate: u32) -> BdkFeeRate {
        BdkFeeRate::from_sat_per_vb_u32(rate)
    }

    fn medium(rate: u32) -> FeeRate {
        sat_per_vb(rate).into()
    }

    fn classify(
        pending_for: u64,
        replaceable_at: Option<BdkFeeRate>,
        medium: Option<FeeRate>,
    ) -> PendingSendStatus {
        PendingSendThresholds::DEFAULT.classify(SINCE, SINCE + pending_for, replaceable_at, medium)
    }

    fn waiting(bumpable: bool) -> PendingSendStatus {
        PendingSendStatus::Waiting { since: SINCE, bumpable }
    }

    fn delayed(rate: u32) -> PendingSendStatus {
        PendingSendStatus::Delayed { since: SINCE, fee_rate: Arc::new(sat_per_vb(rate).into()) }
    }

    #[test]
    fn a_send_waits_after_half_an_hour_and_is_delayed_after_an_hour_below_medium() {
        let low = Some(sat_per_vb(2));

        assert_eq!(
            classify(0, low, Some(medium(10))),
            PendingSendStatus::Recent { bumpable: true }
        );
        assert_eq!(
            classify(30 * MINUTE - 1, low, Some(medium(10))),
            PendingSendStatus::Recent { bumpable: true }
        );
        assert_eq!(classify(30 * MINUTE, low, Some(medium(10))), waiting(true));
        assert_eq!(classify(60 * MINUTE - 1, low, Some(medium(10))), waiting(true));
        assert_eq!(classify(60 * MINUTE, low, Some(medium(10))), delayed(2));
        assert_eq!(classify(5 * 60 * MINUTE, low, Some(medium(10))), delayed(2));
    }

    #[test]
    fn a_send_paying_at_least_medium_is_never_delayed() {
        assert_eq!(
            classify(2 * 60 * MINUTE, Some(sat_per_vb(10)), Some(medium(10))),
            waiting(true)
        );
        assert_eq!(
            classify(2 * 60 * MINUTE, Some(sat_per_vb(12)), Some(medium(10))),
            waiting(true)
        );
    }

    #[test]
    fn without_cached_rates_a_send_is_never_delayed() {
        assert_eq!(classify(2 * 60 * MINUTE, Some(sat_per_vb(2)), None), waiting(true));
    }

    #[test]
    fn a_send_that_cannot_be_bumped_is_never_delayed() {
        assert_eq!(
            classify(0, None, Some(medium(10))),
            PendingSendStatus::Recent { bumpable: false }
        );
        assert_eq!(classify(45 * MINUTE, None, Some(medium(10))), waiting(false));
        assert_eq!(classify(2 * 60 * MINUTE, None, Some(medium(10))), waiting(false));

        assert!(!waiting(false).is_bumpable());
        assert!(delayed(2).is_bumpable());
    }

    #[test]
    fn a_send_first_seen_in_the_future_is_recent() {
        let status = PendingSendThresholds::DEFAULT.classify(
            SINCE,
            SINCE - 10 * MINUTE,
            Some(sat_per_vb(2)),
            Some(medium(10)),
        );

        assert_eq!(status, PendingSendStatus::Recent { bumpable: true });
    }

    #[test]
    fn the_next_change_is_the_next_threshold_a_send_can_still_cross() {
        let thresholds = PendingSendThresholds::DEFAULT;
        let recent = PendingSendStatus::Recent { bumpable: false };
        let now = SINCE + 10 * MINUTE;

        assert_eq!(thresholds.next_change_at(SINCE, now, &recent), Some(SINCE + 30 * MINUTE));

        let now = SINCE + 40 * MINUTE;
        assert_eq!(thresholds.next_change_at(SINCE, now, &waiting(true)), Some(SINCE + 3_600));
        assert_eq!(thresholds.next_change_at(SINCE, now, &waiting(false)), None);

        // past the hour at or above medium, only new rates could make it delayed
        let now = SINCE + 90 * MINUTE;
        assert_eq!(thresholds.next_change_at(SINCE, now, &waiting(true)), None);
        assert_eq!(thresholds.next_change_at(SINCE, now, &delayed(2)), None);
    }

    #[test]
    fn a_ui_test_delay_halves_into_the_waiting_threshold() {
        let thresholds = PendingSendThresholds::delayed_after(20);
        let low = Some(sat_per_vb(2));

        assert_eq!(
            thresholds.classify(SINCE, SINCE + 9, low, Some(medium(10))),
            PendingSendStatus::Recent { bumpable: true }
        );
        assert_eq!(thresholds.classify(SINCE, SINCE + 10, low, Some(medium(10))), waiting(true));
        assert_eq!(thresholds.classify(SINCE, SINCE + 20, low, Some(medium(10))), delayed(2));
    }

    #[test]
    fn elapsed_time_shows_minutes_hours_and_days() {
        let at = |secs| delayed(2).elapsed_fmt_at(SINCE + secs);

        assert_eq!(at(30), Some("<1m".to_string()));
        assert_eq!(at(45 * MINUTE + 59), Some("45m".to_string()));
        assert_eq!(at(60 * MINUTE), Some("1h 0m".to_string()));
        assert_eq!(at(92 * MINUTE), Some("1h 32m".to_string()));
        assert_eq!(at(26 * 60 * MINUTE + 5 * MINUTE), Some("1d 2h".to_string()));
        assert_eq!(PendingSendStatus::Recent { bumpable: true }.elapsed_fmt_at(SINCE), None);
    }
}

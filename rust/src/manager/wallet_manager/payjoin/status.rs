//! The Payjoin session identity and status the wallet reports to the send screens

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bitcoin::hashes::{Hash as _, sha256};
use nid::Nanoid;
use serde::{Deserialize, Serialize};

use crate::transaction::TxId;

/// Identifies one Payjoin payment attempt, from its confirm screen to its broadcast
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, derive_more::Display)]
#[serde(transparent)]
pub struct PayjoinSessionId(String);

uniffi::custom_newtype!(PayjoinSessionId, String);

impl PayjoinSessionId {
    /// Creates the identity of a new payment attempt
    pub(crate) fn generate() -> Self {
        Self(Nanoid::<21>::new().to_string())
    }

    /// The identity of a session saved before sessions had one, derived from its original
    /// transaction so every read of that record names the same payment
    pub(crate) fn for_legacy_session(fallback_tx: &[u8]) -> Self {
        Self(format!("legacy-{}", sha256::Hash::hash(fallback_tx)))
    }
}

/// The receiver endpoint and identity that start one Payjoin payment
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct PayjoinIntent {
    /// Identifies this payment in every status the wallet reports for it
    pub session_id: PayjoinSessionId,

    /// The receiver's BIP 77 endpoint, taken from its validated pj URI
    pub endpoint: String,
}

impl PayjoinIntent {
    /// Starts a new payment attempt to `endpoint`
    pub(crate) fn new(endpoint: String) -> Self {
        Self { session_id: PayjoinSessionId::generate(), endpoint }
    }
}

/// Which transaction a Payjoin session ended by broadcasting
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PayjoinTransactionKind {
    /// The receiver's proposal, which spends a coin of the receiver's too
    Proposal,

    /// The signed original, sent as an ordinary payment
    Fallback,
}

/// The wallet's Payjoin send, as the send screens show it
///
/// Sent as `PayjoinStatusChanged` whenever it changes, and read once through
/// `RustWalletManager::payjoin_status` for the initial snapshot
#[derive(Debug, Clone, Default, Hash, Eq, PartialEq, uniffi::Enum)]
#[uniffi::export(Eq, Hash)]
pub enum PayjoinStatus {
    /// No Payjoin payment is in progress
    #[default]
    Idle,

    /// Waiting for the receiver's proposal; the original is broadcast at `expires_at_secs`
    /// unless the receiver answers or the user cancels first
    Negotiating {
        session_id: PayjoinSessionId,

        /// Unix time in seconds
        expires_at_secs: u64,
    },

    /// The session's final transaction is being broadcast
    Broadcasting { session_id: PayjoinSessionId, txid: Arc<TxId>, kind: PayjoinTransactionKind },

    /// The session's final transaction was broadcast and saved to the wallet
    Completed { session_id: PayjoinSessionId, txid: Arc<TxId>, kind: PayjoinTransactionKind },
}

/// When a Payjoin session stops waiting for the receiver and broadcasts the original
///
/// Counted from the session's creation, so the countdown the send screen shows and the
/// negotiating actor's own deadline are the same instant, before and after a restart
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct PayjoinDeadline {
    expires_at_secs: u64,
}

impl PayjoinDeadline {
    /// The deadline of a session created at `created_at_secs`
    ///
    /// A record from before creation times were stored starts its wait over from now
    pub(crate) fn for_session(created_at_secs: Option<u64>, session_timeout: Duration) -> Self {
        let created_at_secs = created_at_secs.unwrap_or_else(unix_now_secs);
        Self { expires_at_secs: created_at_secs.saturating_add(session_timeout.as_secs()) }
    }

    pub(crate) const fn expires_at_secs(self) -> u64 {
        self.expires_at_secs
    }

    pub(crate) fn has_passed(self) -> bool {
        unix_now_secs() >= self.expires_at_secs
    }

    /// The deadline on the monotonic clock, so a later wall clock change cannot move it
    pub(crate) fn instant(self) -> Instant {
        let remaining = self.expires_at_secs.saturating_sub(unix_now_secs());
        Instant::now() + Duration::from_secs(remaining)
    }
}

pub(crate) fn unix_now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

use std::sync::Arc;

use cove_macros::new_type;
use nid::Nanoid;

new_type!(PayjoinSessionId, String);

impl PayjoinSessionId {
    /// Creates a new stable identity for one Payjoin payment attempt
    #[must_use]
    pub fn generate() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }
}

/// A BIP-77 v2 endpoint that passed Payjoin URI validation
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct PayjoinEndpoint(String);

impl PayjoinEndpoint {
    /// Creates an endpoint from a value that already passed Payjoin URI validation
    #[must_use]
    pub(crate) fn from_validated(endpoint: String) -> Arc<Self> {
        Arc::new(Self(endpoint))
    }

    /// Returns the validated endpoint string
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The immutable data that identifies and starts one Payjoin payment
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
#[uniffi::export(Eq, Hash)]
pub struct PayjoinIntent {
    /// The stable identity of this payment attempt
    pub session_id: PayjoinSessionId,
    /// The validated receiver endpoint
    pub endpoint: Arc<PayjoinEndpoint>,
}

impl PayjoinIntent {
    /// Creates a fresh payment intent for a validated Payjoin endpoint
    #[must_use]
    pub fn new(endpoint: Arc<PayjoinEndpoint>) -> Self {
        Self { session_id: PayjoinSessionId::generate(), endpoint }
    }
}

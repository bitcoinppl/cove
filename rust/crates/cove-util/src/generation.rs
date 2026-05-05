use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Captured generation value that can cross async and platform boundaries
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct GenerationToken {
    pub value: u64,
}

/// Shared source of truth for stale async-work tokens
///
/// A generation token only answers whether work was started before the latest
/// invalidation. It does not serialize later state mutations; owners of the
/// mutated state must check the token and apply changes at their own
/// serialization boundary.
#[derive(Debug, Clone, Default, uniffi::Object)]
pub struct GenerationTracker(Arc<AtomicU64>);

/// Rust-owned pairing of a shared tracker and this operation's captured token
#[derive(Debug, Clone)]
pub struct GenerationClaim {
    shared_tracker: GenerationTracker,
    captured_token: GenerationToken,
}

impl GenerationTracker {
    #[must_use]
    pub fn claim(&self) -> GenerationClaim {
        GenerationClaim { shared_tracker: self.clone(), captured_token: self.advance() }
    }

    pub fn invalidate(&self) {
        let _ = self.advance();
    }
}

impl GenerationClaim {
    #[must_use]
    pub fn token(&self) -> GenerationToken {
        self.captured_token
    }

    #[must_use]
    pub fn is_current(&self) -> bool {
        self.shared_tracker.is_current(self.captured_token)
    }
}

#[uniffi::export]
impl GenerationTracker {
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn advance(&self) -> GenerationToken {
        let value = self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1);

        GenerationToken { value }
    }

    #[must_use]
    pub fn capture(&self) -> GenerationToken {
        GenerationToken { value: self.0.load(Ordering::Relaxed) }
    }

    #[must_use]
    pub fn is_current(&self, captured_token: GenerationToken) -> bool {
        self.0.load(Ordering::Relaxed) == captured_token.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_token_is_current() {
        let tracker = GenerationTracker::new();
        let token = tracker.capture();

        assert!(tracker.is_current(token));
    }

    #[test]
    fn advance_invalidates_older_tokens() {
        let tracker = GenerationTracker::new();
        let old_token = tracker.capture();
        let new_token = tracker.advance();

        assert!(!tracker.is_current(old_token));
        assert!(tracker.is_current(new_token));
    }

    #[test]
    fn claim_is_current_until_tracker_advances() {
        let tracker = GenerationTracker::new();
        let claim = tracker.claim();

        assert!(claim.is_current());

        tracker.invalidate();

        assert!(!claim.is_current());
    }
}

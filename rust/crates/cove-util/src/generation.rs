use std::sync::Arc;

use parking_lot::Mutex;

/// Captured generation value that can cross async and platform boundaries
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct GenerationToken {
    pub value: u64,
}

/// Shared source of truth for the current generation
#[derive(Debug, Clone, Default, uniffi::Object)]
pub struct GenerationTracker(Arc<Mutex<u64>>);

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

    #[must_use]
    pub fn run_if_current<T>(
        &self,
        captured_token: GenerationToken,
        update: impl FnOnce() -> T,
    ) -> Option<T> {
        let current = self.0.lock();
        if *current != captured_token.value {
            return None;
        }

        Some(update())
    }

    #[must_use]
    pub fn run_result_if_current<T, E>(
        &self,
        captured_token: GenerationToken,
        update: impl FnOnce() -> Result<T, E>,
    ) -> Option<Result<T, E>> {
        let current = self.0.lock();
        if *current != captured_token.value {
            return None;
        }

        Some(update())
    }
}

impl GenerationClaim {
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.shared_tracker.is_current(self.captured_token)
    }

    #[must_use]
    pub fn run_if_current<T>(&self, update: impl FnOnce() -> T) -> Option<T> {
        self.shared_tracker.run_if_current(self.captured_token, update)
    }

    #[must_use]
    pub fn run_result_if_current<T, E>(
        &self,
        update: impl FnOnce() -> Result<T, E>,
    ) -> Option<Result<T, E>> {
        self.shared_tracker.run_result_if_current(self.captured_token, update)
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
        let mut current = self.0.lock();
        *current = current.wrapping_add(1);

        GenerationToken { value: *current }
    }

    #[must_use]
    pub fn capture(&self) -> GenerationToken {
        GenerationToken { value: *self.0.lock() }
    }

    #[must_use]
    pub fn is_current(&self, captured_token: GenerationToken) -> bool {
        *self.0.lock() == captured_token.value
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
    fn stale_token_does_not_run_update() {
        let tracker = GenerationTracker::new();
        let stale_token = tracker.capture();
        let _ = tracker.advance();

        let result = tracker.run_if_current(stale_token, || 1);

        assert_eq!(result, None);
    }

    #[test]
    fn current_token_runs_result_update() {
        let tracker = GenerationTracker::new();
        let token = tracker.advance();

        let result = tracker.run_result_if_current(token, || Ok::<_, ()>(1));

        assert_eq!(result, Some(Ok(1)));
    }

    #[test]
    fn claim_is_current_until_tracker_advances() {
        let tracker = GenerationTracker::new();
        let claim = tracker.claim();

        assert!(claim.is_current());

        tracker.invalidate();

        assert!(!claim.is_current());
    }

    #[test]
    fn stale_claim_does_not_run_update() {
        let tracker = GenerationTracker::new();
        let claim = tracker.claim();
        tracker.invalidate();

        let result = claim.run_if_current(|| 1);

        assert_eq!(result, None);
    }
}

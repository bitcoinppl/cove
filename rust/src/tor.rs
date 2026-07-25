use serde::{Deserialize, Serialize};

/// Persisted Tor routing preference
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum TorConfig {
    /// Connect without Tor
    #[default]
    Off,
    /// Connect through Cove's built-in Tor runtime
    BuiltIn,
    /// Connect through an external SOCKS5 proxy
    External { host: String, port: u16 },
}

/// Crash-loop bookkeeping for built-in Tor auto-start
///
/// Arti terminates the process when the consensus marks the build obsolete, which
/// presents as a launch crash loop. Counting attempts that never reached a healthy
/// runtime lets warmup stop auto-starting instead of crashing on every launch
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TorLaunchHealth {
    consecutive_early_exits: u32,
}

impl TorLaunchHealth {
    const TRIP_THRESHOLD: u32 = 3;

    /// Records that a built-in Tor start is about to be attempted
    pub(crate) fn record_attempt(&mut self) {
        self.consecutive_early_exits = self.consecutive_early_exits.saturating_add(1);
    }

    /// Clears the crash-loop count after a runtime proves itself healthy
    pub(crate) fn mark_healthy(&mut self) {
        self.consecutive_early_exits = 0;
    }

    /// Returns whether auto-start should be skipped until the user asks again
    pub(crate) const fn suppresses_auto_start(&self) -> bool {
        self.consecutive_early_exits >= Self::TRIP_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::TorLaunchHealth;

    #[test]
    fn launch_health_trips_after_three_unhealthy_attempts() {
        let mut health = TorLaunchHealth::default();

        assert!(!health.suppresses_auto_start());

        health.record_attempt();
        health.record_attempt();
        assert!(!health.suppresses_auto_start());

        health.record_attempt();
        assert!(health.suppresses_auto_start());
    }

    #[test]
    fn launch_health_resets_when_a_runtime_proves_healthy() {
        let mut health = TorLaunchHealth::default();
        for _ in 0..5 {
            health.record_attempt();
        }

        health.mark_healthy();

        assert!(!health.suppresses_auto_start());
        assert_eq!(health, TorLaunchHealth::default());
    }
}

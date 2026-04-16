/// Aggregate lock state for a set of outpoints.
///
/// Used by Transaction Details to render a single icon and implement
/// the three-state bulk toggle:
///
/// - `Unlocked` — none locked, tap locks all
/// - `Mixed`    — some locked, tap locks remaining unlocked
/// - `Locked`   — all locked, tap unlocks all
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LockState {
    /// No outpoints in the set are locked.
    Unlocked,

    /// Some outpoints are locked, some are not.
    Mixed,

    /// Every outpoint in the set is locked.
    Locked,
}

impl std::fmt::Display for LockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unlocked => write!(f, "unlocked"),
            Self::Mixed => write!(f, "mixed"),
            Self::Locked => write!(f, "locked"),
        }
    }
}

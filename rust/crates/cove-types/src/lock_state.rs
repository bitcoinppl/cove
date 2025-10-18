use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum LockState {
    Locked,
    Unlocked,
}

impl Default for LockState {
    fn default() -> Self {
        Self::Locked
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum UnlockMode {
    Main,
    Decoy,
    Wipe,
    Locked,
}

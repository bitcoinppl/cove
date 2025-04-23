use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record,
)]
pub struct BlockSizeLast {
    pub block_height: u64,
    pub last_seen: Duration,
}

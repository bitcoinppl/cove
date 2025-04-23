use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, uniffi::Record)]
pub struct AddressIndex {
    pub last_seen_index: u8,
    pub address_list_hash: u64,
}

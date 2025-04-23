use cove_macros::{impl_default_for, new_type};
use nid::Nanoid;
use rand::random;

new_type!(WalletId, String, "cove::wallet::metadata::WalletId");
impl_default_for!(WalletId);

impl WalletId {
    pub fn new() -> Self {
        let nanoid: Nanoid = Nanoid::new();
        Self(nanoid.to_string())
    }

    pub fn preview_new() -> Self {
        Self("testtesttest".to_string())
    }

    pub fn preview_new_random() -> Self {
        let random_id = format!("random{}", random::<u32>());
        Self(random_id)
    }
}

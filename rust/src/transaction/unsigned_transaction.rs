use crate::wallet::{confirm::ConfirmDetails, metadata::WalletId};

use super::{Amount, TxId};

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Object,
)]
pub struct UnsignedTransaction {
    pub wallet_id: WalletId,
    pub tx_id: TxId,
    pub confirm_details: ConfirmDetails,
    pub created_at: u64,
}

#[uniffi::export]
impl UnsignedTransaction {
    pub fn id(&self) -> TxId {
        self.tx_id.clone()
    }

    pub fn label(&self) -> String {
        "Sending".to_string()
    }

    pub fn details(&self) -> ConfirmDetails {
        self.confirm_details.clone()
    }

    pub fn spending_amount(&self) -> Amount {
        self.confirm_details.spending_amount()
    }

    pub fn sending_amount(&self) -> Amount {
        self.confirm_details.sending_amount()
    }
}

// MARK: previews
#[uniffi::export]
impl UnsignedTransaction {
    #[uniffi::constructor]
    pub fn preview_new() -> Self {
        Self {
            wallet_id: WalletId::preview_new(),
            tx_id: TxId::preview_new(),
            confirm_details: ConfirmDetails::preview_new(),
            created_at: jiff::Timestamp::now().as_second() as u64,
        }
    }
}

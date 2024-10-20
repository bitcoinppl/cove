use super::{Wallet, WalletAddressType, WalletError};

#[uniffi::export]
impl Wallet {
    #[uniffi::constructor]
    pub fn new_from_xpub(xpub: String) -> Result<Self, WalletError> {
        Wallet::try_new_persisted_from_xpub(xpub)
    }
}

#[uniffi::export]
fn wallet_address_type_to_string(wallet_address_type: WalletAddressType) -> String {
    let str = match wallet_address_type {
        WalletAddressType::NativeSegwit => "Native Segwit",
        WalletAddressType::WrappedSegwit => "Wrapped Segwit",
        WalletAddressType::Legacy => "Legacy",
    };

    str.to_string()
}

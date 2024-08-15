use bdk_chain::bitcoin::Address as BdkAddress;
use bdk_wallet::AddressInfo as BdkAddressInfo;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    derive_more::Display,
    derive_more::From,
    derive_more::Into,
    uniffi::Object,
)]
pub struct Address(BdkAddress);

#[derive(
    Debug,
    PartialEq,
    Eq,
    derive_more::Display,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    uniffi::Object,
)]
pub struct AddressInfo(BdkAddressInfo);

impl Clone for AddressInfo {
    fn clone(&self) -> Self {
        Self(BdkAddressInfo {
            address: self.0.address.clone(),
            index: self.0.index,
            keychain: self.0.keychain,
        })
    }
}

mod ffi {
    use super::*;

    #[uniffi::export]
    impl Address {
        fn string(&self) -> String {
            self.to_string()
        }
    }

    #[uniffi::export]
    impl AddressInfo {
        fn address(&self) -> Address {
            self.address.clone().into()
        }

        fn index(&self) -> u32 {
            self.index
        }
    }
}

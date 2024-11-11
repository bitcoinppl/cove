use bdk_wallet::bitcoin::Psbt as BdkPsbt;
use derive_more::{AsRef, Deref, From, Into};

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object, From, Deref, AsRef, Into)]
pub struct Psbt(BdkPsbt);

use crate::{
    psbt::Psbt,
    transaction::{Amount, FeeRate},
};

use super::Address;

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, uniffi::Object, serde::Serialize, serde::Deserialize,
)]
pub struct ConfirmDetails {
    pub spending_amount: Amount,
    pub sending_amount: Amount,
    pub fee_total: Amount,
    pub fee_rate: FeeRate,
    pub sending_to: Address,
    pub psbt: Psbt,
}

use crate::transaction::fees::BdkFeeRate;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ConfirmDetailsError {
    #[error("unable to represent PSBT as QR code: {0}")]
    QrCodeCreation(String),
}

type Error = ConfirmDetailsError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl ConfirmDetails {
    pub fn spending_amount(&self) -> Amount {
        self.spending_amount
    }

    pub fn sending_amount(&self) -> Amount {
        self.sending_amount
    }

    pub fn fee_total(&self) -> Amount {
        self.fee_total
    }

    pub fn fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    pub fn sending_to(&self) -> Address {
        self.sending_to.clone()
    }

    pub fn is_equal(&self, rhs: &Self) -> bool {
        self.spending_amount == rhs.spending_amount
            && self.sending_amount == rhs.sending_amount
            && self.fee_total == rhs.fee_total
            && self.fee_rate == rhs.fee_rate
            && self.sending_to == rhs.sending_to
    }

    pub fn psbt_to_hex(&self) -> String {
        self.psbt.serialize_hex()
    }

    pub fn psbt_bytes(&self) -> Vec<u8> {
        self.psbt.serialize()
    }

    pub fn psbt_to_bbqr(&self) -> Result<Vec<String>> {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let data = self.psbt.serialize();

        let split = Split::try_from_data(
            data.as_slice(),
            FileType::Psbt,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 100,
                min_version: Version::V01,
                max_version: Version::V23,
            },
        )
        .map_err(|e| ConfirmDetailsError::QrCodeCreation(e.to_string()))?;

        Ok(split.parts)
    }
}

// MARK: CONFIRM DETAILS PREVIEW
mod ffi_preview {
    use crate::psbt::BdkPsbt;

    use super::*;

    #[uniffi::export]
    impl ConfirmDetails {
        #[uniffi::constructor]
        pub fn preview_new() -> Self {
            Self {
                spending_amount: Amount::from_sat(1_000_000),
                sending_amount: Amount::from_sat(1_000_000 - 658),
                fee_total: Amount::from_sat(658),
                fee_rate: BdkFeeRate::from_sat_per_vb_unchecked(658).into(),
                sending_to: Address::preview_new(),
                psbt: psbt_preview_new(),
            }
        }
    }

    fn psbt_preview_new() -> Psbt {
        let psbt_hex = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";
        let psbt_bytes = hex::decode(psbt_hex).expect("unable to decode psbt hex");

        BdkPsbt::deserialize(&psbt_bytes)
            .expect("unable to deserialize psbt")
            .into()
    }
}

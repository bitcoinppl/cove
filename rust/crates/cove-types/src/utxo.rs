use std::{
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use bdk_wallet::{
    KeychainKind, LocalOutput,
    chain::{ChainPosition, ConfirmationBlockTime},
};
use bitcoin::{address::FromScriptError, params::Params};

use crate::{Network, OutPoint, address::Address, amount::Amount};

#[derive(
    Debug,
    Clone,
    Copy,
    Hash,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    uniffi::Enum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum UtxoType {
    Output,
    Change,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
#[uniffi::export(Eq, Hash)]
pub struct Utxo {
    pub outpoint: Arc<OutPoint>,
    pub label: Option<String>,
    pub datetime: u64,
    pub amount: Arc<Amount>,
    pub address: Arc<Address>,
    pub derivation_index: u32,
    pub block_height: u32,
    pub type_: UtxoType,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct UtxoList {
    pub total: Amount,
    pub utxos: Vec<Utxo>,
}

#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum UtxoError {
    #[error("utxo unconfirmed transaction can't be converted to Utxo")]
    Unconfirmed,

    #[error("address parse error {0}")]
    AddressParseError(#[from] FromScriptError),
}

impl Utxo {
    #[must_use]
    pub fn name(&self) -> &str {
        if let Some(label) = &self.label {
            return label;
        }

        match self.type_ {
            UtxoType::Output => "Receive Address",
            UtxoType::Change => "Change Address",
        }
    }

    /// Creates a `Utxo` from a `LocalOutput`
    ///
    /// # Errors
    /// Returns `UtxoError::Unconfirmed` if the output is not confirmed
    /// Returns `UtxoError::AddressParseError` if the address cannot be parsed
    #[allow(clippy::needless_pass_by_value)] // LocalOutput is not cheap to clone
    pub fn try_from_local(local: LocalOutput, network: Network) -> Result<Self, UtxoError> {
        let confirmed: &ConfirmationBlockTime = match &local.chain_position {
            ChainPosition::Confirmed { anchor: confirmed, .. } => confirmed,
            ChainPosition::Unconfirmed { .. } => {
                return Err(UtxoError::Unconfirmed);
            }
        };

        let network = bitcoin::Network::from(network);
        let address =
            bitcoin::Address::from_script(&local.txout.script_pubkey, Params::from(network))?;

        let datetime = confirmed.confirmation_time;
        let block_height = confirmed.block_id.height;

        let amount = Amount::from(local.txout.value);
        let address = Address::from(address);
        let type_ = UtxoType::from(local.keychain);
        let outpoint = OutPoint::from(local.outpoint);
        let derivation_index = local.derivation_index;

        let utxo = Self {
            label: None,
            datetime,
            outpoint: Arc::new(outpoint),
            amount: Arc::new(amount),
            address: Arc::new(address),
            derivation_index,
            block_height,
            type_,
        };

        Ok(utxo)
    }
}

impl UtxoType {
    #[must_use]
    pub const fn is_change(&self) -> bool {
        matches!(self, Self::Change)
    }

    #[must_use]
    pub const fn is_output(&self) -> bool {
        matches!(self, Self::Output)
    }

    #[must_use]
    pub const fn reverse(self) -> Self {
        match self {
            Self::Output => Self::Change,
            Self::Change => Self::Output,
        }
    }
}

impl UtxoList {
    #[must_use]
    pub fn new(utxos: Vec<Utxo>) -> Self {
        let total: u64 = utxos.iter().map(|utxo| utxo.amount.as_ref().as_sats()).sum();
        let total = Amount::from_sat(total);
        Self { total, utxos }
    }

    #[must_use]
    pub fn outpoints(&self) -> Vec<bitcoin::OutPoint> {
        self.utxos.iter().map(|utxo| utxo.outpoint.as_ref().into()).collect()
    }
}
impl From<Vec<Utxo>> for UtxoList {
    fn from(utxos: Vec<Utxo>) -> Self {
        Self::new(utxos)
    }
}

impl From<KeychainKind> for UtxoType {
    fn from(keychain: KeychainKind) -> Self {
        match keychain {
            KeychainKind::External => Self::Output,
            KeychainKind::Internal => Self::Change,
        }
    }
}

// MARK: FFI
#[uniffi::export]
fn utxo_name(utxo: &Utxo) -> String {
    utxo.name().to_string()
}

#[uniffi::export]
#[allow(clippy::cast_possible_wrap)] // datetime is always valid timestamp
fn utxo_date(utxo: &Utxo) -> String {
    let Ok(timestamp) = jiff::Timestamp::from_second(utxo.datetime as i64) else {
        return String::new();
    };

    timestamp.strftime("%b %d, %Y").to_string()
}

#[uniffi::export]
fn utxo_hash_to_uint(utxo: &Utxo) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    utxo.hash(&mut hasher);
    hasher.finish()
}

#[uniffi::export]
fn utxo_is_equal(lhs: &Utxo, rhs: &Utxo) -> bool {
    lhs == rhs
}

// MARK: FFI PREVIEW
pub mod ffi_preview {
    use super::{Address, Amount, Arc, OutPoint, Utxo, UtxoType};
    use rand::random_range;

    #[must_use]
    pub fn preview_new_utxo_list(output_count: u8, change_count: u8) -> Vec<Utxo> {
        let mut utxos = Vec::with_capacity((output_count + change_count) as usize);

        for _ in 0..output_count {
            utxos.push(Utxo::preview_new_output());
        }

        for _ in 0..change_count {
            utxos.push(Utxo::preview_new_change());
        }

        utxos
    }

    impl Utxo {
        fn preview_new_output() -> Self {
            Self::preview_new(UtxoType::Output)
        }

        fn preview_new_change() -> Self {
            Self::preview_new(UtxoType::Change)
        }

        #[allow(clippy::cast_sign_loss)] // timestamp is always positive
        fn preview_new(type_: UtxoType) -> Self {
            let outpoint = OutPoint::_ffi_preview_new();

            let random_sats = random_range(10_100..=10_000_000);
            let amount = Amount::from_sat(random_sats).into();

            let now = jiff::Timestamp::now().as_second().cast_unsigned();
            let random_timestamp = random_range(1_684_242_780..=now);

            let block_height = random_range(0..=900_000);

            Self {
                outpoint: Arc::new(outpoint),
                label: None,
                datetime: random_timestamp,
                amount,
                address: Address::random().into(),
                derivation_index: 0,
                block_height,
                type_,
            }
        }
    }
}

#[uniffi::export(name = "previewNewUtxoList")]
#[must_use]
pub fn _ffi_preview_new_utxo_list(output_count: u8, change_count: u8) -> Vec<Utxo> {
    ffi_preview::preview_new_utxo_list(output_count, change_count)
}

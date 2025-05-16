use std::sync::Arc;

use bdk_wallet::{
    KeychainKind, LocalOutput,
    chain::{ChainPosition, ConfirmationBlockTime},
};
use bitcoin::{address::FromScriptError, params::Params};

use crate::{Network, OutPoint, address::Address, amount::Amount};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum)]
pub enum UtxoType {
    Output,
    Change,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct Utxo {
    pub id: String,
    pub outpoint: Arc<OutPoint>,
    pub label: Option<String>,
    pub datetime: u64,
    pub amount: Arc<Amount>,
    pub address: Arc<Address>,
    pub derivation_index: u32,
    pub block_height: u32,
    pub type_: UtxoType,
}

#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum UtxoError {
    #[error("utxo unconfirmed transaction can't be converted to Utxo")]
    Unconfirmed,

    #[error("address parse error {0}")]
    AddressParseError(#[from] FromScriptError),
}

impl Utxo {
    pub fn try_from_local(local: LocalOutput, network: Network) -> Result<Self, UtxoError> {
        let confirmed: &ConfirmationBlockTime = match &local.chain_position {
            ChainPosition::Confirmed { anchor: confirmed, .. } => confirmed,
            ChainPosition::Unconfirmed { last_seen: _ } => {
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

        let utxo = Utxo {
            id: outpoint.to_string(),
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

impl From<KeychainKind> for UtxoType {
    fn from(keychain: KeychainKind) -> Self {
        match keychain {
            KeychainKind::External => UtxoType::Output,
            KeychainKind::Internal => UtxoType::Change,
        }
    }
}

pub mod ffi {
    use super::*;
    use rand::random_range;

    #[uniffi::export]
    pub fn preview_new_utxo_list(output_count: u8, change_count: u8) -> Vec<Utxo> {
        let mut utxos = Vec::with_capacity((output_count + change_count) as usize);

        for _ in 0..output_count {
            utxos.push(Utxo::preview_new_output())
        }

        for _ in 0..change_count {
            utxos.push(Utxo::preview_new_change())
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

        fn preview_new(type_: UtxoType) -> Self {
            let outpoint = OutPoint::preview_new();

            let random_sats = random_range(10_100..=10_00_000);
            let amount = Amount::from_sat(random_sats).into();

            let now = jiff::Timestamp::now().as_second() as u64;
            let random_timestamp = random_range(1684242780..=now);

            let block_height = random_range(0..=900_000);

            Self {
                id: outpoint.to_string(),
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

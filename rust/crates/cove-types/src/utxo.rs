use bdk_chain::{ChainPosition, ConfirmationBlockTime};
use bdk_wallet::{KeychainKind, LocalOutput};
use bitcoin::{address::FromScriptError, params::Params};

use crate::{Network, OutPoint, address::Address, amount::Amount};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, PartialOrd, Ord, uniffi::Enum)]
pub enum UtxoType {
    Output,
    Change,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct Utxo {
    pub outpoint: OutPoint,
    pub label: Option<String>,
    pub datetime: u64,
    pub amount: Amount,
    pub address: Address,
    pub derivation_index: u32,
    pub block_height: u32,
    pub type_: UtxoType,
}

#[uniffi::export]
impl Utxo {
    pub fn id(&self) -> String {
        self.outpoint.to_string()
    }
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
        let type_ = local.keychain.into();
        let outpoint = local.outpoint.into();
        let derivation_index = local.derivation_index;

        let utxo = Utxo {
            label: None,
            datetime,
            outpoint,
            amount,
            address,
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

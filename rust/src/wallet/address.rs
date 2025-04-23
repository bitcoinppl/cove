use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use bdk_chain::ConfirmationBlockTime;
use bdk_chain::bitcoin::Address as BdkAddress;
use bdk_chain::bitcoin::address::NetworkUnchecked;
use bdk_chain::bitcoin::params::Params;
use bdk_chain::tx_graph::CanonicalTx;
use bdk_wallet::{AddressInfo as BdkAddressInfo, bitcoin::Transaction as BdkTransaction};
use serde::Deserialize;

use crate::transaction::Amount;
use crate::transaction::TransactionDirection;
use cove_types::Network;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    derive_more::Display,
    derive_more::From,
    derive_more::Deref,
    derive_more::AsRef,
    derive_more::Into,
    uniffi::Object,
    serde::Serialize,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct AddressWithNetwork {
    pub address: Address,
    pub network: Network,
    pub amount: Option<Amount>,
}

type Error = AddressError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum AddressError {
    #[error("no ouputs")]
    NoOutputs,

    #[error("unable to create address from script: {0}")]
    ScriptError(String),

    #[error("invalid not a valid address for any network")]
    InvalidAddress,

    #[error("valid address, but for an unsupported network")]
    UnsupportedNetwork,

    #[error("address for wrong network, current network is {current}")]
    WrongNetwork { current: Network },

    #[error("empty address")]
    EmptyAddress,
}

impl Clone for AddressInfo {
    fn clone(&self) -> Self {
        Self(BdkAddressInfo {
            address: self.0.address.clone(),
            index: self.0.index,
            keychain: self.0.keychain,
        })
    }
}

impl Hash for AddressInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.address.hash(state);
        self.0.index.hash(state);
        self.0.keychain.hash(state);
    }
}

impl Address {
    pub fn new(address: BdkAddress) -> Self {
        Self(address)
    }

    pub fn try_new(
        tx: &CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
        wallet: &bdk_wallet::Wallet,
    ) -> Result<Self, Error> {
        let txid = tx.tx_node.txid;
        let network = wallet.network();
        let direction: TransactionDirection = wallet.sent_and_received(&tx.tx_node.tx).into();
        let tx_details = wallet.get_tx(txid).expect("transaction").tx_node.tx;

        let output = match direction {
            TransactionDirection::Incoming => tx_details
                .output
                .iter()
                .find(|output| wallet.is_mine(output.script_pubkey.clone()))
                .ok_or(AddressError::NoOutputs)?,

            TransactionDirection::Outgoing => {
                tx_details.output.first().ok_or(AddressError::NoOutputs)?
            }
        };

        let script = output.script_pubkey.clone().into_boxed_script();

        let address = BdkAddress::from_script(&script, Params::from(network))
            .map_err(|e| Error::ScriptError(e.to_string()))?;

        Ok(Self::new(address))
    }

    pub fn into_unchecked(self) -> BdkAddress<NetworkUnchecked> {
        self.0.into_unchecked()
    }
}

impl AddressWithNetwork {
    pub fn try_new(str: &str) -> Result<Self, Error> {
        let str = str.trim();
        let str = str.trim_start_matches("bitcoin:");

        let (address_str, amount) = extract_amount(str);

        let address: BdkAddress<NetworkUnchecked> =
            address_str.parse().map_err(|_| Error::InvalidAddress)?;

        let network = Network::Bitcoin;
        if let Ok(address) = address.clone().require_network(network.into()) {
            return Ok(Self {
                address: address.into(),
                network,
                amount,
            });
        }

        let network = Network::Testnet;
        if let Ok(address) = address.clone().require_network(network.into()) {
            return Ok(Self {
                address: address.into(),
                network,
                amount,
            });
        }

        let network = Network::Signet;
        if let Ok(address) = address.require_network(network.into()) {
            return Ok(Self {
                address: address.into(),
                network,
                amount,
            });
        }

        Err(Error::UnsupportedNetwork)
    }
}

fn extract_amount(full_qr: &str) -> (&str, Option<Amount>) {
    let Some(pos) = full_qr.find("?amount=") else { return (full_qr, None) };

    let address = &full_qr[..pos];

    let number_start = pos + 8;
    let mut number_end = number_start;
    for char in full_qr[number_start..].chars() {
        if char.is_ascii_digit() || char == '.' {
            number_end += 1;
        } else {
            break;
        }
    }

    let number = &full_qr[number_start..number_end];
    let Ok(amount_float) = number.parse::<f64>() else { return (address, None) };
    let Ok(amount) = Amount::from_btc(amount_float) else { return (address, None) };

    (address, Some(amount))
}

use std::str::FromStr as _;

use bdk_chain::bitcoin::address::NetworkChecked;

use crate::database::Database;

#[uniffi::export]
fn address_is_equal(lhs: Arc<Address>, rhs: Arc<Address>) -> bool {
    lhs == rhs
}

#[uniffi::export]
impl AddressWithNetwork {
    #[uniffi::constructor(name = "new")]
    pub fn new(address: String) -> Result<Self, Error> {
        Self::try_new(&address)
    }

    fn address(&self) -> Address {
        self.address.clone()
    }

    fn network(&self) -> Network {
        self.network
    }

    fn amount(&self) -> Option<Arc<Amount>> {
        self.amount.map(Arc::new)
    }
}

#[uniffi::export]
impl Address {
    #[uniffi::constructor]
    pub fn from_string(address: String) -> Result<Self> {
        let network = Database::global().global_config.selected_network();
        let bdk_address = BdkAddress::from_str(&address)
            .map_err(|_| Error::InvalidAddress)?
            .require_network(network.into())
            .map_err(|_| Error::WrongNetwork { current: network })?;

        Ok(Self(bdk_address))
    }

    #[uniffi::constructor(name = "preview_new")]
    pub fn preview_new() -> Self {
        let address = BdkAddress::from_str("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").unwrap();

        let address: BdkAddress<NetworkChecked> =
            address.require_network(Network::Bitcoin.into()).unwrap();

        Self::new(address)
    }

    fn string(&self) -> String {
        self.to_string()
    }
}

#[uniffi::export]
impl AddressInfo {
    fn address_unformatted(&self) -> String {
        self.address.to_string()
    }

    fn address(&self) -> Address {
        self.address.clone().into()
    }

    fn index(&self) -> u32 {
        self.index
    }
}

#[uniffi::export]
fn address_is_valid(address: String) -> Result<(), Error> {
    let network = Database::global().global_config.selected_network();
    address_is_valid_for_network(address, network)
}

#[uniffi::export]
fn address_is_valid_for_network(address: String, network: Network) -> Result<(), Error> {
    let address = address.trim();
    let address = BdkAddress::from_str(address).map_err(|_| Error::InvalidAddress)?;

    address
        .require_network(network.into())
        .map_err(|_| Error::WrongNetwork { current: network })?;

    Ok(())
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bdk_address = BdkAddress::from_str(&s)
            .map_err(serde::de::Error::custom)?
            .assume_checked();

        Ok(Address(bdk_address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_amount_no_amount() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f";
        let (a, amount) = extract_amount(a);
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, None);
    }

    #[test]
    fn test_extract_amount_with_amount() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.001";
        let (a, amount) = extract_amount(a);
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.001).unwrap()));
    }

    #[test]
    fn test_extract_amount_with_amount_and_spaces() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.001  ";
        let (a, amount) = extract_amount(a);
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.001).unwrap()));
    }

    #[test]
    fn test_extract_amount_with_amount_and_other_query_params() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.002&foo=bar";
        let (a, amount) = extract_amount(a);
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.002).unwrap()));
    }

    #[test]
    fn test_address_with_network() {
        let assert = |address_with_network: AddressWithNetwork, amount: Option<Amount>| {
            assert_eq!(
                address_with_network.address.to_string(),
                "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm"
            );
            assert_eq!(address_with_network.network, Network::Bitcoin);
            assert_eq!(address_with_network.amount, amount);
        };

        let address_with_network =
            AddressWithNetwork::try_new("bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm");

        assert!(address_with_network.is_ok());
        assert(address_with_network.unwrap(), None);

        let address_with_network =
            AddressWithNetwork::try_new("bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?amount=0.001");

        assert!(address_with_network.is_ok());
        assert(
            address_with_network.unwrap(),
            Some(Amount::from_btc(0.001).unwrap()),
        );

        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin:bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?amount=0.001",
        );

        assert!(address_with_network.is_ok());
        assert(
            address_with_network.unwrap(),
            Some(Amount::from_btc(0.001).unwrap()),
        );

        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin:bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?amount=0.002&foo=bar",
        );

        assert!(address_with_network.is_ok());
        assert(
            address_with_network.unwrap(),
            Some(Amount::from_btc(0.002).unwrap()),
        );
    }
}

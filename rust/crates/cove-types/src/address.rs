use std::hash::Hash;
use std::str::FromStr as _;
use std::{hash::Hasher, sync::Arc};

use bdk_wallet::{
    AddressInfo as BdkAddressInfo,
    chain::{ConfirmationBlockTime, bitcoin::Address as BdkAddress, tx_graph::CanonicalTx},
};
use bitcoin::NetworkKind;
use bitcoin::bip32::DerivationPath;
use bitcoin::{
    Transaction,
    address::{NetworkChecked, NetworkUnchecked},
    params::Params,
};
use serde::Deserialize;
use strum::IntoEnumIterator as _;
use url::Url;

use crate::{Network, amount::Amount, transaction::TransactionDirection};

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
pub struct AddressInfoWithDerivation {
    pub info: AddressInfo,
    pub derivation_path: Option<String>,
}

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
    WrongNetwork { current: Network, valid_for: Network },

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
        tx: &CanonicalTx<Arc<Transaction>, ConfirmationBlockTime>,
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
        let (address_str, amount) = parse_bitcoin_uri(str)?;

        let address: BdkAddress<NetworkUnchecked> =
            address_str.parse().map_err(|_| Error::InvalidAddress)?;

        let network = Network::Bitcoin;
        if let Ok(address) = address.clone().require_network(network.into()) {
            return Ok(Self { address: address.into(), network, amount });
        }

        let network = Network::Testnet;
        if let Ok(address) = address.clone().require_network(network.into()) {
            return Ok(Self { address: address.into(), network, amount });
        }

        let network = Network::Signet;
        if let Ok(address) = address.require_network(network.into()) {
            return Ok(Self { address: address.into(), network, amount });
        }

        Err(Error::UnsupportedNetwork)
    }

    pub fn is_valid_for_network(&self, network: Network) -> bool {
        let current_network_type = NetworkKind::from(self.network);
        let network_type = NetworkKind::from(network);
        current_network_type == network_type
    }
}

fn parse_bitcoin_uri(input: &str) -> Result<(String, Option<Amount>), Error> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::EmptyAddress);
    }

    let has_scheme = input
        .split_once(':')
        .map(|(scheme, _)| scheme.eq_ignore_ascii_case("bitcoin"))
        .unwrap_or(false);

    let normalized = if has_scheme { input.to_string() } else { format!("bitcoin:{input}") };

    let url = Url::parse(&normalized).map_err(|_| Error::InvalidAddress)?;

    if !url.scheme().eq_ignore_ascii_case("bitcoin") {
        return Err(Error::InvalidAddress);
    }

    let mut address = String::new();

    if let Some(host) = url.host_str() {
        address.push_str(host);
    }

    let path = url.path();
    if !path.is_empty() && path != "/" {
        let trimmed_path = path.trim_start_matches('/');
        if !trimmed_path.is_empty() {
            if !address.is_empty() && !address.ends_with('/') {
                address.push('/');
            }
            address.push_str(trimmed_path);
        }
    }

    if address.is_empty() {
        return Err(Error::EmptyAddress);
    }

    let amount = url.query_pairs().find(|(key, _)| key == "amount").and_then(|(_, value)| {
        let value = value.trim();
        value.parse::<f64>().ok().and_then(|btc| Amount::from_btc(btc).ok())
    });

    Ok((address, amount))
}

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

    #[uniffi::method(name = "isValidForNetwork")]
    pub fn ffi_is_valid_for_network(&self, network: Network) -> bool {
        self.is_valid_for_network(network)
    }
}

#[uniffi::export]
impl Address {
    #[uniffi::constructor]
    pub fn from_string(address: &str, network: Network) -> Result<Self> {
        let address = address.trim();
        let bdk_address = BdkAddress::from_str(address).map_err(|_| Error::InvalidAddress)?;

        let network_to_check = network.into();
        if bdk_address.is_valid_for_network(network_to_check) {
            return Ok(Self(bdk_address.require_network(network_to_check).expect("just checked")));
        }

        for network in Network::iter() {
            if bdk_address.is_valid_for_network(network.into()) {
                return Err(Error::WrongNetwork {
                    current: network_to_check.into(),
                    valid_for: network,
                });
            }
        }

        Err(Error::UnsupportedNetwork)
    }

    #[uniffi::constructor(name = "preview_new")]
    pub fn preview_new() -> Self {
        let address = BdkAddress::from_str("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").unwrap();

        let address: BdkAddress<NetworkChecked> =
            address.require_network(Network::Bitcoin.into()).unwrap();

        Self::new(address)
    }

    #[uniffi::method]
    pub fn spaced_out(&self) -> String {
        address_string_spaced_out(self.to_string())
    }

    fn string(&self) -> String {
        self.to_string()
    }

    #[uniffi::method]
    fn unformatted(&self) -> String {
        self.to_string()
    }

    #[uniffi::method(name = "toString")]
    fn ffi_to_string(&self) -> String {
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
impl AddressInfoWithDerivation {
    fn address_unformatted(&self) -> String {
        self.info.address.to_string()
    }

    fn address_spaced_out(&self) -> String {
        address_string_spaced_out(self.info.address.to_string())
    }

    fn address(&self) -> Address {
        self.info.address.clone().into()
    }

    fn index(&self) -> u32 {
        self.info.index
    }

    fn derivation_path(&self) -> Option<String> {
        self.derivation_path.clone()
    }
}

impl AddressInfoWithDerivation {
    pub fn new(info: AddressInfo, derivation_path_prefix: Option<DerivationPath>) -> Self {
        let derivation_path = derivation_path_prefix.map(|p| format!("{p}/0/{}", info.index));
        Self { info, derivation_path }
    }
}

#[uniffi::export]
fn address_string_spaced_out(address: String) -> String {
    let groups = address.len() / 5;
    let mut final_address = String::with_capacity(address.len() + groups);

    for (i, char) in address.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            final_address.push(' ');
        }

        final_address.push(char)
    }

    final_address
}

#[uniffi::export]
fn address_is_valid(address: String, network: Network) -> Result<(), Error> {
    address_is_valid_for_network(address, network)
}

#[uniffi::export]
fn address_is_valid_for_network(address: String, network: Network) -> Result<(), Error> {
    Address::from_string(&address, network)?;
    Ok(())
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bdk_address =
            BdkAddress::from_str(&s).map_err(serde::de::Error::custom)?.assume_checked();

        Ok(Address(bdk_address))
    }
}

pub mod ffi {
    use super::*;

    use rand::seq::IndexedRandom;

    fn random_address() -> &'static str {
        const ADDRESSES: [&str; 10] = [
            "tb1q2z9f42gfafthstgn34es2eamr2afv474sdsld8",
            "tb1p6vhsxjsszp63gedr8ywq8qx00wnkqx3pmuxatffh8za62v5uy0xqk92z4y",
            "tb1q6exja52re3dykawlwqfca4kv0tg0y7crpnttvt",
            "tb1psq467xgaqwda3nexzshg8llzhyrw5k5053k3jzv769rlyldtp73q2wcqzk",
            "tb1pc5ul0lzjl6nwxewmrmay5ppcmn3w2dzxw349t0unnjnemq53tv9qg0xgfy",
            "tb1pqcffycem084xfr5kql5ypqeqr9uknzls08xat7f8p9ag3f8tmk7sz66vl2",
            "tb1pvam2nqaw9hsw0nzahuhdak4l8v0h5dwsaca6264ns7ppk8unrk9qvjrtxn",
            "tb1qum986qkqf363jhaau2nlavyehdqg487p8m2ydu",
            "tb1qmt3vg7e4krvy77sevcdtlaxh9qasen0dhrx63s",
            "tb1qhajp86w02393277e9wp4u2puqfs6gl6mpthyez",
        ];
        let mut rng = rand::rng();
        ADDRESSES.choose(&mut rng).unwrap()
    }

    #[uniffi::export]
    impl Address {
        #[uniffi::constructor]
        pub fn random() -> Self {
            Self::new(BdkAddress::from_str(random_address()).unwrap().assume_checked())
        }

        #[uniffi::method(name = "hashToUint")]
        fn ffi_hash(&self) -> u64 {
            let mut hasher = std::hash::DefaultHasher::new();
            self.hash(&mut hasher);
            hasher.finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bitcoin_uri_no_amount() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f";
        let (a, amount) = parse_bitcoin_uri(a).unwrap();
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, None);
    }

    #[test]
    fn test_parse_bitcoin_uri_with_amount() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.001";
        let (a, amount) = parse_bitcoin_uri(a).unwrap();
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.001).unwrap()));
    }

    #[test]
    fn test_parse_bitcoin_uri_with_amount_and_spaces() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.001  ";
        let (a, amount) = parse_bitcoin_uri(a).unwrap();
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.001).unwrap()));
    }

    #[test]
    fn test_parse_bitcoin_uri_with_amount_and_other_query_params() {
        let a = "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?amount=0.002&foo=bar";
        let (a, amount) = parse_bitcoin_uri(a).unwrap();
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, Some(Amount::from_btc(0.002).unwrap()));
    }

    #[test]
    fn test_parse_bitcoin_uri_with_scheme_and_unused_params() {
        let a = "bitcoin://bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f?label=Donation&foo=bar";
        let (a, amount) = parse_bitcoin_uri(a).unwrap();
        assert_eq!(a, "bc1q0g0vn4yqyk0zjwxw0zv5pltyy9jm89vclxgsv3f");
        assert_eq!(amount, None);
    }

    #[test]
    fn test_address_string_spaced_out() {
        let address = "bc1pkdj04w4lxsv570j5nsd249lqe4w4j608r2nq9997ruh0wv96cnksy5jeny";
        let expected = "bc1pk dj04w 4lxsv 570j5 nsd24 9lqe4 w4j60 8r2nq 9997r uh0wv 96cnk sy5je ny";
        assert_eq!(address_string_spaced_out(address.to_string()), expected);
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
        assert(address_with_network.unwrap(), Some(Amount::from_btc(0.001).unwrap()));

        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin:bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?amount=0.001",
        );

        assert!(address_with_network.is_ok());
        assert(address_with_network.unwrap(), Some(Amount::from_btc(0.001).unwrap()));

        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin:bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?amount=0.002&foo=bar",
        );

        assert!(address_with_network.is_ok());
        assert(address_with_network.unwrap(), Some(Amount::from_btc(0.002).unwrap()));
    }

    #[test]
    fn test_address_with_network_label_query_only() {
        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin:bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?label=Donation%20For%20Cove",
        );

        assert!(address_with_network.is_ok());
        let address_with_network = address_with_network.unwrap();
        assert_eq!(
            address_with_network.address.to_string(),
            "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm"
        );
        assert_eq!(address_with_network.network, Network::Bitcoin);
        assert_eq!(address_with_network.amount, None);

        let address_with_network = AddressWithNetwork::try_new(
            "bitcoin://bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm?label=Donation",
        )
        .unwrap();

        assert_eq!(
            address_with_network.address.to_string(),
            "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm"
        );
        assert_eq!(address_with_network.network, Network::Bitcoin);
        assert_eq!(address_with_network.amount, None);
    }
}

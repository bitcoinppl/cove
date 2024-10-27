use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use bdk_chain::bitcoin::address::NetworkUnchecked;
use bdk_chain::bitcoin::params::Params;
use bdk_chain::bitcoin::Address as BdkAddress;
use bdk_chain::tx_graph::CanonicalTx;
use bdk_chain::ConfirmationBlockTime;
use bdk_wallet::{bitcoin::Transaction as BdkTransaction, AddressInfo as BdkAddressInfo};

use crate::network::Network;
use crate::transaction::TransactionDirection;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct AddressWithNetwork {
    pub address: Address,
    pub network: Network,
}

type Error = AddressError;

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
}

impl AddressWithNetwork {
    pub fn try_new(str: &str) -> Result<Self, Error> {
        let address: BdkAddress<NetworkUnchecked> =
            str.parse().map_err(|_| Error::InvalidAddress)?;

        let network = Network::Bitcoin;
        if let Ok(address) = address.clone().require_network(network.into()) {
            return Ok(Self {
                address: address.into(),
                network,
            });
        }

        let network = Network::Testnet;
        if let Ok(address) = address.require_network(network.into()) {
            return Ok(Self {
                address: address.into(),
                network,
            });
        }

        Err(Error::UnsupportedNetwork)
    }
}

mod ffi {
    use std::str::FromStr as _;

    use bdk_chain::bitcoin::address::NetworkChecked;

    use super::*;

    #[uniffi::export]
    impl AddressWithNetwork {
        fn address(&self) -> Address {
            self.address.clone()
        }

        fn network(&self) -> Network {
            self.network
        }
    }

    #[uniffi::export]
    impl Address {
        #[uniffi::constructor(name = "preview_new")]
        pub fn preview_new() -> Self {
            let address = BdkAddress::from_str(
                "bc1p0000304alk4tg3vxcu7l9m4xf4cvauzml5608cssvz5f60jwg68q83lyn9",
            )
            .unwrap();

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
        fn adress_string(&self) -> String {
            self.address.to_string()
        }

        fn address(&self) -> Address {
            self.address.clone().into()
        }

        fn index(&self) -> u32 {
            self.index
        }
    }
}

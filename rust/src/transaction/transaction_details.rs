use std::sync::Arc;

use bdk_chain::{tx_graph::CanonicalTx, ChainPosition as BdkChainPosition, ConfirmationBlockTime};
use bdk_wallet::bitcoin::Transaction as BdkTransaction;
use bdk_wallet::Wallet as BdkWallet;

use crate::wallet::{address, Address};

use super::{Amount, FeeRate, SentAndReceived, TxId};

#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TransactionDetailError {
    #[error("Unable to determine fee: {0}")]
    FeeError(String),

    #[error("Unable to determine fee rate: {0}")]
    FeeRateError(String),

    #[error("Unable to determine address: {0}")]
    AddressError(#[from] address::AddressError),
}

type Error = TransactionDetailError;
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct TransactionDetails {
    pub tx_id: TxId,
    pub address: Address,
    pub sent_and_received: SentAndReceived,
    pub fee: Amount,
    pub fee_rate: FeeRate,
    pub pending_or_confirmed: PendingOrConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PendingOrConfirmed {
    Pending(PendingDetails),
    Confirmed(ConfirmedDetails),
}

impl TransactionDetails {
    pub fn try_new(
        wallet: &BdkWallet,
        tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
    ) -> Result<Self, Error> {
        let txid = tx.tx_node.txid;
        let sent_and_received = wallet.sent_and_received(&tx.tx_node.tx).into();
        let chain_postition = &tx.chain_position;
        let tx_details = wallet.get_tx(txid).expect("transaction").tx_node.tx;

        let fee = wallet
            .calculate_fee(&tx_details)
            .map_err(|e| Error::FeeError(e.to_string()))?
            .into();

        let fee_rate = wallet
            .calculate_fee_rate(&tx_details)
            .map_err(|e| Error::FeeRateError(e.to_string()))?
            .into();

        let address = Address::try_new(&tx_details, wallet.network().into())?;
        let pending_or_confirmed = PendingOrConfirmed::new(chain_postition);

        let me = Self {
            tx_id: txid.into(),
            address,
            sent_and_received,
            fee,
            pending_or_confirmed,
            fee_rate,
        };

        Ok(me)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PendingDetails {
    last_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConfirmedDetails {
    block_number: u32,
}

impl PendingOrConfirmed {
    pub fn new(chain_position: &BdkChainPosition<&ConfirmationBlockTime>) -> Self {
        match chain_position {
            BdkChainPosition::Unconfirmed(last_seen) => Self::Pending(PendingDetails {
                last_seen: *last_seen,
            }),
            BdkChainPosition::Confirmed(confirmation_blocktime) => {
                Self::Confirmed(ConfirmedDetails {
                    block_number: confirmation_blocktime.block_id.height,
                })
            }
        }
    }

    fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed(_))
    }
}

mod ffi {
    use crate::transaction::{TransactionDirection, Unit};

    use super::*;

    #[uniffi::export]
    impl TransactionDetails {
        #[uniffi::method]
        pub fn address(&self) -> Address {
            self.address.clone()
        }

        #[uniffi::method]
        pub fn amount(&self) -> Amount {
            self.sent_and_received.amount()
        }

        #[uniffi::method]
        pub fn fee(&self) -> Amount {
            self.fee
        }

        #[uniffi::method]
        pub fn amount_fmt(&self, unit: Unit) -> String {
            self.sent_and_received.amount_fmt(unit)
        }

        #[uniffi::method]
        pub fn is_received(&self) -> bool {
            self.sent_and_received.direction() == TransactionDirection::Incoming
        }

        #[uniffi::method]
        pub fn is_sent(&self) -> bool {
            !self.is_received()
        }

        #[uniffi::method]
        pub fn number_of_confirmations(&self) -> u32 {
            todo!()
        }

        #[uniffi::method]
        pub fn is_confirmed(&self) -> bool {
            self.pending_or_confirmed.is_confirmed()
        }
    }
}

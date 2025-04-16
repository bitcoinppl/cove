use std::sync::Arc;

use bdk_chain::{ChainPosition as BdkChainPosition, ConfirmationBlockTime, tx_graph::CanonicalTx};
use bdk_wallet::Wallet as BdkWallet;
use bdk_wallet::bitcoin::Transaction as BdkTransaction;
use bip329::{Label, Labels, TransactionRecord};
use bitcoin::params::Params;
use jiff::Timestamp;
use numfmt::{Formatter, Precision};

use crate::{
    database::Database,
    fiat::{FiatCurrency, client::FIAT_CLIENT},
    format::NumberFormatter as _,
    task,
    transaction::{TransactionDirection, Unit},
};

use crate::{
    device::Device,
    wallet::{Address, address},
};

use super::{Amount, FeeRate, SentAndReceived, TxId};

#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TransactionDetailError {
    #[error("Unable to determine fee: {0}")]
    Fee(String),

    #[error("Unable to determine fee rate: {0}")]
    FeeRate(String),

    #[error("Unable to determine address: {0}")]
    Address(#[from] address::AddressError),

    #[error("Unable to get fiat amount: {0}")]
    FiatAmount(String),

    #[error("Unable to get change address: {0}")]
    ChangeAddress(String),
}

type Error = TransactionDetailError;
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TransactionDetails {
    pub tx_id: TxId,
    pub address: Address,
    pub sent_and_received: SentAndReceived,
    pub fee: Option<Amount>,
    pub fee_rate: Option<FeeRate>,
    pub pending_or_confirmed: PendingOrConfirmed,
    pub labels: Labels,
    pub input_indexes: Vec<u32>,
    pub output_indexes: Vec<u32>,
    // for outgoing transactions we might have a change address
    pub change_address: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PendingOrConfirmed {
    Pending(PendingDetails),
    Confirmed(ConfirmedDetails),
}

impl TransactionDetails {
    pub fn try_new(
        wallet: &BdkWallet,
        tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
        labels: Labels,
    ) -> Result<Self, Error> {
        let txid = tx.tx_node.txid;
        let sent_and_received: SentAndReceived = wallet.sent_and_received(&tx.tx_node.tx).into();
        let chain_postition = &tx.chain_position;
        let tx_details = wallet.get_tx(txid).expect("transaction").tx_node.tx;

        let fee = wallet.calculate_fee(&tx_details).ok().map(Into::into);
        let fee_rate = wallet.calculate_fee_rate(&tx_details).ok().map(Into::into);

        let address = Address::try_new(&tx, wallet)?;
        let pending_or_confirmed = PendingOrConfirmed::new(chain_postition);

        let change_address = match sent_and_received.direction {
            TransactionDirection::Incoming => None,
            TransactionDirection::Outgoing => {
                let outputs = tx.tx_node.tx.output.iter();
                let script = outputs.map(|o| o.script_pubkey.clone()).find_map(|pubkey| {
                    if wallet.is_mine(pubkey.clone()) {
                        Some(pubkey.into_boxed_script())
                    } else {
                        None
                    }
                });

                match script {
                    Some(script) => {
                        let network = wallet.network();
                        let address = bitcoin::Address::from_script(&script, Params::from(network))
                            .map_err(|e| Error::ChangeAddress(e.to_string()))?;

                        Some(address)
                    }
                    None => None,
                }
            }
        }
        .map(Address::new);

        let input_indexes = tx
            .tx_node
            .tx
            .input
            .iter()
            .filter(|input| wallet.is_mine(input.script_sig.clone()))
            .map(|input| input.previous_output.vout)
            .collect();

        let output_indexes = tx
            .tx_node
            .tx
            .output
            .iter()
            .enumerate()
            .filter(|(_index, output)| wallet.is_mine(output.script_pubkey.clone()))
            .map(|(index, _output)| index as u32)
            .collect();

        let me = Self {
            tx_id: txid.into(),
            address,
            sent_and_received,
            fee,
            pending_or_confirmed,
            fee_rate,
            labels,
            input_indexes,
            output_indexes,
            change_address,
        };

        Ok(me)
    }

    pub fn sent_sans_fee(&self) -> Option<Amount> {
        if self.is_received() {
            return None;
        }

        let fee: Amount = self.fee?;
        let sent: Amount = self.amount();

        let sans_fee = sent.checked_sub(fee.0)?;

        Some(sans_fee.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct PendingDetails {
    last_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ConfirmedDetails {
    block_number: u32,
    confirmation_time: u64,
}

impl PendingOrConfirmed {
    pub fn new(chain_position: &BdkChainPosition<ConfirmationBlockTime>) -> Self {
        match chain_position {
            BdkChainPosition::Unconfirmed { last_seen } => Self::Pending(PendingDetails {
                last_seen: (*last_seen).unwrap_or_default(),
            }),
            BdkChainPosition::Confirmed {
                anchor: confirmation_blocktime,
                ..
            } => Self::Confirmed(ConfirmedDetails {
                block_number: confirmation_blocktime.block_id.height,
                confirmation_time: confirmation_blocktime.confirmation_time,
            }),
        }
    }

    fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed(_))
    }
}

#[uniffi::export]
impl TransactionDetails {
    #[uniffi::method]
    pub fn tx_id(&self) -> TxId {
        self.tx_id
    }

    #[uniffi::method]
    pub fn address(&self) -> Address {
        self.address.clone()
    }

    #[uniffi::method]
    pub fn amount(&self) -> Amount {
        self.sent_and_received.amount()
    }

    #[uniffi::method]
    pub async fn amount_fiat(&self) -> Result<f64, Error> {
        let amount = self.amount();

        task::spawn(async move {
            FIAT_CLIENT
                .current_value_in_currency(amount, currency())
                .await
                .map_err(|e| Error::FiatAmount(e.to_string()))
        })
        .await
        .unwrap()
    }

    #[uniffi::method]
    pub async fn amount_fiat_fmt(&self) -> Result<String, Error> {
        let amount = self.amount_fiat().await?;
        Ok(fiat_amount_fmt(amount))
    }

    #[uniffi::method]
    pub fn fee_fmt(&self, unit: Unit) -> Option<String> {
        let fee = self.fee?;
        Some(fee.fmt_string_with_unit(unit))
    }

    #[uniffi::method]
    pub async fn fee_fiat_fmt(&self) -> Result<String, Error> {
        let fee = self.fee.ok_or(Error::Fee("No fee".to_string()))?;
        let fiat = task::spawn(async move {
            FIAT_CLIENT
                .current_value_in_currency(fee, currency())
                .await
                .map_err(|e| Error::FiatAmount(e.to_string()))
        })
        .await
        .unwrap()?;

        Ok(fiat_amount_fmt(fiat))
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
    pub fn sent_sans_fee_fmt(&self, unit: Unit) -> Option<String> {
        let amount = self.sent_sans_fee()?;
        Some(amount.fmt_string_with_unit(unit))
    }

    #[uniffi::method]
    pub async fn sent_sans_fee_fiat_fmt(&self) -> Result<String, Error> {
        let amount = self
            .sent_sans_fee()
            .ok_or(Error::Fee("No fee".to_string()))?;

        let fiat = task::spawn(async move {
            FIAT_CLIENT
                .current_value_in_currency(amount, currency())
                .await
                .map_err(|e| Error::FiatAmount(e.to_string()))
        })
        .await
        .unwrap()?;

        Ok(fiat_amount_fmt(fiat))
    }

    #[uniffi::method]
    pub fn is_confirmed(&self) -> bool {
        self.pending_or_confirmed.is_confirmed()
    }

    #[uniffi::method]
    pub fn confirmation_date_time(&self) -> Option<String> {
        let confirm_time = match &self.pending_or_confirmed {
            PendingOrConfirmed::Pending(_) => None,
            PendingOrConfirmed::Confirmed(confirmed) => Some(confirmed.confirmation_time),
        }? as i64;

        // get timezone
        let timezone_string = Device::global().timezone();
        // let timezone = Tz::from_str(&timezone_string).ok()?;

        // Create a Timestamp from Unix seconds
        let ts = Timestamp::from_second(confirm_time).ok()?;

        // Convert to local time zone
        let local = match ts.in_tz(&timezone_string) {
            Ok(local) => local,
            Err(error) => {
                tracing::warn!("unable to convert timestamp: {error}");
                ts.in_tz("UTC").ok()?
            }
        };

        // Format the timestamp
        jiff::fmt::strtime::format("%B %e, %Y at %-I:%M %p", &local).ok()
    }

    #[uniffi::method]
    pub fn transaction_url(&self) -> String {
        format!("https://mempool.guide/tx/{}", self.tx_id.0)
    }

    #[uniffi::method]
    pub fn transaction_label(&self) -> Option<String> {
        let label = self.labels.transaction_label()?;
        if label.is_empty() {
            return None;
        }

        Some(label.to_string())
    }

    #[uniffi::method]
    pub fn block_number(&self) -> Option<u32> {
        match &self.pending_or_confirmed {
            PendingOrConfirmed::Pending(_) => None,
            PendingOrConfirmed::Confirmed(confirmed) => Some(confirmed.block_number),
        }
    }

    #[uniffi::method]
    pub fn block_number_fmt(&self) -> Option<String> {
        let block_number = self.block_number()?;

        let mut f = Formatter::new()
            .separator(',')
            .unwrap()
            .precision(Precision::Decimals(0));

        Some(f.fmt(block_number).to_string())
    }
    #[uniffi::method]
    pub fn address_spaced_out(&self) -> String {
        self.address.spaced_out()
    }
}

#[uniffi::export]
impl TransactionDetails {
    #[uniffi::constructor]
    pub fn preview_new_confirmed() -> Self {
        Self {
            tx_id: TxId::preview_new(),
            address: Address::preview_new(),
            sent_and_received: SentAndReceived::preview_new(),
            fee: Some(Amount::from_sat(880303)),
            fee_rate: Some(FeeRate::preview_new()),
            pending_or_confirmed: PendingOrConfirmed::Confirmed(ConfirmedDetails {
                block_number: 840_000,
                confirmation_time: 1677721600,
            }),
            labels: Default::default(),
            input_indexes: vec![],
            output_indexes: vec![],
            change_address: None,
        }
    }

    #[uniffi::constructor]
    pub fn preview_confirmed_received() -> Self {
        let mut me = Self::preview_new_confirmed();
        me.sent_and_received = SentAndReceived::preview_incoming();
        me
    }

    #[uniffi::constructor]
    pub fn preview_confirmed_sent() -> Self {
        let mut me = Self::preview_new_confirmed();
        me.sent_and_received = SentAndReceived::preview_outgoing();
        me
    }

    #[uniffi::constructor]
    pub fn preview_pending_received() -> Self {
        let mut me = Self::preview_new_confirmed();
        me.sent_and_received = SentAndReceived::preview_incoming();
        me.pending_or_confirmed = PendingOrConfirmed::Pending(PendingDetails {
            last_seen: 1677721600,
        });

        me
    }

    #[uniffi::constructor]
    pub fn preview_pending_sent() -> Self {
        let mut me = Self::preview_new_confirmed();
        me.sent_and_received = SentAndReceived::preview_outgoing();
        me.pending_or_confirmed = PendingOrConfirmed::Pending(PendingDetails {
            last_seen: 1677721600,
        });

        me
    }

    #[uniffi::constructor(default(label = "bike payment"))]
    pub fn preview_new_with_label(label: String) -> Self {
        let mut me = Self::preview_new_confirmed();
        me.labels = vec![Label::from(TransactionRecord {
            ref_: *TxId::preview_new(),
            label: Some(label),
            origin: None,
        })]
        .into();

        me
    }
}

/// MARK: local helpers
fn currency() -> FiatCurrency {
    Database::global()
        .global_config
        .fiat_currency()
        .unwrap_or_default()
}

fn fiat_amount_fmt(amount: f64) -> String {
    let amount_fmt = amount.thousands_fiat();

    let currency = currency();
    let symbol = currency.symbol();
    let suffix = currency.suffix();

    format!("≈ {symbol}{amount_fmt} {suffix}")
}

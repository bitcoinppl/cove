use serde::Serialize;

use crate::{
    device::Device,
    fiat::FiatCurrency,
    transaction::{ConfirmedTransaction, TransactionDirection, TxId},
};

pub struct HistoricalFiatPriceReport {
    currency: FiatCurrency,
    txns: Vec<(ConfirmedTransaction, Option<f32>)>,
    timezone: String,
}

#[derive(Debug)]
pub struct Csv(Vec<u8>);

#[derive(Debug, thiserror::Error)]
pub enum CsvCreationError {
    #[error("failed to fianlize csv: {0}")]
    FinalizeCsv(String),

    #[error("failed to write csv row: {0}")]
    WriteCsvRow(#[from] csv::Error),
}

impl Csv {
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn into_string(self) -> String {
        String::from_utf8(self.into_bytes()).expect("we only create rows with valid uft8 strings")
    }
}
type Row = TxnWithHistoricalPrice;

#[derive(Debug, Serialize)]
pub struct TxnWithHistoricalPrice {
    pub tx_id: TxId,
    pub date_time: String,
    pub block_height: u32,
    pub label: Option<String>,
    pub btc_amount: f64,
    pub sats_amount: i64,
    pub fiat_price: Option<f32>,
    pub txn_direction: &'static str,
}

impl HistoricalFiatPriceReport {
    pub fn new(currency: FiatCurrency, txns: Vec<(ConfirmedTransaction, Option<f32>)>) -> Self {
        Self {
            currency,
            txns,
            timezone: Device::global().timezone(),
        }
    }

    pub fn create_csv(self) -> Result<Csv, CsvCreationError> {
        let fiat_header = format!(
            "Amount ({}{})",
            self.currency.symbol(),
            self.currency.suffix()
        );

        let mut csv = csv::Writer::from_writer(vec![]);

        // write header
        csv.write_record([
            "Transaction ID",
            "Confirmed At",
            "Block Height",
            "Label",
            "Amont (BTC)",
            "Amount (Sats)",
            &fiat_header,
            "Transaction Direction",
        ])?;

        let rows = self.txns.iter().map(|txn| self.create_row(txn));

        // write each row
        for row in rows {
            let row = row?;
            csv.serialize(row)?;
        }

        let csv = csv
            .into_inner()
            .map_err(|e| CsvCreationError::FinalizeCsv(e.to_string()))?;

        Ok(Csv(csv))
    }

    fn create_row(&self, txn: &(ConfirmedTransaction, Option<f32>)) -> Result<Row, csv::Error> {
        let (txn, fiat_price) = txn;
        let fiat_price = *fiat_price;

        // convert to local time zone
        let datetime_local = match txn.confirmed_at.in_tz(&self.timezone) {
            Ok(local) => local,
            Err(error) => {
                tracing::warn!("unable to convert timestamp: {error}");
                txn.confirmed_at
                    .in_tz("UTC")
                    .expect("all timestamps after unix epoch")
            }
        };

        let sent_and_received = txn.sent_and_received;
        let txn_direction = sent_and_received.direction();

        let amount = sent_and_received.amount();
        let btc_amount = amount.as_btc();
        let sats_amount = amount.as_sats() as i64;

        let direction_multiplier = match txn_direction {
            TransactionDirection::Incoming => 1.0,
            TransactionDirection::Outgoing => -1.0,
        };

        let txn_direction = match txn_direction {
            TransactionDirection::Incoming => "Received",
            TransactionDirection::Outgoing => "Sent",
        };

        let row = Row {
            tx_id: txn.id(),
            date_time: jiff::fmt::rfc2822::to_string(&datetime_local)
                .expect("all datetimes are valid"),
            block_height: txn.block_height(),
            label: txn.label_opt(),
            btc_amount,
            sats_amount,
            fiat_price: fiat_price.map(|fiat_price| fiat_price * direction_multiplier),
            txn_direction,
        };

        Ok(row)
    }
}

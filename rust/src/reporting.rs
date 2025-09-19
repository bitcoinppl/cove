use csv::WriterBuilder;
use serde::Serialize;

use crate::{
    device::Device,
    fiat::FiatCurrency,
    transaction::{ConfirmedTransaction, TransactionDirection, TxId},
};
use cove_util::result_ext::ResultExt as _;

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
    pub date_time_utc: String,
    pub date_time_local: String,
    pub block_height: u32,
    pub label: Option<String>,
    pub btc_amount: f64,
    pub sats_amount: i64,
    pub fiat_price: Option<String>,
    pub txn_direction: &'static str,
}

impl HistoricalFiatPriceReport {
    pub fn new(currency: FiatCurrency, txns: Vec<(ConfirmedTransaction, Option<f32>)>) -> Self {
        Self { currency, txns, timezone: Device::global().timezone() }
    }

    pub fn create_csv(self) -> Result<Csv, CsvCreationError> {
        let fiat_header = format!("Amount ({}{})", self.currency.symbol(), self.currency.suffix());

        let confirmed_at_local_header = format!("Confirmed At ({})", self.timezone);

        let mut csv = WriterBuilder::new().has_headers(false).from_writer(vec![]);

        // write header
        csv.write_record([
            "Transaction ID",
            "Confirmed At (UTC)",
            confirmed_at_local_header.as_str(),
            "Block Height",
            "Label",
            "Amount (BTC)",
            "Amount (Sats)",
            &fiat_header,
            "Transaction Direction",
        ])?;

        let rows = self.txns.iter().map(|txn| self.create_row(txn));

        // write each row
        // skip the header row because we wrote a custom one
        for row in rows {
            let row = row?;
            csv.serialize(row)?;
        }

        let csv = csv.into_inner().map_err_str(CsvCreationError::FinalizeCsv)?;

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
                txn.confirmed_at.in_tz("UTC").expect("all timestamps after unix epoch")
            }
        };

        let datetime_local_string = datetime_local.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string();

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

        let fiat_price = fiat_price
            .map(|fiat_price| fiat_price as f64 * btc_amount)
            .map(|fiat_price| fiat_price * direction_multiplier)
            .map(|fiat_price| {
                let rounded = (fiat_price * 100.0).round() / 100.0;
                format!("{rounded:.2}")
            });

        let row = Row {
            tx_id: txn.id(),
            date_time_utc: txn.confirmed_at.to_string(),
            date_time_local: datetime_local_string,
            block_height: txn.block_height(),
            label: txn.label_opt(),
            btc_amount,
            sats_amount,
            fiat_price,
            txn_direction,
        };

        Ok(row)
    }
}

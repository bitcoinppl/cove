use crate::{
    database::{Database, historical_price::HistoricalPriceTable},
    fiat::{FiatCurrency, client::FIAT_CLIENT},
    transaction::ConfirmedTransaction,
};

pub struct HistoricalPrice {
    db: HistoricalPriceTable,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to get historical price for transaction: {0}")]
    GetHistoricalPrice(#[from] reqwest::Error),

    #[error("empty historical prices for block {block_number} at timestamp {timestamp}")]
    EmptyHistoricalPrices { block_number: u32, timestamp: u64 },
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl HistoricalPrice {
    pub fn new() -> Self {
        let db = Database::global();

        Self {
            db: db.historical_prices(),
        }
    }

    pub async fn get_price_for_transaction(
        &self,
        txn: &ConfirmedTransaction,
        currency: FiatCurrency,
    ) -> Result<Option<f32>> {
        let block_number = txn.block_height();
        let confirmed_at = txn.confirmed_at();

        // we have a record for this block number
        if let Ok(Some(price)) = self.db.get_price_for_block(block_number) {
            return Ok(price.for_currency(currency));
        }

        // don't have a record for this block number lets try to get it
        let historical_prices_response = FIAT_CLIENT.historical_prices(confirmed_at).await?;
        let price = historical_prices_response.prices.first().ok_or_else(|| {
            Error::EmptyHistoricalPrices {
                block_number,
                timestamp: confirmed_at,
            }
        })?;

        if let Err(error) = self.db.set_price_for_block(block_number, *price) {
            tracing::error!(
                "unable to save (database error) historical price for block {block_number} at timestamp {confirmed_at}: {error}"
            );
        }

        Ok(price.for_currency(currency))
    }
}

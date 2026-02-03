use crate::{
    database::{Database, error::DatabaseError, historical_price::HistoricalPriceTable},
    fiat::{FiatCurrency, client::FIAT_CLIENT, historical::HistoricalPrice},
    network::Network,
    transaction::ConfirmedTransaction,
};
use futures::stream::{self, StreamExt};

pub struct HistoricalPriceService {
    db: HistoricalPriceTable,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to get historical price for transaction: {0}")]
    GetHistoricalPrice(#[from] reqwest::Error),

    #[error("empty historical prices for block {block_number} at timestamp {timestamp}")]
    EmptyHistoricalPrices { block_number: u32, timestamp: u64 },

    #[error("failed to get historical price from the database: {0}")]
    Database(#[from] DatabaseError),
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl HistoricalPriceService {
    pub fn new() -> Self {
        let db = Database::global();

        Self { db: db.historical_prices() }
    }

    pub async fn get_prices_for_transactions(
        &self,
        network: Network,
        currency: FiatCurrency,
        txns: Vec<ConfirmedTransaction>,
    ) -> Result<Vec<(ConfirmedTransaction, Option<f32>)>> {
        use ahash::AHashMap as HashMap;

        type BlockHeight = u32;
        type Timestamp = u64;

        let block_number_timestamp: HashMap<BlockHeight, Timestamp> =
            txns.iter().map(|txn| (txn.block_height(), txn.confirmed_at())).collect();

        let block_heights = {
            let mut block_heights = txns.iter().map(|txn| txn.block_height()).collect::<Vec<u32>>();

            block_heights.sort_unstable();
            block_heights.dedup();

            block_heights
        };

        let db_prices: HashMap<BlockHeight, Option<HistoricalPrice>> =
            match self.db.get_prices_for_blocks(network, &block_heights) {
                Ok(prices) => prices,
                Err(error) => {
                    tracing::error!("failed to get historical prices from the database: {error}");
                    block_heights.iter().map(|block_height| (*block_height, None)).collect()
                }
            };

        // if we don't have a price for a block, we need to fetch it
        let blocks_to_fetch = db_prices
            .iter()
            .filter(|(_, price)| price.is_none())
            .map(|(block_number, _)| *block_number)
            .collect::<Vec<_>>();

        let fetched_prices: HashMap<u32, HistoricalPrice> =
            stream::iter(blocks_to_fetch.into_iter())
                .map(|block_number| {
                    let timestamp = *block_number_timestamp
                        .get(&block_number)
                        .expect("bug in creating block_number_timestamp");

                    async move {
                        match self
                            .get_and_save_price_for_timestamp(network, block_number, timestamp)
                            .await
                        {
                            Ok(price) => Some((block_number, price)),
                            Err(_) => None,
                        }
                    }
                })
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .flatten()
                .collect();

        let txns_with_prices = txns
            .iter()
            .map(|txn| {
                let txn = txn.clone();
                let block_height = txn.block_height();

                // get the price from the database or from the fetched prices
                let price_opt = db_prices
                    .get(&block_height)
                    .and_then(Option::as_ref)
                    .or_else(|| fetched_prices.get(&block_height));

                let fiat_price = price_opt.and_then(|price| price.for_currency(currency));
                (txn, fiat_price)
            })
            .collect();

        Ok(txns_with_prices)
    }

    #[allow(dead_code)]
    pub async fn get_price_for_transaction(
        &self,
        network: Network,
        txn: &ConfirmedTransaction,
        currency: FiatCurrency,
    ) -> Result<Option<f32>> {
        let block_number = txn.block_height();

        // we have a record for this block number
        if let Ok(Some(price)) = self.db.get_price_for_block(network, block_number) {
            return Ok(price.for_currency(currency));
        }

        // don't have a record for this block number lets try to get it
        let price = self
            .get_and_save_price_for_timestamp(network, block_number, txn.confirmed_at())
            .await?;

        Ok(price.for_currency(currency))
    }

    /// Get historical price for a block, fetching from API if not cached
    pub async fn get_price_for_block(
        &self,
        network: Network,
        block_number: u32,
        timestamp: u64,
        currency: FiatCurrency,
    ) -> Result<Option<f32>> {
        // check cache first
        if let Ok(Some(price)) = self.db.get_price_for_block(network, block_number) {
            return Ok(HistoricalPrice::from(price).for_currency(currency));
        }

        // fetch and cache
        let price = self.get_and_save_price_for_timestamp(network, block_number, timestamp).await?;

        Ok(price.for_currency(currency))
    }

    async fn get_and_save_price_for_timestamp(
        &self,
        network: Network,
        block_number: u32,
        timestamp: u64,
    ) -> Result<HistoricalPrice, Error> {
        let historical_prices_response = FIAT_CLIENT.historical_prices(timestamp).await?;
        let price = historical_prices_response
            .prices
            .first()
            .ok_or_else(|| Error::EmptyHistoricalPrices { block_number, timestamp })?;

        if let Err(error) = self.db.set_price_for_block(network, block_number, *price) {
            tracing::error!(
                "unable to save (database error) historical price for block {block_number} at timestamp {timestamp}: {error}"
            );
        }

        Ok(*price)
    }
}

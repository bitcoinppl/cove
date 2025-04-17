use std::cmp::Ordering;

use crate::fiat::{FiatCurrency, historical::HistoricalPrice};

use super::BlockNumber;

/// A space-efficient version of HistoricalPrice where only USD is required
/// and other currencies are optional to save space when they aren't available
#[derive(Debug, Copy, Clone, PartialEq, uniffi::Record)]
pub struct HistoricalPriceRecord {
    pub time: u64,
    pub usd: f32,
    pub eur: Option<f32>,
    pub gbp: Option<f32>,
    pub cad: Option<f32>,
    pub chf: Option<f32>,
    pub aud: Option<f32>,
    pub jpy: Option<f32>,
}

impl From<HistoricalPrice> for HistoricalPriceRecord {
    fn from(price: HistoricalPrice) -> Self {
        Self {
            time: price.time,
            usd: price.usd,
            eur: positive_or_none(price.eur),
            gbp: positive_or_none(price.gbp),
            cad: positive_or_none(price.cad),
            chf: positive_or_none(price.chf),
            aud: positive_or_none(price.aud),
            jpy: positive_or_none(price.jpy),
        }
    }
}

impl From<HistoricalPriceRecord> for HistoricalPrice {
    fn from(record: HistoricalPriceRecord) -> Self {
        Self {
            time: record.time,
            usd: record.usd,
            eur: record.eur.unwrap_or(-1.0),
            gbp: record.gbp.unwrap_or(-1.0),
            cad: record.cad.unwrap_or(-1.0),
            chf: record.chf.unwrap_or(-1.0),
            aud: record.aud.unwrap_or(-1.0),
            jpy: record.jpy.unwrap_or(-1.0),
        }
    }
}

fn positive_or_none(value: f32) -> Option<f32> {
    if value >= 0.0 { Some(value) } else { None }
}

impl HistoricalPriceRecord {
    /// Get the price for a specific currency
    pub fn for_currency(&self, currency: FiatCurrency) -> Option<f32> {
        match currency {
            FiatCurrency::Usd => Some(self.usd),
            FiatCurrency::Eur => self.eur,
            FiatCurrency::Gbp => self.gbp,
            FiatCurrency::Cad => self.cad,
            FiatCurrency::Chf => self.chf,
            FiatCurrency::Aud => self.aud,
            FiatCurrency::Jpy => self.jpy,
        }
    }
}

impl redb::Key for BlockNumber {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for BlockNumber {
    type SelfType<'a> = BlockNumber;
    type AsBytes<'a> = [u8; 4];

    fn fixed_width() -> Option<usize> {
        Some(4)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let block_number = u32::from_le_bytes(data.try_into().unwrap());
        Self(block_number)
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.0.to_le_bytes()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new(std::any::type_name::<BlockNumber>())
    }
}

impl redb::Value for HistoricalPriceRecord {
    type SelfType<'a>
        = &'a HistoricalPriceRecord
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        todo!()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        // value.0.as_ref()
        todo!()
    }

    fn type_name() -> redb::TypeName {
        todo!()
    }
}

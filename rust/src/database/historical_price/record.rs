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

/// Error type for HistoricalPriceRecord
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum HistoricalPriceRecordError {
    #[error("failed to convert bytes to HistoricalPriceRecord: {0:?}")]
    ConversionError(#[from] ByteReaderError),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum ByteReaderError {
    #[error("buffer too small")]
    BufferTooSmall,
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct CurrencyFlag: u8 {
        const EUR = 1 << 0;
        const GBP = 1 << 1;
        const CAD = 1 << 2;
        const CHF = 1 << 3;
        const AUD = 1 << 4;
        const JPY = 1 << 5;
    }
}

pub type Error = HistoricalPriceRecordError;

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

    /// Convert from bytes
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        // at minimum we need 1+8+4 = 13 bytes
        if bytes.len() < 13 {
            return Err(ByteReaderError::BufferTooSmall.into());
        }

        let mut r = ByteReader::new(bytes);
        let flag = CurrencyFlag::from_bits_truncate(r.read_u8()?);

        let time = r.read_u64_le()?;
        let usd = r.read_f32_le()?;

        // helper: if the bit is set, read 4 bytes as f32 and advance idx
        let mut get_opt = |bit: CurrencyFlag| -> Result<Option<f32>, ByteReaderError> {
            if flag.contains(bit) {
                let v = r.read_f32_le()?;
                Ok(Some(v))
            } else {
                Ok(None)
            }
        };

        let eur = get_opt(CurrencyFlag::EUR)?;
        let gbp = get_opt(CurrencyFlag::GBP)?;
        let cad = get_opt(CurrencyFlag::CAD)?;
        let chf = get_opt(CurrencyFlag::CHF)?;
        let aud = get_opt(CurrencyFlag::AUD)?;
        let jpy = get_opt(CurrencyFlag::JPY)?;

        Ok(HistoricalPriceRecord {
            time,
            usd,
            eur,
            gbp,
            cad,
            chf,
            aud,
            jpy,
        })
    }

    /// Convert to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let flag = CurrencyFlag::from(*self).bits();
        let mut bytes = Vec::with_capacity(8 + 4 + (6 * 4));

        bytes.push(flag);
        bytes.extend_from_slice(&self.time.to_le_bytes());
        bytes.extend_from_slice(&self.usd.to_le_bytes());

        if let Some(eur) = self.eur {
            bytes.extend_from_slice(&eur.to_le_bytes());
        }

        if let Some(gbp) = self.gbp {
            bytes.extend_from_slice(&gbp.to_le_bytes());
        }

        if let Some(cad) = self.cad {
            bytes.extend_from_slice(&cad.to_le_bytes());
        }

        if let Some(chf) = self.chf {
            bytes.extend_from_slice(&chf.to_le_bytes());
        }

        if let Some(aud) = self.aud {
            bytes.extend_from_slice(&aud.to_le_bytes());
        }

        if let Some(jpy) = self.jpy {
            bytes.extend_from_slice(&jpy.to_le_bytes());
        }

        bytes.shrink_to_fit();
        bytes
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], ByteReaderError> {
        if self.bytes.len() < self.index + N {
            return Err(ByteReaderError::BufferTooSmall);
        }

        let out = self.bytes[self.index..self.index + N]
            .try_into()
            .map_err(|_| ByteReaderError::BufferTooSmall)?;

        self.index += N;
        Ok(out)
    }

    fn read_u8(&mut self) -> Result<u8, ByteReaderError> {
        Ok(self.read_array::<1>()?[0])
    }

    fn read_u64_le(&mut self) -> Result<u64, ByteReaderError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_f32_le(&mut self) -> Result<f32, ByteReaderError> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }
}

impl From<HistoricalPriceRecord> for CurrencyFlag {
    fn from(record: HistoricalPriceRecord) -> Self {
        let mut f = Self::empty();
        f.set(Self::EUR, record.eur.is_some());
        f.set(Self::GBP, record.gbp.is_some());
        f.set(Self::CAD, record.cad.is_some());
        f.set(Self::CHF, record.chf.is_some());
        f.set(Self::AUD, record.aud.is_some());
        f.set(Self::JPY, record.jpy.is_some());
        f
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
    type SelfType<'a> = HistoricalPriceRecord;
    type AsBytes<'a> = Vec<u8>;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        Self::try_from_bytes(data).unwrap()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.to_bytes()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new(std::any::type_name::<HistoricalPriceRecord>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(
        eur: bool,
        gbp: bool,
        cad: bool,
        chf: bool,
        aud: bool,
        jpy: bool,
    ) -> HistoricalPriceRecord {
        HistoricalPriceRecord {
            time: 0,
            usd: 1.0,
            eur: if eur { Some(1.1) } else { None },
            gbp: if gbp { Some(1.2) } else { None },
            cad: if cad { Some(1.3) } else { None },
            chf: if chf { Some(1.4) } else { None },
            aud: if aud { Some(1.5) } else { None },
            jpy: if jpy { Some(1.6) } else { None },
        }
    }

    fn random_record(rng: &mut impl rand::Rng) -> HistoricalPriceRecord {
        HistoricalPriceRecord {
            time: 1745268220,
            usd: rng.random_range(0.0..500_000.0),
            eur: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
            gbp: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
            cad: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
            chf: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
            aud: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
            jpy: if rng.random_bool(0.5) {
                Some(rng.random())
            } else {
                None
            },
        }
    }

    #[test]
    fn no_optional_currencies() {
        let rec = make_record(false, false, false, false, false, false);
        assert_eq!(CurrencyFlag::from(rec), CurrencyFlag::empty());
    }

    #[test]
    fn single_currency_present() {
        let rec = make_record(true, false, false, false, false, false);
        assert_eq!(CurrencyFlag::from(rec), CurrencyFlag::EUR);
    }

    #[test]
    fn multiple_currencies_present() {
        let rec = make_record(true, true, false, true, false, true);
        let expected =
            CurrencyFlag::EUR | CurrencyFlag::GBP | CurrencyFlag::CHF | CurrencyFlag::JPY;
        assert_eq!(CurrencyFlag::from(rec), expected);
    }

    #[test]
    fn all_currencies_present() {
        let rec = make_record(true, true, true, true, true, true);
        let expected = CurrencyFlag::all();
        assert_eq!(CurrencyFlag::from(rec), expected);
    }

    #[test]
    fn round_trip_flag_bits() {
        let rec = make_record(true, false, true, false, true, false);
        let flag = CurrencyFlag::from(rec);
        let raw: u8 = flag.bits();
        let parsed = CurrencyFlag::from_bits_truncate(raw);
        assert_eq!(parsed, flag);
    }

    #[test]
    fn round_trip_records() {
        let record = HistoricalPriceRecord {
            time: 1745268220,
            usd: 1.0,
            eur: Some(1.1),
            gbp: Some(1.2),
            cad: None,
            chf: Some(1.4),
            aud: Some(1.5),
            jpy: Some(1.6),
        };

        let bytes = record.to_bytes();
        let parsed = HistoricalPriceRecord::try_from_bytes(&bytes).expect("roundtrip failed");
        assert_eq!(record, parsed);

        let record = HistoricalPriceRecord {
            time: 1745268220,
            usd: 1.0,
            eur: Some(1.1),
            gbp: Some(1.2),
            cad: Some(47.0),
            chf: Some(1.4),
            aud: Some(1.5),
            jpy: None,
        };

        let bytes = record.to_bytes();
        let parsed = HistoricalPriceRecord::try_from_bytes(&bytes).expect("roundtrip failed");
        assert_eq!(record, parsed);

        let record = HistoricalPriceRecord {
            time: 1745268220,
            usd: 1.0,
            eur: None,
            gbp: Some(1.2),
            cad: Some(47.0),
            chf: Some(1.4),
            aud: Some(1.5),
            jpy: None,
        };

        let bytes = record.to_bytes();
        let parsed = HistoricalPriceRecord::try_from_bytes(&bytes).expect("roundtrip failed");
        assert_eq!(record, parsed);
    }

    #[test]
    fn round_trip_random_records() {
        let mut rng = rand::rng();

        for _ in 0..100 {
            let rec = random_record(&mut rng);
            let bytes = rec.to_bytes();
            let parsed = HistoricalPriceRecord::try_from_bytes(&bytes).expect("roundtrip failed");

            assert_eq!(rec.time, parsed.time, "time mismatch");
            assert!((rec.usd - parsed.usd).abs() < f32::EPSILON, "usd mismatch");

            macro_rules! assert_opt_f32 {
                ($a:expr, $b:expr, $name:literal) => {
                    match ($a, $b) {
                        (Some(x), Some(y)) => {
                            assert!((x - y).abs() < f32::EPSILON, "{} mismatch", $name)
                        }
                        (None, None) => {}
                        _ => panic!("{} presence mismatch", $name),
                    }
                };
            }

            assert_opt_f32!(rec.eur, parsed.eur, "eur");
            assert_opt_f32!(rec.gbp, parsed.gbp, "gbp");
            assert_opt_f32!(rec.cad, parsed.cad, "cad");
            assert_opt_f32!(rec.chf, parsed.chf, "chf");
            assert_opt_f32!(rec.aud, parsed.aud, "aud");
            assert_opt_f32!(rec.jpy, parsed.jpy, "jpy");
        }
    }
}

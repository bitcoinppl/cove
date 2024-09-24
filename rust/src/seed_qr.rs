use bitvec::{field::BitField as _, order::Msb0, vec::BitVec};

#[derive(Debug, Clone, uniffi::Object)]
pub enum SeedQr {
    Standard(Vec<u16>),
    Compact(Vec<u16>),
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SeedQrError {
    #[error("Not a standard seed QR, contains non numeric chars")]
    ContainsNonNumericChars,

    #[error("Index out of bounds: {0}, max is: 2047")]
    IndexOutOfBounds(u16),

    #[error("Incorrect word length, got: {0}, expected: 12,15,18,21 or 24")]
    IncorrectWordLength(u16),
}

type Error = SeedQrError;

impl SeedQr {
    pub fn try_from_str(qr: &str) -> Result<Self, Error> {
        let word_indexes = parse_str_into_word_indexes(qr)?;
        Ok(Self::Standard(word_indexes))
    }

    pub fn try_from_data(data: Vec<u8>) -> Result<Self, Error> {
        let word_indexes = parse_data_into_word_indexes(data)?;
        Ok(Self::Compact(word_indexes))
    }
}

fn parse_data_into_word_indexes(mut data: Vec<u8>) -> Result<Vec<u16>, SeedQrError> {
    let checksum = calculate_checksum(&data);

    data.push(checksum);
    let bits = BitVec::<u8, Msb0>::from_vec(data);

    let indexes: Vec<u16> = bits
        .chunks(11)
        .filter(|chunk| chunk.len() == 11)
        .map(|chunk| chunk.load_be::<u16>())
        .collect();

    match indexes.len() {
        12 | 15 | 18 | 21 | 24 => Ok(indexes),
        other => Err(SeedQrError::IncorrectWordLength(other as u16)),
    }
}

fn calculate_checksum(entropy: &[u8]) -> u8 {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(entropy);
    let hash = hasher.finalize();

    let checksum_bits = entropy.len() * 8 / 32;
    BitVec::<u8, Msb0>::from_slice(&hash[..])
        .into_iter()
        .take(checksum_bits)
        .collect::<BitVec<u8, Msb0>>()
        .load_be::<u8>()
}

fn parse_str_into_word_indexes(qr: &str) -> Result<Vec<u16>, SeedQrError> {
    if !qr.chars().all(|c| c.is_numeric()) {
        return Err(SeedQrError::ContainsNonNumericChars);
    }

    let max_index = qr.len();
    let mut indexes: Vec<u16> = Vec::with_capacity((qr.len() / 4) + 1);
    let mut current_starting_index = 0;

    let end_index = |starting_index: usize| -> usize {
        let index = starting_index + 4;
        if index > max_index {
            max_index
        } else {
            index
        }
    };

    while current_starting_index < max_index {
        let starting_index = current_starting_index;
        let ending_index = end_index(starting_index);

        let word_index = u16::from_str_radix(&qr[starting_index..ending_index], 10)
            .expect("already checked all numeric");

        if word_index > 2047 {
            return Err(SeedQrError::IndexOutOfBounds(word_index));
        }

        indexes.push(word_index);

        current_starting_index = ending_index;
    }

    match indexes.len() {
        12 | 15 | 18 | 21 | 24 => Ok(indexes),
        other => Err(SeedQrError::IncorrectWordLength(other as u16)),
    }
}

mod ffi {
    use super::*;

    #[uniffi::export]
    impl SeedQr {
        pub fn words(&self) -> Vec<String> {
            let word_indexes = match self {
                SeedQr::Standard(word_idx) => word_idx,
                SeedQr::Compact(word_idx) => word_idx,
            };

            todo!()
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_parse_str_into_word_indexes() {
        let qr = "192402220235174306311124037817700641198012901210";
        let expected = vec![
            1924, 222, 235, 1743, 631, 1124, 378, 1770, 641, 1980, 1290, 1210,
        ];

        assert_eq!(parse_str_into_word_indexes(qr).unwrap(), expected);
    }

    #[test]
    fn test_parse_data_into_word_indexes() {
        let bytes = b"\x0et\xb6A\x07\xf9L\xc0\xcc\xfa\xe6\xa1=\xcb\xec6b\x15O\xecg\xe0\xe0\t\x99\xc0x\x92Y}\x19\n";
        let expected = vec![
            115, 1325, 1154, 127, 1190, 771, 415, 742, 1289, 1906, 2008, 870, 266, 1343, 1420,
            2016, 1792, 614, 896, 1929, 300, 1524, 801, 643,
        ];

        assert_eq!(
            parse_data_into_word_indexes(bytes.to_vec()).unwrap(),
            expected
        );
    }
}

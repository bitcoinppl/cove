use bip39::{Language, Mnemonic};

use crate::mnemonic::WordAccess as _;

#[derive(Debug, Clone, uniffi::Object)]
pub enum SeedQr {
    Standard(Mnemonic),
    Compact(Mnemonic),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SeedQrError {
    #[error("Not a standard seed QR, contains non numeric chars")]
    ContainsNonNumericChars,

    #[error("Index out of bounds: {0}, max is: 2047")]
    IndexOutOfBounds(u16),

    #[error("Incorrect word length, got: {0}, expected: 12,15,18,21 or 24")]
    IncorrectWordLength(u16),

    #[error("unable to parse mnemonic: {0}")]
    InvalidMnemonic(#[from] Bip39Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, thiserror::Error, uniffi::Error)]
pub enum Bip39Error {
    #[error("Mnemonic has a word count that is not a multiple of 6, found {0}.")]
    BadWordCount(u32),

    // Error contains the index of the word.
    // Use `mnemonic.split_whitespace().get(i)` to get the word.
    #[error("Mnemonic contains an unknown word at index {0}.")]
    UnknownWord(u32),

    #[error("Entropy was not a multiple of 32 bits or between 128-256n bits in length.")]
    BadEntropyBitCount(u32),

    #[error("The mnemonic has an invalid checksum.")]
    InvalidChecksum,

    // Use the helper methods of the inner struct to which languages are possible."
    #[error("The mnemonic can be interpreted as multiple languages")]
    AmbiguousLanguages,
}

type Error = SeedQrError;

impl SeedQr {
    pub fn try_from_str(qr: &str) -> Result<Self, Error> {
        let word_list = Language::English.word_list();
        let indexes = parse_str_into_word_indexes(qr)?;
        let words: String =
            indexes.iter().map(|index| word_list[*index as usize]).collect::<Vec<&str>>().join(" ");

        let mnemonic = Mnemonic::parse_in(Language::English, &words)?;
        Ok(Self::Standard(mnemonic))
    }

    pub fn try_from_data(data: &[u8]) -> Result<Self, Error> {
        let mnemonic = Mnemonic::from_entropy(data)?;
        Ok(Self::Compact(mnemonic))
    }

    pub fn mnemonic(&self) -> &Mnemonic {
        match self {
            SeedQr::Standard(mnemonic) => mnemonic,
            SeedQr::Compact(mnemonic) => mnemonic,
        }
    }

    pub fn into_mnemonic(self) -> Mnemonic {
        match self {
            SeedQr::Standard(mnemonic) => mnemonic,
            SeedQr::Compact(mnemonic) => mnemonic,
        }
    }

    fn words(&self) -> impl Iterator<Item = &'static str> + '_ {
        let mnemonic = self.mnemonic();
        mnemonic.words()
    }
}

#[uniffi::export]
impl SeedQr {
    #[uniffi::constructor]
    pub fn new_from_data(data: Vec<u8>) -> Result<Self, Error> {
        Self::try_from_data(&data)
    }

    #[uniffi::constructor]
    pub fn new_from_str(qr: &str) -> Result<Self, Error> {
        Self::try_from_str(qr)
    }

    #[uniffi::method]
    pub fn get_words(&self) -> Vec<String> {
        self.words().map(|word| word.to_string()).collect()
    }

    #[uniffi::method]
    pub fn grouped_plain_words(&self, groups_of: u8) -> Vec<Vec<String>> {
        self.mnemonic().grouped_plain_words_of(groups_of as usize)
    }
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
        if index > max_index { max_index } else { index }
    };

    while current_starting_index < max_index {
        let starting_index = current_starting_index;
        let ending_index = end_index(starting_index);

        let word_index: u16 =
            qr[starting_index..ending_index].parse().expect("already checked all numeric");

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

impl From<bip39::Error> for Bip39Error {
    fn from(error: bip39::Error) -> Self {
        match error {
            bip39::Error::BadWordCount(words) => Self::BadWordCount(words as u32),
            bip39::Error::UnknownWord(index) => Self::UnknownWord(index as u32),
            bip39::Error::BadEntropyBitCount(bits) => Self::BadEntropyBitCount(bits as u32),
            bip39::Error::InvalidChecksum => Self::InvalidChecksum,
            bip39::Error::AmbiguousLanguages(_) => Self::AmbiguousLanguages,
        }
    }
}

impl From<bip39::Error> for SeedQrError {
    fn from(error: bip39::Error) -> Self {
        Self::InvalidMnemonic(error.into())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    struct TestVector {
        words: &'static str,
        bytes: Vec<u8>,
        standard: &'static str,
    }

    #[test]
    fn test_parse_str_into_word_indexes() {
        let qr = "192402220235174306311124037817700641198012901210";
        let expected = vec![1924, 222, 235, 1743, 631, 1124, 378, 1770, 641, 1980, 1290, 1210];

        assert_eq!(parse_str_into_word_indexes(qr).unwrap(), expected);
    }

    #[test]
    fn test_get_words_from_str() {
        let qr = "192402220235174306311124037817700641198012901210";
        let words = vec![
            "vacuum", "bridge", "buddy", "supreme", "exclude", "milk", "consider", "tail",
            "expand", "wasp", "pattern", "nuclear",
        ];

        let seed_qr = SeedQr::try_from_str(qr).unwrap();
        assert_eq!(seed_qr.get_words(), words);
    }

    #[test]
    fn test_parse_data_into_word_indexes_12_words() {
        let bytes: Vec<u8> = vec![
            0b01011011, 0b10111101, 0b10011101, 0b01110001, 0b10101000, 0b11101100, 0b01111001,
            0b10010000, 0b10000011, 0b00011010, 0b11111111, 0b00110101, 0b10011101, 0b01000010,
            0b01100101, 0b01000101,
        ];

        let hex = "5bbd9d71a8ec7990831aff359d426545";
        let hex_bytes = hex::decode(hex).unwrap();

        assert_eq!(bytes, hex_bytes);
        let bytes = hex_bytes;

        let seed_qr = SeedQr::try_from_data(&bytes).unwrap();
        let expected = "forum undo fragile fade shy sign arrest garment culture tube off merit"
            .split_whitespace()
            .collect::<Vec<&str>>();

        assert_eq!(seed_qr.get_words(), expected);
    }

    #[test]
    fn test_vectors() {
        let test_vectors = vec![
            TestVector {
                words: "attack pizza motion avocado network gather crop fresh patrol unusual wild holiday candy pony ranch winter theme error hybrid van cereal salon goddess expire",
                standard: "011513251154012711900771041507421289190620080870026613431420201617920614089619290300152408010643",
                bytes: vec![
                    0x0e, 0x74, 0xb6, 0x41, 0x07, 0xf9, 0x4c, 0xc0, 0xcc, 0xfa, 0xe6, 0xa1, 0x3d,
                    0xcb, 0xec, 0x36, 0x62, 0x15, 0x4f, 0xec, 0x67, 0xe0, 0xe0, 0x09, 0x99, 0xc0,
                    0x78, 0x92, 0x59, 0x7d, 0x19, 0x0a,
                ],
            },
            TestVector {
                words: "atom solve joy ugly ankle message setup typical bean era cactus various odor refuse element afraid meadow quick medal plate wisdom swap noble shallow",
                standard: "011416550964188800731119157218870156061002561932122514430573003611011405110613292018175411971576",
                bytes: vec![
                    0x0e, 0x59, 0xdd, 0xe2, 0x76, 0x00, 0x93, 0x17, 0xf1, 0x27, 0x5f, 0x13, 0x89,
                    0x88, 0x80, 0x78, 0xc9, 0x93, 0x68, 0xd1, 0xe8, 0x24, 0x89, 0xb5, 0xf6, 0x29,
                    0x53, 0x1f, 0xc5, 0xb6, 0xa5, 0x6e,
                ],
            },
            TestVector {
                words: "sound federal bonus bleak light raise false engage round stock update render quote truck quality fringe palace foot recipe labor glow tortoise potato still",
                standard: "166206750203018810361417065805941507171219081456140818651401074412730727143709940798183613501710",
                bytes: vec![
                    0xcf, 0xca, 0x8c, 0x65, 0x8b, 0xc8, 0x19, 0x62, 0x54, 0x92, 0x52, 0xbc, 0x7a,
                    0xc3, 0xba, 0x5b, 0x0b, 0x01, 0xd2, 0x6b, 0xca, 0xe8, 0x9f, 0x2b, 0x5e, 0xce,
                    0xbe, 0x26, 0x3d, 0xcb, 0x2a, 0x36,
                ],
            },
            TestVector {
                words: "forum undo fragile fade shy sign arrest garment culture tube off merit",
                standard: "073318950739065415961602009907670428187212261116",
                bytes: vec![
                    0x5b, 0xbd, 0x9d, 0x71, 0xa8, 0xec, 0x79, 0x90, 0x83, 0x1a, 0xff, 0x35, 0x9d,
                    0x42, 0x65, 0x45,
                ],
            },
            TestVector {
                words: "good battle boil exact add seed angle hurry success glad carbon whisper",
                standard: "080301540200062600251559007008931730078802752004",
                bytes: vec![
                    0x64, 0x62, 0x68, 0x64, 0x27, 0x20, 0x33, 0x85, 0xc2, 0x33, 0x7d, 0xd8, 0x4c,
                    0x50, 0x89, 0xfd,
                ],
            },
            TestVector {
                words: "approve fruit lens brass ring actual stool coin doll boss strong rate",
                standard: "008607501025021714880023171503630517020917211425",
                bytes: vec![
                    0x0a, 0xcb, 0xba, 0x00, 0x8d, 0x9b, 0xa0, 0x05, 0xf5, 0x99, 0x6b, 0x40, 0xa3,
                    0x47, 0x5c, 0xd9,
                ],
            },
            TestVector {
                words: "dignity utility vacant shiver thought canoe feel multiply item youth actor coyote",
                standard: "049619221923158517990268067811630950204300210397",
                bytes: vec![
                    0x3e, 0x1e, 0x0b, 0xc1, 0xe3, 0x1e, 0x0e, 0x43, 0x15, 0x34, 0x8b, 0x76, 0xdf,
                    0xec, 0x0a, 0x98,
                ],
            },
            TestVector {
                words: "vocal tray giggle tool duck letter category pattern train magnet excite swamp",
                standard: "196218530783182905421028028912901848107106301753",
                bytes: vec![
                    0xf5, 0x5c, 0xf5, 0x87, 0xf2, 0x54, 0x3d, 0x01, 0x09, 0x0d, 0x0a, 0xe7, 0x10,
                    0xbd, 0x3b, 0x6d,
                ],
            },
            TestVector {
                words: "corn voice scrap arrow original diamond trial property benefit choose junk lock",
                standard: "038719631547010112530489185713790169032209701051",
                bytes: vec![
                    0x30, 0x7e, 0xaf, 0x05, 0x86, 0x59, 0xca, 0x7a, 0x7a, 0x0d, 0x63, 0x15, 0x25,
                    0x09, 0xe5, 0x41,
                ],
            },
        ];

        for vector in test_vectors {
            let vector_words = vector.words.split_whitespace().collect::<Vec<&str>>();

            let seed_qr = SeedQr::try_from_str(vector.standard);
            assert!(seed_qr.is_ok());
            let seed_qr = seed_qr.unwrap();
            assert_eq!(seed_qr.get_words(), vector_words);

            let seed_qr = SeedQr::try_from_data(&vector.bytes);
            assert!(seed_qr.is_ok());
            let seed_qr = seed_qr.unwrap();
            assert_eq!(seed_qr.get_words(), vector_words);
        }
    }

    #[test]
    fn test_15_word_length() {
        let words = "play element inch believe wrestle because feed sign pool soldier roof loop monitor burst grace".split_whitespace().collect::<Vec<&str>>();
        let bytes = hex::decode("a648f5c90a5fe427952e42a819d2eec1f8f03d99").unwrap();

        let seed_qr = SeedQr::try_from_data(&bytes).unwrap();
        assert_eq!(seed_qr.get_words(), words);
    }

    #[test]
    fn test_18_word_length() {
        let words = "chuckle remind squeeze useful area absorb pretty essence occur orchard knock worry usage fan cradle rifle daring abandon".split_whitespace().collect::<Vec<&str>>();
        let bytes = hex::decode("2896bb4e77f0b401aa8a6a98d381ef7eeef8a58c7dcd3780").unwrap();

        let seed_qr = SeedQr::try_from_data(&bytes).unwrap();
        assert_eq!(seed_qr.get_words(), words);
    }

    #[test]
    fn test_21_word_length() {
        let words = "cinnamon quote sweet lend clown link save world dog air text misery unveil betray attitude goat inspire identify wrap inspire tank".split_whitespace().collect::<Vec<&str>>();
        let bytes =
            hex::decode("2916036ebff2c1042ff7ed4080af7ec6dee62ac3ab20754e13f83aad").unwrap();

        let seed_qr = SeedQr::try_from_data(&bytes).unwrap();
        assert_eq!(seed_qr.get_words(), words);
    }
}

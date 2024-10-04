use std::sync::Arc;

use parking_lot::Mutex;

use crate::{NfcReaderError, ParseResult};

#[derive(Debug, Clone, uniffi::Object)]
pub struct NfcReader(Arc<Mutex<crate::NfcReader>>);

#[uniffi::export]
impl NfcReader {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let reader = crate::NfcReader::new();
        Self(Arc::new(Mutex::new(reader)))
    }

    #[uniffi::method]
    pub fn parse(&self, data: Vec<u8>) -> Result<ParseResult, NfcReaderError> {
        self.0.lock().parse(data)
    }
}

impl Default for NfcReader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NfcConst {
    pub number_of_blocks_per_chunk: u16,
    pub bytes_per_block: u16,
}

#[uniffi::export]
impl NfcConst {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            number_of_blocks_per_chunk: crate::NUMBER_OF_BLOCKS_PER_CHUNK,
            bytes_per_block: crate::BYTES_PER_BLOCK,
        }
    }
}

impl Default for NfcConst {
    fn default() -> Self {
        Self::new()
    }
}

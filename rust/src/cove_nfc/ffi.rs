use std::sync::Arc;

use parking_lot::Mutex;

use crate::cove_nfc::{NfcReaderError, ParseResult};
use macros::impl_default_for;

impl_default_for!(FfiNfcReader);
impl_default_for!(NfcConst);

#[derive(Debug, Clone, uniffi::Object)]
pub struct FfiNfcReader(Arc<Mutex<crate::cove_nfc::NfcReader>>);

#[uniffi::export]
impl FfiNfcReader {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let reader = crate::cove_nfc::NfcReader::new();
        Self(Arc::new(Mutex::new(reader)))
    }

    #[uniffi::method]
    pub fn parse(&self, data: Vec<u8>) -> Result<ParseResult, NfcReaderError> {
        self.0.lock().parse(data)
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NfcConst {
    pub number_of_blocks_per_chunk: u16,
    pub bytes_per_block: u16,
}

#[uniffi::export]
fn nfc_consts() -> NfcConst {
    Default::default()
}

impl NfcConst {
    fn new() -> Self {
        Self {
            number_of_blocks_per_chunk: crate::cove_nfc::NUMBER_OF_BLOCKS_PER_CHUNK,
            bytes_per_block: crate::cove_nfc::BYTES_PER_BLOCK,
        }
    }
}

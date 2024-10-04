use std::sync::Arc;

use parking_lot::Mutex;

use crate::{NfcReaderError, ParseResult};

#[derive(Debug, Clone, uniffi::Object)]
pub struct FfiNfcReader(Arc<Mutex<crate::NfcReader>>);

#[uniffi::export]
impl FfiNfcReader {
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

impl Default for FfiNfcReader {
    fn default() -> Self {
        Self::new()
    }
}

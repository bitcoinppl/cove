use std::sync::Arc;

use parking_lot::Mutex;

use crate::{NfcReaderError, ParseResult};
use cove_macros::impl_default_for;

use super::{message_info::MessageInfo, record::NdefRecord};

impl_default_for!(FfiNfcReader);
impl_default_for!(NfcConst);

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

    #[uniffi::method]
    pub fn is_resumeable(&self, data: Vec<u8>) -> Result<(), crate::ResumeError> {
        self.0.lock().is_resumeable(data)
    }

    #[uniffi::method]
    pub fn is_started(&self) -> bool {
        self.0.lock().is_started()
    }

    #[uniffi::method]
    pub fn message_info(&self) -> Option<MessageInfo> {
        self.0.lock().message_info().cloned()
    }

    #[uniffi::method]
    pub fn string_from_record(&self, record: NdefRecord) -> Option<String> {
        match record.payload {
            crate::payload::NdefPayload::Text(text_payload) => Some(text_payload.text),
            crate::payload::NdefPayload::Data(data) => String::from_utf8(data).ok(),
        }
    }

    #[uniffi::method]
    pub fn data_from_records(&self, records: Vec<NdefRecord>) -> Vec<u8> {
        records
            .into_iter()
            .map(|record| record.payload)
            .filter_map(|payload| match payload {
                crate::payload::NdefPayload::Data(data) => Some(data),
                _ => None,
            })
            .flatten()
            .collect::<Vec<u8>>()
    }
}

#[derive(Debug, Clone, uniffi::Object)]
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

    pub fn number_of_blocks_per_chunk(&self) -> u16 {
        self.number_of_blocks_per_chunk
    }

    pub fn bytes_per_block(&self) -> u16 {
        self.bytes_per_block
    }

    pub fn total_bytes_per_chunk(&self) -> u16 {
        self.number_of_blocks_per_chunk() * self.bytes_per_block()
    }
}

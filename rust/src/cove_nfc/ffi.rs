use std::sync::Arc;

use parking_lot::Mutex;

use crate::cove_nfc::{NfcReaderError, ParseResult};
use macros::impl_default_for;

use super::{message_info::MessageInfo, record::NdefRecord};

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

    #[uniffi::method]
    pub fn is_resumeable(&self, data: Vec<u8>) -> Result<(), crate::cove_nfc::ResumeError> {
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
            crate::cove_nfc::payload::NdefPayload::Text(text_payload) => Some(text_payload.text),
            crate::cove_nfc::payload::NdefPayload::Data(data) => String::from_utf8(data).ok(),
        }
    }

    #[uniffi::method]
    pub fn data_from_records(&self, records: Vec<NdefRecord>) -> Vec<u8> {
        records
            .into_iter()
            .map(|record| record.payload)
            .filter_map(|payload| match payload {
                crate::cove_nfc::payload::NdefPayload::Data(data) => Some(data),
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
            number_of_blocks_per_chunk: crate::cove_nfc::NUMBER_OF_BLOCKS_PER_CHUNK,
            bytes_per_block: crate::cove_nfc::BYTES_PER_BLOCK,
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

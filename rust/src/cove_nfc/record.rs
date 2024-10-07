use crate::cove_nfc::{header::NdefHeader, payload::NdefPayload};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NdefRecord {
    pub header: NdefHeader,
    pub type_: Vec<u8>,
    pub id: Option<Vec<u8>>,
    pub payload: NdefPayload,
}

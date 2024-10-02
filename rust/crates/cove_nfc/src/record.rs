use crate::{header::NdefHeader, payload::NdefPayload};

#[derive(Debug)]
pub struct NdefRecord {
    pub header: NdefHeader,
    pub type_: Vec<u8>,
    pub id: Option<Vec<u8>>,
    pub payload: NdefPayload,
}

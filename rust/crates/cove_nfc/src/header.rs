use crate::ndef_type::NdefType;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NdefHeader {
    pub message_begin: bool,
    pub message_end: bool,
    pub chunked: bool,
    pub short_record: bool,
    pub has_id_length: bool,
    pub type_name_format: NdefType,
    pub type_length: u8,
    pub payload_length: u32,
    pub id_length: Option<u8>,
}

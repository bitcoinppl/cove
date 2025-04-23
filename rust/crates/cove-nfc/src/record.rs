use crate::{header::NdefHeader, payload::NdefPayload};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NdefRecord {
    pub header: NdefHeader,
    pub type_: Vec<u8>,
    pub id: Option<Vec<u8>>,
    pub payload: NdefPayload,
}

// only used for uniffi
mod ffi {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
    pub struct NdefRecordReader {
        record: NdefRecord,
    }

    #[uniffi::export]
    impl NdefRecordReader {
        #[uniffi::constructor]
        pub fn new(record: NdefRecord) -> Self {
            Self { record }
        }

        pub fn type_(&self) -> Option<String> {
            String::from_utf8(self.record.type_.clone()).ok()
        }

        pub fn id(&self) -> Option<String> {
            let id = self.record.id.as_ref()?;
            String::from_utf8(id.clone()).ok()
        }
    }
}

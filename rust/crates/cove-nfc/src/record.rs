use crate::{header::NdefHeader, payload::NdefPayload};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NdefRecord {
    pub header: NdefHeader,
    pub type_: Vec<u8>,
    pub id: Option<Vec<u8>>,
    pub payload: NdefPayload,
}

// only used for uniffi
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct NdefRecordReader {
    record: NdefRecord,
}

#[uniffi::export]
impl NdefRecordReader {
    #[uniffi::constructor]
    #[must_use]
    pub const fn new(record: NdefRecord) -> Self {
        Self { record }
    }

    #[must_use]
    pub fn type_(&self) -> Option<String> {
        String::from_utf8(self.record.type_.clone()).ok()
    }

    #[must_use]
    pub fn id(&self) -> Option<String> {
        let id = self.record.id.as_ref()?;
        String::from_utf8(id.clone()).ok()
    }
}

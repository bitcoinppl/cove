#[derive(Debug, Clone, uniffi::Enum)]
pub enum StringOrData {
    String(String),
    Data(Vec<u8>),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MultiFormat {
    NoOp,
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error, derive_more::Display)]
pub enum MultiFormatError {
    /// Invalid QR
    InvalidQr,
}

impl StringOrData {
    pub fn new(data: Vec<u8>) -> Self {
        if let Ok(str) = std::str::from_utf8(&data) {
            Self::String(str.to_string())
        } else {
            Self::Data(data)
        }
    }
}

impl TryFrom<StringOrData> for MultiFormat {
    type Error = MultiFormatError;

    fn try_from(value: StringOrData) -> Result<Self, Self::Error> {
        todo!()
    }
}

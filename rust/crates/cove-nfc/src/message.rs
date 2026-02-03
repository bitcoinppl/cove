/// A NFC message, could contain a string, data (bytes) or both
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub enum NfcMessage {
    String(String),
    Data(Vec<u8>),
    Both(String, Vec<u8>),
}

#[derive(Debug, Copy, Clone, thiserror::Error, uniffi::Error)]
pub enum NfcMessageError {
    #[error("neither string nor data was provided")]
    NoStringNorData,
}

pub type Error = NfcMessageError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl NfcMessage {
    /// Creates a new NFC message from optional string and data.
    ///
    /// # Errors
    /// Returns an error if neither string nor data is provided.
    #[uniffi::constructor(default(string = None, data = None))]
    pub fn try_new(mut string: Option<String>, mut data: Option<Vec<u8>>) -> Result<Self> {
        if let Some(str) = &string
            && str.is_empty()
        {
            string = None;
        }

        if let Some(d) = &data
            && d.is_empty()
        {
            data = None;
        }

        match (string, data) {
            (Some(string), None) => Ok(Self::String(string)),
            (None, Some(data)) => Ok(Self::Data(data)),
            (Some(string), Some(data)) => Ok(Self::Both(string, data)),
            (None, None) => Err(NfcMessageError::NoStringNorData),
        }
    }

    #[uniffi::method]
    #[must_use]
    pub fn string(&self) -> Option<String> {
        match self {
            Self::String(s) | Self::Both(s, _) => Some(s.clone()),
            Self::Data(_) => None,
        }
    }

    #[uniffi::method]
    #[must_use]
    pub fn data(&self) -> Option<Vec<u8>> {
        match self {
            Self::Data(d) | Self::Both(_, d) => Some(d.clone()),
            Self::String(_) => None,
        }
    }
}

#[uniffi::export]
fn nfc_message_is_equal(lhs: &NfcMessage, rhs: &NfcMessage) -> bool {
    lhs == rhs
}

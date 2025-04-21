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
    #[uniffi::constructor(default(string = None, data = None))]
    pub fn try_new(mut string: Option<String>, mut data: Option<Vec<u8>>) -> Result<Self> {
        if let Some(str) = &string {
            if str.is_empty() {
                string = None;
            }
        }

        if let Some(d) = &data {
            if d.is_empty() {
                data = None;
            }
        }

        match (string, data) {
            (Some(string), None) => Ok(Self::String(string)),
            (None, Some(data)) => Ok(Self::Data(data)),
            (Some(string), Some(data)) => Ok(Self::Both(string, data)),
            (None, None) => Err(NfcMessageError::NoStringNorData),
        }
    }

    #[uniffi::method]
    pub fn string(&self) -> Option<String> {
        match self {
            NfcMessage::String(s) => Some(s.clone()),
            NfcMessage::Both(s, _d) => Some(s.clone()),
            _ => None,
        }
    }

    #[uniffi::method]
    pub fn data(&self) -> Option<Vec<u8>> {
        match self {
            NfcMessage::Data(d) => Some(d.clone()),
            NfcMessage::Both(_s, d) => Some(d.clone()),
            _ => None,
        }
    }
}

#[uniffi::export]
fn nfc_message_is_equal(lhs: &NfcMessage, rhs: &NfcMessage) -> bool {
    lhs == rhs
}

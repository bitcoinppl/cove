#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NdefPayload {
    Text(TextPayload),
    Data(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TextPayload {
    pub format: TextPayloadFormat,
    pub language: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TextPayloadFormat {
    Utf8,
    Utf16,
}
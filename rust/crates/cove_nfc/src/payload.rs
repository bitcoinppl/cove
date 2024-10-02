#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NdefPayload {
    Text(TextPayload),
    Data(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPayload {
    pub format: TextPayloadFormat,
    pub language: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextPayloadFormat {
    Utf8,
    Utf16,
}

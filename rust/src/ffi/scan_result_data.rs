#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiScanResultData {
    String(String),
    Data(Vec<u8>),
}

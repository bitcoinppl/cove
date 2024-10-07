#[derive(Debug, PartialEq, Eq, Clone, Copy, uniffi::Record)]
pub struct MessageInfo {
    pub total_payload_length: u16,
}

impl MessageInfo {
    pub fn new(total_payload_length: u16) -> Self {
        Self {
            total_payload_length,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, uniffi::Record)]
pub struct MessageInfo {
    /// The payload length of the message, including the header info
    pub full_message_length: u16,

    /// The payload length of the message, reported in the info header
    /// This is the length of the payload, without the header info
    reported_length: u16,
}

impl MessageInfo {
    pub fn new(reported_length: u16) -> Self {
        Self {
            reported_length,
            full_message_length: total_with_info(reported_length),
        }
    }
}

fn total_with_info(total_payload_length: u16) -> u16 {
    let fixed_header_length = [226, 67, 0, 1, 0, 0, 4, 0, 3].len() as u16;
    let payload_length_indicator_length = if total_payload_length < 255 {
        1
    } else {
        // 1 byte (255) to indicate the length is longer than 255
        // the length is encoded as a u16 (2 bytes)
        3
    };

    total_payload_length + fixed_header_length + payload_length_indicator_length
}

/// Minimal BBQr encoder/decoder for Key Teleport packet types (R and S).
///
/// BBQr format: `B$<encoding><file_type><num_parts_hex2><part_index_hex2><base32_data>`
/// For single-frame packets: num_parts=01, part_index=00.
/// Encoding byte `2` = Base32, no compression (as required by the COLDCARD spec).
use data_encoding::BASE32_NOPAD;

use crate::error::Error;

/// Key Teleport BBQr file type codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KtFileType {
    /// `R` — receiver packet (encrypted pubkey)
    Receiver,
    /// `S` — sender packet (sender pubkey + encrypted body)
    Sender,
}

impl KtFileType {
    pub fn as_char(self) -> char {
        match self {
            KtFileType::Receiver => 'R',
            KtFileType::Sender => 'S',
        }
    }

    fn from_char(c: char) -> Result<Self, Error> {
        match c {
            'R' => Ok(KtFileType::Receiver),
            'S' => Ok(KtFileType::Sender),
            other => Err(Error::InvalidBbqr(format!("unknown Key Teleport type: '{other}'"))),
        }
    }
}

/// Encode binary data as a single-frame BBQr string with the given Key Teleport file type.
pub fn encode(data: &[u8], file_type: KtFileType) -> String {
    let b32 = BASE32_NOPAD.encode(data);
    // num_parts=01 (1 frame total), part_index=00 (first/only frame)
    format!("B$2{}0100{}", file_type.as_char(), b32)
}

/// Decode a single-frame BBQr string, returning the file type and binary payload.
/// Multi-frame packets are rejected — higher-level transport code handles reassembly.
pub fn decode(s: &str) -> Result<(KtFileType, Vec<u8>), Error> {
    let s = s.trim().to_uppercase();

    let rest =
        s.strip_prefix("B$").ok_or_else(|| Error::InvalidBbqr("missing 'B$' header".into()))?;

    if rest.len() < 6 {
        return Err(Error::InvalidBbqr("too short to be a valid BBQr packet".into()));
    }

    let mut chars = rest.chars();
    let encoding = chars.next().unwrap();
    if encoding != '2' {
        return Err(Error::InvalidBbqr(format!(
            "unsupported encoding '{encoding}' (only Base32/'2' is supported)"
        )));
    }

    let file_type = KtFileType::from_char(chars.next().unwrap())?;

    // num_parts and part_index are 2 uppercase hex chars each
    let header_tail: String = chars.take(4).collect();
    if header_tail.len() != 4 {
        return Err(Error::InvalidBbqr("truncated header".into()));
    }
    let num_parts = u8::from_str_radix(&header_tail[0..2], 16)
        .map_err(|_| Error::InvalidBbqr("bad num_parts".into()))?;
    let part_index = u8::from_str_radix(&header_tail[2..4], 16)
        .map_err(|_| Error::InvalidBbqr("bad part_index".into()))?;

    if num_parts != 1 || part_index != 0 {
        return Err(Error::InvalidBbqr(format!(
            "multi-frame BBQr not supported here (num_parts={num_parts}, part_index={part_index})"
        )));
    }

    let b32_data = &s[8..]; // "B$" + encoding + type + 4 header chars = 8
    let data = BASE32_NOPAD
        .decode(b32_data.as_bytes())
        .map_err(|e| Error::InvalidBbqr(format!("Base32 decode failed: {e}")))?;

    Ok((file_type, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip_receiver() {
        let data = vec![0xAAu8; 33];
        let encoded = encode(&data, KtFileType::Receiver);
        assert!(encoded.starts_with("B$2R0100"));
        let (ft, decoded) = decode(&encoded).unwrap();
        assert_eq!(ft, KtFileType::Receiver);
        assert_eq!(decoded, data);
    }

    #[test]
    fn encode_decode_roundtrip_sender() {
        let mut data = vec![0x01u8; 33];
        data.extend_from_slice(&[0xBBu8; 80]);
        let encoded = encode(&data, KtFileType::Sender);
        assert!(encoded.starts_with("B$2S0100"));
        let (ft, decoded) = decode(&encoded).unwrap();
        assert_eq!(ft, KtFileType::Sender);
        assert_eq!(decoded, data);
    }

    #[test]
    fn decode_known_example() {
        // From keyteleport.com: B$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO
        let s = "B$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO";
        let (ft, data) = decode(s).unwrap();
        assert_eq!(ft, KtFileType::Receiver);
        assert_eq!(data.len(), 33);
    }

    #[test]
    fn rejects_wrong_header() {
        assert!(decode("QR2R0100AAAA").is_err());
    }

    #[test]
    fn rejects_unknown_file_type() {
        assert!(decode("B$2E0100AAAA").is_err());
    }
}

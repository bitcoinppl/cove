use std::{fmt, sync::Arc};

use bbqr::file_type::FileType;
use derive_more::{AsRef, Deref, From, Into};
use keyteleport::{Error as ProtocolError, Packet, PsbtPacket, ReceiverPacket, SenderPacket};

use crate::multi_format::StringOrData;

#[derive(Clone, PartialEq, Eq, From, Into, AsRef, Deref, uniffi::Object)]
pub struct KeyTeleportReceiverPacket(ReceiverPacket);

impl fmt::Debug for KeyTeleportReceiverPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("KeyTeleportReceiverPacket")
            .field(&format_args!("{} bytes", self.0.as_bytes().len()))
            .finish()
    }
}

#[uniffi::export]
impl KeyTeleportReceiverPacket {
    pub fn bbqr_part(&self) -> Result<String, KeyTeleportPacketEncodingError> {
        self.0.to_bbqr_part().map_err(Into::into)
    }

    pub fn url(&self) -> Result<String, KeyTeleportPacketEncodingError> {
        self.0.to_url().map_err(Into::into)
    }
}

#[derive(Clone, PartialEq, Eq, From, Into, AsRef, Deref, uniffi::Object)]
pub struct KeyTeleportSenderPacket(SenderPacket);

impl fmt::Debug for KeyTeleportSenderPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("KeyTeleportSenderPacket")
            .field(&format_args!("{} bytes", self.0.as_bytes().len()))
            .finish()
    }
}

#[uniffi::export]
impl KeyTeleportSenderPacket {
    pub fn bbqr_part(&self) -> Result<String, KeyTeleportPacketEncodingError> {
        self.0.to_bbqr_part().map_err(Into::into)
    }

    pub fn url(&self) -> Result<String, KeyTeleportPacketEncodingError> {
        self.0.to_url().map_err(Into::into)
    }
}

/// A failure while rendering a validated KeyTeleport packet
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum KeyTeleportPacketEncodingError {
    /// The packet could not be encoded as a single-part BBQr value
    #[error("unable to encode KeyTeleport packet: {0}")]
    Encoding(String),
}

impl From<ProtocolError> for KeyTeleportPacketEncodingError {
    fn from(error: ProtocolError) -> Self {
        Self::Encoding(error.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedKeyTeleport {
    Receiver(Arc<KeyTeleportReceiverPacket>),
    Sender(Arc<KeyTeleportSenderPacket>),
    UnsupportedPsbt,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum KeyTeleportParseError {
    #[error("unrecognized KeyTeleport packet")]
    Unrecognized,
}

pub(crate) fn parse_key_teleport_string(
    value: &str,
) -> Result<ParsedKeyTeleport, KeyTeleportParseError> {
    let trimmed = value.trim();

    let packet = Packet::from_url(trimmed)
        .or_else(|_| Packet::from_bbqr_part(trimmed))
        .map_err(|_| KeyTeleportParseError::Unrecognized)?;

    parse_packet(packet)
}

pub(crate) fn parse_key_teleport_bbqr_payload(
    data: Vec<u8>,
    file_type: FileType,
) -> Result<ParsedKeyTeleport, KeyTeleportParseError> {
    let packet = match file_type {
        FileType::KeyTeleportReceiver => Packet::Receiver(
            ReceiverPacket::new(data).map_err(|_| KeyTeleportParseError::Unrecognized)?,
        ),

        FileType::KeyTeleportSender => Packet::Sender(
            SenderPacket::new(data).map_err(|_| KeyTeleportParseError::Unrecognized)?,
        ),

        FileType::KeyTeleportPsbt => Packet::Psbt(PsbtPacket::new(data)),

        _ => return Err(KeyTeleportParseError::Unrecognized),
    };

    parse_packet(packet)
}

pub(crate) fn parse_key_teleport_input(
    input: StringOrData,
) -> Result<ParsedKeyTeleport, KeyTeleportParseError> {
    match input {
        StringOrData::String(value) => parse_key_teleport_string(&value),

        StringOrData::Data(data) => {
            let value = String::from_utf8(data).map_err(|_| KeyTeleportParseError::Unrecognized)?;
            parse_key_teleport_string(&value)
        }
    }
}

fn parse_packet(packet: Packet) -> Result<ParsedKeyTeleport, KeyTeleportParseError> {
    match packet {
        Packet::Receiver(packet) => {
            Ok(ParsedKeyTeleport::Receiver(Arc::new(KeyTeleportReceiverPacket::from(packet))))
        }

        Packet::Sender(packet) => {
            Ok(ParsedKeyTeleport::Sender(Arc::new(KeyTeleportSenderPacket::from(packet))))
        }

        Packet::Psbt(_) => Ok(ParsedKeyTeleport::UnsupportedPsbt),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use keyteleport::{Payload, ReceiverSession, SenderSession};

    use super::*;

    const DOC_EXAMPLE_R_URL: &str =
        "https://keyteleport.com/#B$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO";

    #[test]
    fn parses_keyteleport_url_receiver_packet() {
        let parsed = parse_key_teleport_string(DOC_EXAMPLE_R_URL).unwrap();

        match parsed {
            ParsedKeyTeleport::Receiver(packet) => {
                assert!(packet.bbqr_part().unwrap().starts_with("B$2R0100"));
            }
            _ => panic!("expected receiver packet"),
        }
    }

    #[test]
    fn parses_keyteleport_sender_packet() {
        let receiver = ReceiverSession::from_private_key_bytes([1; 32]).unwrap();
        let request = receiver.request().unwrap();
        let sender = SenderSession::new(&request.packet, &request.numeric_code).unwrap();
        let mnemonic = bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let response = sender.send(Payload::mnemonic(mnemonic).unwrap()).unwrap();
        let packet = KeyTeleportSenderPacket::from(response.packet);

        let parsed = parse_key_teleport_string(&packet.bbqr_part().unwrap()).unwrap();

        assert!(matches!(parsed, ParsedKeyTeleport::Sender(_)));
    }

    #[test]
    fn parses_keyteleport_psbt_as_typed_unsupported() {
        let packet = Packet::Psbt(PsbtPacket::new(vec![1, 2, 3, 4]));
        let bbqr = packet.to_bbqr_part().unwrap();
        let parsed = parse_key_teleport_string(&bbqr).unwrap();

        assert!(matches!(parsed, ParsedKeyTeleport::UnsupportedPsbt));
    }
}

use bbqr::{
    encode::Encoding,
    file_type::FileType,
    join::Joined,
    split::{Split, SplitOptions},
};

use crate::{Error, Result, crypto};

const KEY_TELEPORT_DOMAIN: &str = "keyteleport.com";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    Receiver(ReceiverPacket),
    Sender(SenderPacket),
    Psbt(PsbtPacket),
}

impl Packet {
    pub fn from_bbqr_part(value: &str) -> Result<Self> {
        let joined = Joined::try_from_parts(vec![value.to_string()])?;

        match joined.file_type {
            FileType::KeyTeleportReceiver => Ok(Self::Receiver(ReceiverPacket::new(joined.data)?)),
            FileType::KeyTeleportSender => Ok(Self::Sender(SenderPacket::new(joined.data)?)),
            FileType::KeyTeleportPsbt => Ok(Self::Psbt(PsbtPacket::new(joined.data))),
            _ => Err(Error::InvalidPacket),
        }
    }

    pub fn from_url(value: &str) -> Result<Self> {
        let url = parse_keyteleport_url(value)?;
        let fragment = url.fragment().ok_or(Error::InvalidUrl)?;

        Self::from_bbqr_part(fragment)
    }

    pub fn to_bbqr_part(&self) -> Result<String> {
        match self {
            Self::Receiver(packet) => packet.to_bbqr_part(),
            Self::Sender(packet) => packet.to_bbqr_part(),
            Self::Psbt(packet) => packet.to_bbqr_part(),
        }
    }

    pub fn to_url(&self) -> Result<String> {
        Ok(format!("https://{KEY_TELEPORT_DOMAIN}/#{}", self.to_bbqr_part()?))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReceiverPacket(Vec<u8>);

impl ReceiverPacket {
    pub fn new(payload: Vec<u8>) -> Result<Self> {
        if payload.len() != crypto::RECEIVER_PACKET_LEN {
            return Err(Error::InvalidReceiverPacket);
        }

        Ok(Self(payload))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_bbqr_part(&self) -> Result<String> {
        to_single_part_bbqr(&self.0, FileType::KeyTeleportReceiver)
    }

    pub fn to_url(&self) -> Result<String> {
        Packet::Receiver(self.clone()).to_url()
    }
}

impl std::fmt::Debug for ReceiverPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ReceiverPacket").field(&format_args!("{} bytes", self.0.len())).finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SenderPacket(Vec<u8>);

impl SenderPacket {
    pub fn new(payload: Vec<u8>) -> Result<Self> {
        if payload.len() < 37 {
            return Err(Error::InvalidSenderPacket);
        }

        Ok(Self(payload))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn sender_pubkey_bytes(&self) -> &[u8] {
        &self.0[..33]
    }

    pub fn encrypted_body(&self) -> &[u8] {
        &self.0[33..]
    }

    pub fn to_bbqr_part(&self) -> Result<String> {
        to_single_part_bbqr(&self.0, FileType::KeyTeleportSender)
    }

    pub fn to_url(&self) -> Result<String> {
        Packet::Sender(self.clone()).to_url()
    }
}

impl std::fmt::Debug for SenderPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SenderPacket").field(&format_args!("{} bytes", self.0.len())).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsbtPacket(Vec<u8>);

impl PsbtPacket {
    pub fn new(payload: Vec<u8>) -> Self {
        Self(payload)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_bbqr_part(&self) -> Result<String> {
        to_single_part_bbqr(&self.0, FileType::KeyTeleportPsbt)
    }
}

fn to_single_part_bbqr(payload: &[u8], file_type: FileType) -> Result<String> {
    let split = Split::try_from_data(
        payload,
        file_type,
        SplitOptions {
            encoding: Encoding::Base32,
            min_split_number: 1,
            max_split_number: 1,
            ..Default::default()
        },
    )?;

    split.parts.into_iter().next().ok_or(Error::InvalidPacket)
}

fn parse_keyteleport_url(value: &str) -> Result<url::Url> {
    let trimmed = value.trim();
    let parseable = if trimmed.to_ascii_lowercase().starts_with(&format!("{KEY_TELEPORT_DOMAIN}/"))
    {
        format!("https://{trimmed}")
    } else {
        trimmed.to_string()
    };

    let url = url::Url::parse(&parseable)?;
    let host = url.host_str().ok_or(Error::InvalidUrl)?;

    if !host.eq_ignore_ascii_case(KEY_TELEPORT_DOMAIN) {
        return Err(Error::InvalidUrl);
    }

    Ok(url)
}

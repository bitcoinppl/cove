use std::{fmt, str::FromStr};

use bip39::Mnemonic;
use bitcoin::{base58, bip32::Xpriv, secp256k1::SecretKey};
use zeroize::Zeroize as _;

use crate::{Error, Result};

const MAINNET_XPRV_VERSION: [u8; 4] = [0x04, 0x88, 0xad, 0xe4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedPayloadKind {
    Notes,
    Vault,
    Psbt,
    Backup,
    Unknown(u8),
}

impl fmt::Display for UnsupportedPayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Notes => f.write_str("n"),
            Self::Vault => f.write_str("v"),
            Self::Psbt => f.write_str("p"),
            Self::Backup => f.write_str("b"),
            Self::Unknown(code) => write!(f, "0x{code:02x}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Payload {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

impl Payload {
    pub fn mnemonic(mnemonic: Mnemonic) -> Self {
        Self::Mnemonic(mnemonic)
    }

    pub fn xprv(value: impl AsRef<str>) -> Result<Self> {
        Ok(Self::Xprv(XprvPayload::parse(value.as_ref())?))
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Mnemonic(mnemonic) => encode_mnemonic_payload(mnemonic),
            Self::Xprv(xprv) => encode_xprv_payload(xprv),
        }
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mnemonic(_) => f.write_str("Payload::Mnemonic(****)"),
            Self::Xprv(_) => f.write_str("Payload::Xprv(****)"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct XprvPayload {
    value: String,
}

impl XprvPayload {
    pub fn parse(value: &str) -> Result<Self> {
        let xprv = Xpriv::from_str(value).map_err(|_| Error::InvalidXprvPayload)?;

        Ok(Self { value: xprv.to_string() })
    }

    pub fn expose_string(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for XprvPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("XprvPayload(****)")
    }
}

impl Drop for XprvPayload {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum DecodedPayload {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

impl DecodedPayload {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        let (&code, body) = bytes.split_first().ok_or(Error::InvalidPacket)?;

        match code {
            b's' => decode_stash_payload(body),
            b'x' => decode_xprv_body(body),
            b'n' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Notes)),
            b'v' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Vault)),
            b'p' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Psbt)),
            b'b' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Backup)),
            other => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Unknown(other))),
        }
    }
}

impl fmt::Debug for DecodedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mnemonic(_) => f.write_str("DecodedPayload::Mnemonic(****)"),
            Self::Xprv(_) => f.write_str("DecodedPayload::Xprv(****)"),
        }
    }
}

fn encode_mnemonic_payload(mnemonic: &Mnemonic) -> Result<Vec<u8>> {
    let entropy = mnemonic.to_entropy();
    let marker = 0x80 | ((entropy.len() / 8) - 2) as u8;
    let mut encoded = Vec::with_capacity(1 + 1 + entropy.len());
    encoded.push(b's');
    encoded.push(marker);
    encoded.extend_from_slice(&entropy);

    Ok(encoded)
}

fn encode_xprv_payload(xprv: &XprvPayload) -> Result<Vec<u8>> {
    let decoded = base58::decode_check(&xprv.value)?;
    Xpriv::decode(&decoded)?;

    let mut encoded = Vec::with_capacity(1 + decoded.len());
    encoded.push(b'x');
    encoded.extend_from_slice(&decoded);

    Ok(encoded)
}

fn decode_stash_payload(body: &[u8]) -> Result<DecodedPayload> {
    let (&marker, rest) = body.split_first().ok_or(Error::InvalidMnemonicPayload)?;

    if marker == 0x01 {
        return decode_stash_xprv(rest);
    }

    if marker & 0x80 == 0 {
        return Err(Error::InvalidMnemonicPayload);
    }

    let entropy_len = usize::from((marker & 0x03) + 2) * 8;
    if !matches!(entropy_len, 16 | 24 | 32) || rest.len() < entropy_len {
        return Err(Error::InvalidMnemonicPayload);
    }

    let mnemonic = Mnemonic::from_entropy(&rest[..entropy_len])?;

    Ok(DecodedPayload::Mnemonic(mnemonic))
}

fn decode_stash_xprv(body: &[u8]) -> Result<DecodedPayload> {
    if body.len() < 64 {
        return Err(Error::InvalidXprvPayload);
    }

    let mut encoded = [0_u8; 78];
    encoded[0..4].copy_from_slice(&MAINNET_XPRV_VERSION);
    encoded[13..45].copy_from_slice(&body[0..32]);
    encoded[45] = 0;
    encoded[46..78].copy_from_slice(&body[32..64]);

    let xprv = Xpriv::decode(&encoded)?;
    SecretKey::from_slice(&body[32..64]).map_err(|_| Error::InvalidXprvPayload)?;

    Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }))
}

fn decode_xprv_body(body: &[u8]) -> Result<DecodedPayload> {
    let xprv = Xpriv::decode(body).map_err(|_| Error::InvalidXprvPayload)?;

    Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }))
}

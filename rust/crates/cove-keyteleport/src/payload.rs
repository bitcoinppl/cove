use bip39::Mnemonic;

use crate::error::Error;

/// Type byte prefixes as defined in the Key Teleport spec.
const TYPE_MNEMONIC: u8 = b's';
const TYPE_XPRV: u8 = b'x';

/// A decrypted Key Teleport payload — the secret being transferred.
#[derive(Debug)]
pub enum Payload {
    /// A BIP-39 mnemonic (12 / 18 / 24 words). Type byte `s`.
    Mnemonic(Mnemonic),
    /// A base58-encoded XPRV (serialised `ExtendedPrivKey`). Type byte `x`.
    Xprv(String),
}

impl Payload {
    /// Serialise the payload for encryption: `type_byte || secret_bytes`.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match self {
            Payload::Mnemonic(m) => {
                // Encode the mnemonic entropy as raw bytes, prefixed with the type byte
                let entropy = m.to_entropy();
                let mut out = Vec::with_capacity(1 + entropy.len());
                out.push(TYPE_MNEMONIC);
                out.extend_from_slice(&entropy);
                out
            }
            Payload::Xprv(xprv) => {
                // base58-decoded binary XPRV (78 bytes), prefixed with the type byte
                let decoded =
                    bitcoin::base58::decode(xprv).unwrap_or_else(|_| xprv.as_bytes().to_vec());
                let mut out = Vec::with_capacity(1 + decoded.len());
                out.push(TYPE_XPRV);
                out.extend_from_slice(&decoded);
                out
            }
        }
    }

    /// Deserialise from `type_byte || secret_bytes` after decryption.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::InvalidPayload("empty payload".into()));
        }
        match bytes[0] {
            TYPE_MNEMONIC => {
                let entropy = &bytes[1..];
                let m = Mnemonic::from_entropy(entropy)
                    .map_err(|e| Error::InvalidPayload(format!("invalid mnemonic entropy: {e}")))?;
                Ok(Payload::Mnemonic(m))
            }
            TYPE_XPRV => {
                let bin = &bytes[1..];
                let xprv = bitcoin::base58::encode(bin);
                Ok(Payload::Xprv(xprv))
            }
            other => Err(Error::InvalidPayload(format!("unknown payload type byte 0x{other:02X}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_roundtrip() {
        let entropy = [0x11u8; 32]; // 32 bytes → 24-word mnemonic
        let m = Mnemonic::from_entropy(&entropy).unwrap();
        let p = Payload::Mnemonic(m);
        let bytes = p.to_bytes();
        assert_eq!(bytes[0], TYPE_MNEMONIC);
        let recovered = Payload::from_bytes(&bytes).unwrap();
        match (p, recovered) {
            (Payload::Mnemonic(a), Payload::Mnemonic(b)) => {
                assert_eq!(a.to_string(), b.to_string())
            }
            _ => panic!("type mismatch"),
        }
    }

    #[test]
    fn unknown_type_byte_is_error() {
        assert!(Payload::from_bytes(&[0xFF, 1, 2, 3]).is_err());
    }

    #[test]
    fn empty_bytes_is_error() {
        assert!(Payload::from_bytes(&[]).is_err());
    }
}

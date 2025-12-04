//! Encode/decode `[u8; 4]` fingerprints as CBOR u32.
//!
//! BCR specs encode fingerprints as big-endian u32 values.
//! This module handles both required and optional fingerprint fields.

use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// Encode an optional fingerprint as a u32
pub fn encode<C, W: Write>(
    fingerprint: &Option<[u8; 4]>,
    encoder: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), EncodeError<W::Error>> {
    if let Some(bytes) = fingerprint {
        encoder.u32(u32::from_be_bytes(*bytes))?;
    }
    Ok(())
}

/// Decode a u32 into an optional fingerprint
pub fn decode<'b, C>(
    decoder: &mut Decoder<'b>,
    _ctx: &mut C,
) -> Result<Option<[u8; 4]>, DecodeError> {
    let value = decoder.u32()?;
    Ok(Some(value.to_be_bytes()))
}

/// Module for required (non-optional) fingerprints
pub mod required {
    use super::*;

    pub fn encode<C, W: Write>(
        fingerprint: &[u8; 4],
        encoder: &mut Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), EncodeError<W::Error>> {
        encoder.u32(u32::from_be_bytes(*fingerprint))?;
        Ok(())
    }

    pub fn decode<'b, C>(decoder: &mut Decoder<'b>, _ctx: &mut C) -> Result<[u8; 4], DecodeError> {
        let value = decoder.u32()?;
        Ok(value.to_be_bytes())
    }
}

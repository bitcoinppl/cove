//! Encode/decode `[u8; 4]` fingerprints as CBOR u32.
//!
//! BCR specs encode fingerprints as big-endian u32 values.
//! This module handles both required and optional fingerprint fields.

use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// Encode an optional fingerprint as a u32
///
/// # Errors
///
/// Returns an error if encoding fails
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
///
/// # Errors
///
/// Returns an error if decoding fails
pub fn decode<C>(decoder: &mut Decoder<'_>, _ctx: &mut C) -> Result<Option<[u8; 4]>, DecodeError> {
    let value = decoder.u32()?;
    Ok(Some(value.to_be_bytes()))
}

/// Module for required (non-optional) fingerprints
pub mod required {
    use minicbor::decode::{Decoder, Error as DecodeError};
    use minicbor::encode::{Encoder, Error as EncodeError, Write};

    /// Encode a fingerprint as a u32
    ///
    /// # Errors
    ///
    /// Returns an error if encoding fails
    pub fn encode<C, W: Write>(
        fingerprint: &[u8; 4],
        encoder: &mut Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), EncodeError<W::Error>> {
        encoder.u32(u32::from_be_bytes(*fingerprint))?;
        Ok(())
    }

    /// Decode a u32 into a fingerprint
    ///
    /// # Errors
    ///
    /// Returns an error if decoding fails
    pub fn decode<C>(decoder: &mut Decoder<'_>, _ctx: &mut C) -> Result<[u8; 4], DecodeError> {
        let value = decoder.u32()?;
        Ok(value.to_be_bytes())
    }
}

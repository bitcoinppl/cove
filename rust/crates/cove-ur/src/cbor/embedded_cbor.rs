//! Encode/decode pre-encoded CBOR as raw bytes.
//!
//! Used for fields in `CryptoHdkey` that contain nested tagged structures
//! (`CryptoCoinInfo`, `CryptoKeypath`) which are stored as pre-encoded CBOR bytes.

use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// Encode by writing raw CBOR bytes directly to the writer
///
/// # Errors
///
/// Returns an error if writing to the encoder fails
pub fn encode<C, W: Write>(
    data: &[u8],
    encoder: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), EncodeError<W::Error>> {
    // write raw bytes directly without any CBOR framing
    encoder.writer_mut().write_all(data).map_err(EncodeError::write)?;
    Ok(())
}

/// Decode by capturing the raw CBOR bytes of the current value
///
/// # Errors
///
/// Returns an error if decoding fails
pub fn decode<C>(decoder: &mut Decoder<'_>, _ctx: &mut C) -> Result<Vec<u8>, DecodeError> {
    let start = decoder.position();
    decoder.skip()?;
    let end = decoder.position();
    Ok(decoder.input()[start..end].to_vec())
}

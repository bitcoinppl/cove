//! Encode/decode optional byte vectors as CBOR bytes.
//!
//! Used for optional fields that should be encoded as CBOR byte strings.

use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// Encode bytes using CBOR byte string encoding
pub fn encode<C, W: Write>(
    data: &[u8],
    encoder: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), EncodeError<W::Error>> {
    encoder.bytes(data)?;
    Ok(())
}

/// Decode CBOR byte string into Vec<u8>
pub fn decode<'b, C>(decoder: &mut Decoder<'b>, _ctx: &mut C) -> Result<Vec<u8>, DecodeError> {
    Ok(decoder.bytes()?.to_vec())
}

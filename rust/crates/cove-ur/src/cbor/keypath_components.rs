//! Encode/decode BIP32 derivation path components.
//!
//! BCR-2020-007 allows two formats for path components:
//! 1. Plain integers with hardened bit: `[0x80000054, 0x80000000, ...]`
//! 2. Index/hardened pairs: `[84, true, 0, true, ...]`
//!
//! We always encode in format 1 (plain integers) but decode both formats.

use crate::keypath::HARDENED_FLAG;
use minicbor::data::Type;
use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// Encode path components as plain integers (format 1)
///
/// # Errors
///
/// Returns an error if encoding fails
pub fn encode<C, W: Write>(
    components: &[u32],
    encoder: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), EncodeError<W::Error>> {
    encoder.array(components.len() as u64)?;
    for &component in components {
        encoder.u32(component)?;
    }
    Ok(())
}

/// Decode path components, supporting both formats
///
/// # Errors
///
/// Returns an error if decoding fails or the array format is invalid
pub fn decode<C>(decoder: &mut Decoder<'_>, _ctx: &mut C) -> Result<Vec<u32>, DecodeError> {
    let array_length =
        decoder.array()?.ok_or_else(|| DecodeError::message("expected definite-length array"))?;

    let array_length_usize =
        usize::try_from(array_length).map_err(|_| DecodeError::message("array length overflow"))?;
    let mut components = Vec::with_capacity(array_length_usize);
    let mut element_index = 0;

    while element_index < array_length {
        let path_index = decoder.u32()?;
        element_index += 1;

        // last element in array, nothing more to check
        if element_index >= array_length {
            components.push(path_index);
            continue;
        }

        // next element isn't a bool, so this is format 1 (plain integer with hardened bit)
        let next_type = decoder.datatype()?;
        if next_type != Type::Bool {
            components.push(path_index);
            continue;
        }

        // format 2: [index, hardened] pair
        let hardened = decoder.bool()?;
        let component = if hardened { path_index | HARDENED_FLAG } else { path_index };
        components.push(component);
        element_index += 1;
    }

    Ok(components)
}

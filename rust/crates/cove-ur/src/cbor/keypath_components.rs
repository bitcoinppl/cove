//! Encode/decode BIP32 derivation path components.
//!
//! BCR-2020-007 allows two formats for path components:
//! 1. Plain integers with hardened bit: `[0x80000054, 0x80000000, ...]`
//! 2. Index/hardened pairs: `[84, true, 0, true, ...]`
//!
//! We always encode in format 1 (plain integers) but decode both formats.

use minicbor::data::Type;
use minicbor::decode::{Decoder, Error as DecodeError};
use minicbor::encode::{Encoder, Error as EncodeError, Write};

/// BIP32 hardened derivation flag (bit 31)
const HARDENED_FLAG: u32 = 0x8000_0000;

/// Encode path components as plain integers (format 1)
pub fn encode<C, W: Write>(
    components: &Vec<u32>,
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
pub fn decode<'b, C>(decoder: &mut Decoder<'b>, _ctx: &mut C) -> Result<Vec<u32>, DecodeError> {
    let array_length = decoder
        .array()?
        .ok_or_else(|| DecodeError::message("expected definite-length array"))?;

    let mut components = Vec::with_capacity(array_length as usize);
    let mut element_index = 0;

    while element_index < array_length {
        let path_index = decoder.u32()?;
        element_index += 1;

        // check if next element is a boolean (format 2: index/hardened pairs)
        if element_index < array_length {
            let next_type = decoder.datatype()?;
            if next_type == Type::Bool {
                // format 2: [index, hardened] pair
                let hardened = decoder.bool()?;
                let component = if hardened {
                    path_index | HARDENED_FLAG
                } else {
                    path_index
                };
                components.push(component);
                element_index += 1;
            } else {
                // format 1: plain integer with hardened bit already set
                components.push(path_index);
            }
        } else {
            // last element
            components.push(path_index);
        }
    }

    Ok(components)
}

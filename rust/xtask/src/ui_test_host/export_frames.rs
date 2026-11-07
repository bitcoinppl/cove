//! Joins the QR frames a UI test read off the app's PSBT export, as a hardware
//! wallet's camera would
//!
//! The test decodes each frame it sees in a screenshot and sends every distinct
//! one so far. [`join`] answers how many parts are still missing, or, once the
//! frames are complete, the PSBT they carry, which the test checks against the
//! draft and then signs through `/sign-psbt`
//!
//! The UI tests stand in for a strict signer here, so the join is at least as
//! strict as one: every frame must parse and decode on its own, the reassembled
//! UR message must match the checksum its parts carry, and the result must be a
//! PSBT. The `bbqr` and `foundation-ur` crates skip some of these checks, so
//! this module makes them itself

use std::collections::BTreeMap;

use bbqr::{encode::Encoding, file_type::FileType, header::Header, join::Joined};
use bitcoin::Psbt;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use cove_ur::CryptoPsbt;
use data_encoding::{BASE32_NOPAD, HEXUPPER};
use foundation_ur::{
    bytewords::{self, Style},
    fountain::part::Part,
    Decoder, UR,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// The checksum a UR's fountain parts carry for the whole message
const CRC32: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);

/// `B$`, the encoding, the file type, the part count and the part index
const BBQR_HEADER_LEN: usize = 8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    /// The decoded text of each frame, in any order, all from one export
    frames: Vec<String>,
}

/// Joins animated UR `crypto-psbt` or BBQr PSBT frames into the PSBT, as hex
///
/// A frame that doesn't parse, or one from another export, fails the request
/// instead of being skipped, so a misdrawn frame can't pass as a missing one
pub fn join(request: JoinRequest) -> Result<Value> {
    let Some(first) = request.frames.first() else {
        bail!("No frames to join");
    };

    // a UR is case-insensitive, and a signer that draws it in QR alphanumeric mode upper-cases it
    if first.to_ascii_lowercase().starts_with("ur:") {
        join_ur(&request.frames)
    } else if first.starts_with("B$") {
        join_bbqr(&request.frames)
    } else {
        Err(eyre!("`{first}` is neither a UR nor a BBQr frame"))
    }
}

fn join_ur(frames: &[String]) -> Result<Value> {
    let frames: Vec<String> = frames.iter().map(|frame| frame.to_ascii_lowercase()).collect();

    if let [single] = frames.as_slice() {
        if UR::parse(single).is_ok_and(|ur| ur.is_single_part()) {
            let psbt = CryptoPsbt::from_ur_string(single)
                .map_err(|error| eyre!("Invalid single-part crypto-psbt: {error}"))?;
            return Ok(complete("ur", &psbt.to_psbt_bytes()));
        }
    }

    let mut decoder = Decoder::default();
    let mut sequences = BTreeMap::new();
    let mut total = 0;

    for frame in &frames {
        let ur = UR::parse(frame).map_err(|error| eyre!("`{frame}` is not a UR: {error}"))?;
        if ur.as_type() != "crypto-psbt" {
            bail!("`{frame}` is a {}, not a crypto-psbt", ur.as_type());
        }

        let checksum = fountain_checksum(&ur).wrap_err_with(|| format!("`{frame}`"))?;
        sequences.insert(ur.sequence().unwrap_or_default(), checksum);
        total = ur.sequence_count().unwrap_or_default();
        decoder.receive(ur).map_err(|error| eyre!("`{frame}` doesn't join the others: {error}"))?;
    }

    let Some(cbor) = decoder.message().map_err(|error| eyre!("UR decoding failed: {error}"))?
    else {
        // the export shows only the plain fragments 1 to N, so each one missing is a part left
        let parts_left = (1..=total).filter(|sequence| !sequences.contains_key(sequence)).count();
        return Ok(json!({ "complete": false, "parts_left": parts_left.max(1) }));
    };

    // foundation-ur reassembles the message without checking it against the checksum its parts
    // carry, which the decoder has already checked they agree on
    let expected = sequences.values().next().copied().unwrap_or_default();
    let actual = CRC32.checksum(cbor);
    if actual != expected {
        bail!(
            "The joined UR message has checksum {actual:08x}, but its parts carry {expected:08x}"
        );
    }

    let psbt = CryptoPsbt::decode(cbor.to_vec())
        .map_err(|error| eyre!("The UR is not a crypto-psbt: {error}"))?;
    Ok(complete("ur", &psbt.to_psbt_bytes()))
}

/// The whole-message checksum a multi-part UR's fountain part carries
fn fountain_checksum(ur: &UR) -> Result<u32> {
    let Some(fragment) = ur.as_bytewords() else {
        bail!("The UR has no Bytewords payload");
    };

    let cbor = bytewords::decode(fragment, Style::Minimal)
        .map_err(|error| eyre!("Invalid Bytewords: {error}"))?;
    let part: Part =
        minicbor::decode(&cbor).map_err(|error| eyre!("Invalid fountain part: {error}"))?;

    Ok(part.checksum)
}

/// One BBQr frame, checked on its own
struct BbqrPart {
    /// The header up to the part index, which every part of one split shares
    split: String,
    total: usize,
    index: usize,
    payload: String,
}

impl BbqrPart {
    /// Parses and decodes a frame, failing on anything a strict reader would reject
    ///
    /// `bbqr` panics on a malformed part index and drops a part whose payload doesn't decode, so
    /// each part is checked here before the crate joins them
    fn parse(frame: &str) -> Result<Self> {
        if !frame.is_ascii() {
            bail!("`{frame}` is not ASCII");
        }

        if frame.len() <= BBQR_HEADER_LEN {
            bail!("`{frame}` has no payload after its header");
        }

        let header = Header::try_from_str(frame).map_err(|error| eyre!("`{frame}`: {error}"))?;
        if header.file_type != FileType::Psbt {
            bail!("`{frame}` holds a {:?}, not a PSBT", header.file_type);
        }

        let total =
            base36(&frame[4..6]).ok_or_else(|| eyre!("`{frame}` has an invalid part count"))?;
        let index =
            base36(&frame[6..8]).ok_or_else(|| eyre!("`{frame}` has an invalid part index"))?;
        if total == 0 || index >= total {
            bail!("`{frame}` is part {index} of {total}");
        }

        let payload = &frame[BBQR_HEADER_LEN..];
        let decoded = match header.encoding {
            Encoding::Hex => HEXUPPER.decode(payload.as_bytes()),
            Encoding::Base32 | Encoding::Zlib => BASE32_NOPAD.decode(payload.as_bytes()),
        };
        decoded.map_err(|error| eyre!("`{frame}` has an undecodable payload: {error}"))?;

        Ok(Self { split: frame[..6].to_string(), total, index, payload: payload.to_string() })
    }
}

/// Two uppercase base 36 digits, as a BBQr header writes its part count and index
fn base36(digits: &str) -> Option<usize> {
    if !digits.bytes().all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase()) {
        return None;
    }

    usize::from_str_radix(digits, 36).ok()
}

fn join_bbqr(frames: &[String]) -> Result<Value> {
    let parts = frames.iter().map(|frame| BbqrPart::parse(frame)).collect::<Result<Vec<_>>>()?;
    let first = &parts[0];

    let mut payloads = BTreeMap::new();
    for part in &parts {
        if part.split != first.split {
            bail!("A `{}` part doesn't join a `{}` split", part.split, first.split);
        }

        if let Some(seen) = payloads.insert(part.index, &part.payload) {
            if seen != &part.payload {
                bail!("Part {} came with two different payloads", part.index);
            }
        }
    }

    if payloads.len() < first.total {
        return Ok(json!({ "complete": false, "parts_left": first.total - payloads.len() }));
    }

    let joined = Joined::try_from_parts(frames.to_vec())
        .map_err(|error| eyre!("The BBQr parts don't join: {error}"))?;
    Psbt::deserialize(&joined.data).wrap_err("The joined BBQr is not a valid PSBT")?;

    Ok(complete("bbqr", &joined.data))
}

fn complete(format: &str, psbt: &[u8]) -> Value {
    json!({ "complete": true, "format": format, "psbt": hex::encode(psbt) })
}

#[cfg(test)]
mod tests {
    use bbqr::split::{Split, SplitOptions};
    use bitcoin::{
        absolute::LockTime, hashes::Hash as _, transaction, Amount, OutPoint, ScriptBuf,
        Transaction, TxIn, TxOut, Txid,
    };

    use super::*;
    use crate::util;

    fn psbt_bytes() -> Vec<u8> {
        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: (0..4)
                .map(|vout| TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array([0x42; 32]), vout),
                    ..TxIn::default()
                })
                .collect(),
            output: vec![TxOut {
                value: Amount::from_sat(50_000),
                script_pubkey: ScriptBuf::new_op_return([0x11; 40]),
            }],
        };

        Psbt::from_unsigned_tx(tx).unwrap().serialize()
    }

    fn join_frames(frames: Vec<String>) -> Result<Value> {
        join(JoinRequest { frames })
    }

    /// A BBQr split of `data` with no compression, at least `parts` parts long
    fn base32_parts(data: &[u8], parts: usize) -> Vec<String> {
        let options = SplitOptions {
            encoding: Encoding::Base32,
            min_split_number: parts,
            ..SplitOptions::default()
        };

        Split::try_from_data(data, FileType::Psbt, options).unwrap().parts
    }

    #[test]
    fn out_of_order_frames_join_once_every_part_is_seen() {
        let psbt = psbt_bytes();

        for (format, mut frames) in [
            ("ur", util::ur_parts(&psbt, 4).unwrap()),
            ("bbqr", util::bbqr_parts(&psbt, 4).unwrap()),
        ] {
            assert!(frames.len() >= 4, "{format} split into {} frames", frames.len());
            frames.reverse();

            // a repeated frame isn't a new part
            let missing = frames.pop().unwrap();
            frames.push(frames[0].clone());
            assert_eq!(
                join_frames(frames.clone()).unwrap(),
                json!({ "complete": false, "parts_left": 1 })
            );

            frames.push(missing);
            let joined = join_frames(frames).unwrap();
            assert_eq!(joined["format"], format);
            assert_eq!(joined["psbt"], hex::encode(&psbt));
        }
    }

    #[test]
    fn a_frame_from_another_export_fails_the_join() {
        let mut frames = util::bbqr_parts(&psbt_bytes(), 3).unwrap();
        frames[1] = util::bbqr_parts(b"psbt\xff", 1).unwrap().remove(0);

        assert!(join_frames(frames).is_err());
    }

    #[test]
    fn a_bbqr_part_whose_payload_doesnt_decode_fails_the_join() {
        let parts = base32_parts(&psbt_bytes(), 2);
        assert_eq!(parts.len(), 2);

        // the real parts claim a third, whose payload isn't base32; bbqr would drop it and join the
        // other two into the whole PSBT
        let mut frames: Vec<String> =
            parts.iter().map(|part| format!("{}03{}", &part[..4], &part[6..])).collect();
        frames.push(format!("{}0302!!!!", &parts[0][..4]));

        let error = join_frames(frames).unwrap_err();
        assert!(format!("{error:#}").contains("undecodable payload"), "{error:#}");
    }

    #[test]
    fn a_bbqr_that_isnt_a_psbt_fails_the_join() {
        let frames = base32_parts(b"psbt\xffnot really", 2);

        let error = join_frames(frames).unwrap_err();
        assert!(format!("{error:#}").contains("not a valid PSBT"), "{error:#}");
    }

    #[test]
    fn malformed_bbqr_headers_fail_without_panicking() {
        let valid = util::bbqr_parts(&psbt_bytes(), 1).unwrap().remove(0);
        let payload = &valid[8..];

        for frame in [
            "B$".to_string(),
            "B$ZP01??".to_string(),
            "B$ZP0100".to_string(),
            format!("B$ZP01??{payload}"),
            format!("B$ZP0z00{payload}"),
            format!("B$ZP0000{payload}"),
            format!("B$ZP0101{payload}"),
            format!("B$ZP01é{payload}"),
            format!("B$ZU0100{payload}"),
            format!("B$QP0100{payload}"),
        ] {
            assert!(join_frames(vec![frame.clone()]).is_err(), "`{frame}` joined");
        }
    }

    #[test]
    fn a_ur_whose_message_doesnt_match_its_checksum_fails_the_join() {
        let frames = util::ur_parts(&psbt_bytes(), 3).unwrap();

        // every part claims the same wrong checksum, each re-encoded with valid Bytewords, so only
        // the whole message's checksum can catch it
        let tampered: Vec<String> = frames
            .iter()
            .map(|frame| {
                let ur = UR::parse(frame).unwrap();
                let cbor = bytewords::decode(ur.as_bytewords().unwrap(), Style::Minimal).unwrap();
                let mut part: Part = minicbor::decode(&cbor).unwrap();
                part.checksum ^= 1;

                let fragment = bytewords::encode(&minicbor::to_vec(&part).unwrap(), Style::Minimal);
                format!("ur:crypto-psbt/{}-{}/{fragment}", part.sequence, part.sequence_count)
            })
            .collect();

        let error = join_frames(tampered).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    }
}

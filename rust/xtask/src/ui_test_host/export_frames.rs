//! Joins the QR frames a UI test read off the app's PSBT export, as a hardware
//! wallet's camera would
//!
//! The test decodes each frame it sees in a screenshot and sends every distinct
//! one so far. [`join`] answers how many parts are still missing, or, once the
//! frames are complete, the PSBT they carry, which the test checks against the
//! draft and then signs through `/sign-psbt`
//!
//! The UI tests stand in for a strict signer here, so the join is at least as
//! strict as one: every frame must parse and decode on its own, every UR part
//! must agree on the message it belongs to and the reassembled message must
//! match its checksum, BBQr parts must be split as the spec says and a
//! compressed one must hold a whole deflate stream, and the result must be a
//! PSBT. The `bbqr` and `foundation-ur` crates skip some of these checks, so
//! this module makes them itself, following
//! <https://github.com/coinkite/BBQr/blob/master/BBQr.md> for BBQr

use std::collections::{BTreeMap, BTreeSet};

use bbqr::{encode::Encoding, file_type::FileType, header::Header};
use bitcoin::Psbt;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use cove_ur::CryptoPsbt;
use data_encoding::{BASE32_NOPAD, HEXUPPER};
use flate2::{Decompress, FlushDecompress, Status};
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

/// The deflate window a compressed BBQr uses, as a power of two
const BBQR_WINDOW_BITS: u8 = 10;

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
    let mut sequences = BTreeSet::new();
    let mut message: Option<MessageId> = None;

    for frame in &frames {
        let ur = UR::parse(frame).map_err(|error| eyre!("`{frame}` is not a UR: {error}"))?;
        if ur.as_type() != "crypto-psbt" {
            bail!("`{frame}` is a {}, not a crypto-psbt", ur.as_type());
        }

        // the decoder stops checking parts once it has the message, so a part that names another
        // message would pass if it came late; each one is checked against the first here instead
        let id = MessageId::of(&ur).wrap_err_with(|| format!("`{frame}`"))?;
        let expected = *message.get_or_insert(id);
        if id != expected {
            bail!("`{frame}` belongs to {id:?}, but the first part to {expected:?}");
        }

        sequences.insert(ur.sequence().unwrap_or_default());
        decoder.receive(ur).map_err(|error| eyre!("`{frame}` doesn't join the others: {error}"))?;
    }

    let Some(MessageId { sequence_count: total, checksum: expected, .. }) = message else {
        bail!("No UR frames to join");
    };

    let Some(cbor) = decoder.message().map_err(|error| eyre!("UR decoding failed: {error}"))?
    else {
        // the export shows only the plain fragments 1 to N, so each one missing is a part left
        let parts_left = (1..=total).filter(|sequence| !sequences.contains(sequence)).count();
        return Ok(json!({ "complete": false, "parts_left": parts_left.max(1) }));
    };

    // foundation-ur reassembles the message without checking it against the checksum its parts
    // carry, which every part was checked to agree on above
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

/// What a multi-part UR's fountain part says about the whole message, which every part of one
/// message shares
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MessageId {
    sequence_count: u32,
    message_length: usize,
    checksum: u32,
}

impl MessageId {
    fn of(ur: &UR) -> Result<Self> {
        let Some(fragment) = ur.as_bytewords() else {
            bail!("The UR has no Bytewords payload");
        };

        let cbor = bytewords::decode(fragment, Style::Minimal)
            .map_err(|error| eyre!("Invalid Bytewords: {error}"))?;
        let part: Part =
            minicbor::decode(&cbor).map_err(|error| eyre!("Invalid fountain part: {error}"))?;

        Ok(Self {
            sequence_count: part.sequence_count,
            message_length: part.message_length,
            checksum: part.checksum,
        })
    }
}

/// One BBQr frame, checked on its own
struct BbqrPart {
    /// The header up to the part index, which every part of one split shares
    split: String,
    encoding: Encoding,
    total: usize,
    index: usize,
    payload: String,
    data: Vec<u8>,
}

impl BbqrPart {
    /// Parses and decodes a frame, failing on anything a strict reader would reject
    ///
    /// `bbqr` panics on a malformed part index and drops a part whose payload doesn't decode, so
    /// each part is checked and decoded here, and [`join_bbqr`] joins them without the crate
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

        // data-encoding also rejects base32 whose padding bits aren't zero
        let payload = &frame[BBQR_HEADER_LEN..];
        let decoded = match header.encoding {
            Encoding::Hex => HEXUPPER.decode(payload.as_bytes()),
            Encoding::Base32 | Encoding::Zlib => BASE32_NOPAD.decode(payload.as_bytes()),
        };

        let data =
            decoded.map_err(|error| eyre!("`{frame}` has an undecodable payload: {error}"))?;

        Ok(Self {
            split: frame[..6].to_string(),
            encoding: header.encoding,
            total,
            index,
            payload: payload.to_string(),
            data,
        })
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

    let mut ordered = BTreeMap::new();
    for part in &parts {
        if part.split != first.split {
            bail!("A `{}` part doesn't join a `{}` split", part.split, first.split);
        }

        if let Some(seen) = ordered.insert(part.index, part) {
            if seen.payload != part.payload {
                bail!("Part {} came with two different payloads", part.index);
            }
        }
    }

    if ordered.len() < first.total {
        return Ok(json!({ "complete": false, "parts_left": first.total - ordered.len() }));
    }

    let ordered: Vec<&BbqrPart> = ordered.into_values().collect();
    check_split_lengths(&ordered)?;

    // the length checks make each part's decoded bytes the same as decoding the whole payload at
    // once, as a reader that joins the text before decoding it does
    let encoded: Vec<u8> = ordered.iter().flat_map(|part| part.data.iter().copied()).collect();
    let psbt = match first.encoding {
        Encoding::Hex | Encoding::Base32 => encoded,
        Encoding::Zlib => inflate(&encoded)?,
    };

    Psbt::deserialize(&psbt).wrap_err("The joined BBQr is not a valid PSBT")?;
    Ok(complete("bbqr", &psbt))
}

/// Checks the parts, in order, are split as the BBQr spec says: every part but the last the same
/// length, which for base32 is a whole number of 8-character groups, and the last no longer
///
/// `bbqr` decodes each part on its own and joins the bytes, so it accepts splits that a reader
/// joining the text first would misread
fn check_split_lengths(parts: &[&BbqrPart]) -> Result<()> {
    let Some((last, rest)) = parts.split_last() else {
        bail!("A BBQr split has no parts");
    };

    let Some(first) = rest.first() else {
        return Ok(());
    };

    let length = first.payload.len();
    if let Some(part) = rest.iter().find(|part| part.payload.len() != length) {
        bail!(
            "Part {} is {} characters long, but part 0 is {length}; every part but the last must be \
             the same length",
            part.index,
            part.payload.len()
        );
    }

    if matches!(first.encoding, Encoding::Base32 | Encoding::Zlib) && length % 8 != 0 {
        bail!("Each base32 part but the last is {length} characters long, not a multiple of 8");
    }

    if last.payload.len() > length {
        bail!(
            "The last part is {} characters long, longer than the {length} of the others",
            last.payload.len()
        );
    }

    Ok(())
}

/// Inflates a compressed BBQr's raw deflate stream, failing unless the stream ends exactly where
/// the data does
///
/// `bbqr` reads the stream until its input runs out, so it takes a truncated one as whole
fn inflate(deflated: &[u8]) -> Result<Vec<u8>> {
    let mut inflater = Decompress::new_with_window_bits(false, BBQR_WINDOW_BITS);
    let mut inflated = Vec::with_capacity(deflated.len() * 4);

    loop {
        if inflated.len() == inflated.capacity() {
            inflated.reserve(inflated.capacity());
        }

        let (read, written) = (inflater.total_in(), inflater.total_out());
        let input = &deflated[usize::try_from(read)?..];
        let status = inflater
            .decompress_vec(input, &mut inflated, FlushDecompress::Finish)
            .map_err(|error| eyre!("The compressed BBQr doesn't inflate: {error}"))?;

        if status == Status::StreamEnd {
            break;
        }

        // with room left to write, a step that reads and writes nothing has run out of input
        if inflater.total_in() == read && inflater.total_out() == written {
            bail!("The compressed BBQr ends before its deflate stream does");
        }
    }

    let read = usize::try_from(inflater.total_in())?;
    if read != deflated.len() {
        bail!("The compressed BBQr has {} bytes after its deflate stream", deflated.len() - read);
    }

    Ok(inflated)
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
    use data_encoding::Encoding as TextEncoding;
    use foundation_ur::Encoder;

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

    /// A PSBT BBQr whose parts each hold one of `chunks`, encoded on its own, as a splitter that
    /// cuts the data rather than its text would make them
    fn parts_of_chunks(encoding: char, text: &TextEncoding, chunks: &[&[u8]]) -> Vec<String> {
        chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                format!("B${encoding}P{:02}{index:02}{}", chunks.len(), text.encode(chunk))
            })
            .collect()
    }

    /// The first `count` fountain parts of the PSBT as a 3-part UR, the plain fragments then mixed
    /// ones
    fn ur_frames(count: usize) -> Vec<String> {
        let cbor = CryptoPsbt::from_psbt_bytes(psbt_bytes()).unwrap().encode().unwrap();
        let mut encoder = Encoder::new();
        encoder.start("crypto-psbt", &cbor, cbor.len().div_ceil(3));
        assert_eq!(encoder.sequence_count(), 3);

        (0..count).map(|_| encoder.next_part().to_string()).collect()
    }

    /// The UR fountain part with its message checksum changed, re-encoded as valid Bytewords
    fn with_wrong_checksum(frame: &str) -> String {
        let ur = UR::parse(frame).unwrap();
        let cbor = bytewords::decode(ur.as_bytewords().unwrap(), Style::Minimal).unwrap();
        let mut part: Part = minicbor::decode(&cbor).unwrap();
        part.checksum ^= 1;

        let fragment = bytewords::encode(&minicbor::to_vec(&part).unwrap(), Style::Minimal);
        format!("ur:crypto-psbt/{}-{}/{fragment}", part.sequence, part.sequence_count)
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

        // every part claims the same wrong checksum, so only the whole message's checksum can
        // catch it
        let tampered: Vec<String> = frames.iter().map(|frame| with_wrong_checksum(frame)).collect();

        let error = join_frames(tampered).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    }

    #[test]
    fn mixed_ur_parts_join_and_one_from_another_message_fails_even_after_completion() {
        let frames = ur_frames(30);

        // without the first plain fragment, only the mixed parts after the third can finish it
        let joined = join_frames(frames[1..].to_vec()).unwrap();
        assert_eq!(joined["psbt"], hex::encode(psbt_bytes()));

        // the decoder already has the message when this part comes, so it would ignore it
        let mut conflicting = frames[..3].to_vec();
        conflicting.push(with_wrong_checksum(&frames[5]));

        let error = join_frames(conflicting).unwrap_err();
        assert!(format!("{error:#}").contains("but the first part to"), "{error:#}");
    }

    #[test]
    fn a_truncated_compressed_bbqr_fails_the_join() {
        let frame = util::bbqr_parts(&psbt_bytes(), 1).unwrap().remove(0);
        assert!(frame.starts_with("B$ZP0100"), "{frame}");

        let mut deflated = BASE32_NOPAD.decode(&frame.as_bytes()[8..]).unwrap();
        deflated.pop();
        let truncated = format!("B$ZP0100{}", BASE32_NOPAD.encode(&deflated));

        let error = join_frames(vec![truncated]).unwrap_err();
        assert!(format!("{error:#}").contains("ends before its deflate stream"), "{error:#}");
    }

    #[test]
    fn a_hex_bbqr_with_unequal_parts_fails_the_join() {
        let psbt = psbt_bytes();

        // a short first part, then 40-byte parts, so only the parts before the last differ
        let mut chunks: Vec<&[u8]> = vec![&psbt[..30]];
        chunks.extend(psbt[30..].chunks(40));
        let error = join_frames(parts_of_chunks('H', &HEXUPPER, &chunks)).unwrap_err();
        assert!(format!("{error:#}").contains("the same length"), "{error:#}");

        let frames = parts_of_chunks('H', &HEXUPPER, &[&psbt[..40], &psbt[40..]]);
        let error = join_frames(frames).unwrap_err();
        assert!(format!("{error:#}").contains("longer than"), "{error:#}");
    }

    #[test]
    fn a_base32_bbqr_split_off_its_8_character_groups_fails_the_join() {
        // 33 bytes is 52.8 base32 characters, so each part ends mid-group; joining the text first
        // would read different bytes than decoding each part
        let psbt = psbt_bytes();
        let chunks: Vec<&[u8]> = psbt.chunks(33).collect();

        let error = join_frames(parts_of_chunks('2', &BASE32_NOPAD, &chunks)).unwrap_err();
        assert!(format!("{error:#}").contains("not a multiple of 8"), "{error:#}");
    }
}

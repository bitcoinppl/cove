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
//! must be a valid fountain part that agrees on the message it belongs to and
//! matches the reassembled message, which must match its checksum, BBQr parts
//! must be split as the spec says and a compressed one must hold a whole
//! deflate stream, and the result must be one PSBT with nothing after it. The `bbqr` and `foundation-ur` crates skip some of these checks, so
//! this module makes them itself, following
//! <https://github.com/coinkite/BBQr/blob/master/BBQr.md> for BBQr

use std::collections::{BTreeMap, BTreeSet};

use bbqr::{encode::Encoding, file_type::FileType, header::Header};
use bitcoin::Psbt;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use cove_ur::registry::CRYPTO_PSBT;
use data_encoding::{BASE32_NOPAD, HEXUPPER};
use flate2::{Decompress, FlushDecompress, Status};
use foundation_ur::{
    bytewords::{self, Style},
    fountain::{chooser::Alloc, part::Part},
    Decoder, UR,
};
use minicbor::data::{Tag, Type};
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
    // the tests send every capture of a frame, and a UR is case-insensitive
    let mut seen = BTreeSet::new();
    let frames: Vec<String> = frames
        .iter()
        .map(|frame| frame.to_ascii_lowercase())
        .filter(|frame| seen.insert(frame.clone()))
        .collect();

    if let [single] = frames.as_slice() {
        if UR::parse(single).is_ok_and(|ur| ur.is_single_part()) {
            return join_single_ur(single);
        }
    }

    let mut decoder = Decoder::default();
    let mut parts = BTreeMap::new();
    let mut message: Option<MessageId> = None;

    for frame in &frames {
        let ur = crypto_psbt_ur(frame)?;
        let part = FountainPart::of(&ur).wrap_err_with(|| format!("`{frame}`"))?;

        // the decoder stops checking parts once it has the message, so a part that names another
        // message would pass if it came late; each one is checked against the first here instead
        let expected = *message.get_or_insert(part.message);
        if part.message != expected {
            bail!("`{frame}` belongs to {:?}, but the first part to {expected:?}", part.message);
        }

        if let Some(seen) = parts.get(&part.sequence) {
            if seen != &part {
                bail!("`{frame}` is a second part {} with other data", part.sequence);
            }

            continue;
        }

        decoder.receive(ur).map_err(|error| eyre!("`{frame}` doesn't join the others: {error}"))?;
        parts.insert(part.sequence, part);
    }

    let Some(message) = message else {
        bail!("No UR frames to join");
    };

    let Some(cbor) = decoder.message().map_err(|error| eyre!("UR decoding failed: {error}"))?
    else {
        // the export shows only the plain fragments 1 to N, so each one missing is a part left
        let parts_left =
            (1..=message.sequence_count).filter(|sequence| !parts.contains_key(sequence)).count();
        return Ok(json!({ "complete": false, "parts_left": parts_left.max(1) }));
    };

    // foundation-ur reassembles the message without checking it against the checksum its parts
    // carry, which every part was checked to agree on above
    let actual = CRC32.checksum(cbor);
    if actual != message.checksum {
        bail!(
            "The joined UR message has checksum {actual:08x}, but its parts carry {:08x}",
            message.checksum
        );
    }

    // the decoder ignores the parts that come after it has the message, and a part it took may
    // carry fragments that a reduction never read, so each part is checked against the message
    let fragments = message.fragments(cbor);
    for part in parts.values() {
        if part.expected_data(&fragments) != part.data {
            bail!("Part {} doesn't carry the message the other parts join to", part.sequence);
        }
    }

    let psbt = psbt_from_cbor(cbor)?;
    Ok(complete("ur", psbt))
}

fn crypto_psbt_ur(frame: &str) -> Result<UR<'_>> {
    let ur = UR::parse(frame).map_err(|error| eyre!("`{frame}` is not a UR: {error}"))?;
    if ur.as_type() != "crypto-psbt" {
        bail!("`{frame}` is a {}, not a crypto-psbt", ur.as_type());
    }

    Ok(ur)
}

fn join_single_ur(frame: &str) -> Result<Value> {
    let ur = crypto_psbt_ur(frame)?;
    let Some(message) = ur.as_bytewords() else {
        bail!("`{frame}` has no Bytewords payload");
    };

    let cbor = bytewords::decode(message, Style::Minimal)
        .map_err(|error| eyre!("`{frame}` has invalid Bytewords: {error}"))?;

    let psbt = psbt_from_cbor(&cbor)?;
    Ok(complete("ur", psbt))
}

/// The PSBT in a `crypto-psbt` message, which must be one CBOR byte string, tagged or not, with
/// nothing after it
///
/// `cove-ur` reads the byte string and ignores anything after it
fn psbt_from_cbor(cbor: &[u8]) -> Result<&[u8]> {
    let mut decoder = minicbor::Decoder::new(cbor);
    let invalid = |error: minicbor::decode::Error| eyre!("The UR is not a crypto-psbt: {error}");

    if decoder.datatype().map_err(invalid)? == Type::Tag {
        let tag = decoder.tag().map_err(invalid)?;
        if tag != Tag::new(CRYPTO_PSBT) {
            bail!("The UR message has tag {}, not crypto-psbt's {CRYPTO_PSBT}", tag.as_u64());
        }
    }

    let psbt = decoder.bytes().map_err(invalid)?;
    if decoder.position() != cbor.len() {
        bail!("The UR message has {} bytes after its PSBT", cbor.len() - decoder.position());
    }

    check_psbt("UR", psbt)?;
    Ok(psbt)
}

/// Checks the bytes are one PSBT with nothing after it, which `Psbt::deserialize` doesn't check
fn check_psbt(format: &str, bytes: &[u8]) -> Result<()> {
    let mut rest = bytes;
    Psbt::deserialize_from_reader(&mut rest)
        .wrap_err_with(|| format!("The joined {format} is not a valid PSBT"))?;

    if !rest.is_empty() {
        bail!("The joined {format} has {} bytes after the PSBT", rest.len());
    }

    Ok(())
}

/// What a multi-part UR's fountain part says about the whole message, which every part of one
/// message shares
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MessageId {
    sequence_count: u32,
    message_length: usize,
    checksum: u32,
    fragment_length: usize,
}

impl MessageId {
    /// The message, zero-padded to whole fragments, cut into its fragments
    fn fragments(&self, message: &[u8]) -> Vec<Vec<u8>> {
        let mut padded = message.to_vec();
        padded.resize(self.fragment_length * self.sequence_count as usize, 0);

        padded.chunks(self.fragment_length).map(<[u8]>::to_vec).collect()
    }
}

/// One multi-part UR's fountain part, checked on its own
#[derive(Debug, PartialEq, Eq)]
struct FountainPart {
    sequence: u32,
    message: MessageId,
    data: Vec<u8>,
}

impl FountainPart {
    /// Decodes the part a UR carries, failing on one a strict decoder would reject
    ///
    /// foundation-ur checks a part is valid only until it has the message, so this checks every one
    fn of(ur: &UR) -> Result<Self> {
        let Some(fragment) = ur.as_bytewords() else {
            bail!("The UR has no Bytewords payload");
        };

        let cbor = bytewords::decode(fragment, Style::Minimal)
            .map_err(|error| eyre!("Invalid Bytewords: {error}"))?;

        let mut decoder = minicbor::Decoder::new(&cbor);
        let part: Part =
            decoder.decode().map_err(|error| eyre!("Invalid fountain part: {error}"))?;

        if decoder.position() != cbor.len() {
            bail!("The fountain part has {} bytes after it", cbor.len() - decoder.position());
        }

        if !part.is_valid() {
            bail!(
                "Invalid fountain part: sequence {} of {}, {} bytes of a {}-byte message",
                part.sequence,
                part.sequence_count,
                part.data.len(),
                part.message_length
            );
        }

        Ok(Self {
            sequence: part.sequence,
            message: MessageId {
                sequence_count: part.sequence_count,
                message_length: part.message_length,
                checksum: part.checksum,
                fragment_length: part.data.len(),
            },
            data: part.data.to_vec(),
        })
    }

    /// The data this part carries for the message: one fragment for a plain part, the XOR of the
    /// fragments the fountain chooser picks for a mixed one
    fn expected_data(&self, fragments: &[Vec<u8>]) -> Vec<u8> {
        let part = Part {
            sequence: self.sequence,
            sequence_count: self.message.sequence_count,
            message_length: self.message.message_length,
            checksum: self.message.checksum,
            data: &self.data,
        };

        let mut data = vec![0; self.message.fragment_length];
        for index in part.indexes::<Alloc, BTreeSet<usize>>() {
            for (byte, fragment_byte) in data.iter_mut().zip(&fragments[index]) {
                *byte ^= fragment_byte;
            }
        }

        data
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

    check_psbt("BBQr", &psbt)?;
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
    use cove_ur::CryptoPsbt;
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

    /// The UR fountain part after `edit`, re-encoded as valid Bytewords
    fn edited_part(frame: &str, edit: impl FnOnce(&mut Part, &mut Vec<u8>)) -> String {
        let ur = UR::parse(frame).unwrap();
        let cbor = bytewords::decode(ur.as_bytewords().unwrap(), Style::Minimal).unwrap();
        let mut part: Part = minicbor::decode(&cbor).unwrap();
        let mut data = part.data.to_vec();
        edit(&mut part, &mut data);

        let part = Part { data: &data, ..part };
        let fragment = bytewords::encode(&minicbor::to_vec(&part).unwrap(), Style::Minimal);
        format!("ur:crypto-psbt/{}-{}/{fragment}", part.sequence, part.sequence_count)
    }

    /// The UR fountain part with a byte after its CBOR, inside valid Bytewords
    fn with_bytes_after_the_part(frame: &str) -> String {
        let ur = UR::parse(frame).unwrap();
        let mut cbor = bytewords::decode(ur.as_bytewords().unwrap(), Style::Minimal).unwrap();
        cbor.push(0);

        let fragment = bytewords::encode(&cbor, Style::Minimal);
        format!(
            "ur:crypto-psbt/{}-{}/{fragment}",
            ur.sequence().unwrap(),
            ur.sequence_count().unwrap()
        )
    }

    fn with_wrong_checksum(frame: &str) -> String {
        edited_part(frame, |part, _| part.checksum ^= 1)
    }

    /// A single-part `crypto-psbt` UR of the CBOR message
    fn single_part_ur(cbor: &[u8]) -> String {
        UR::new("crypto-psbt", cbor).to_string()
    }

    /// The PSBT's `crypto-psbt` message with `extra` after its byte string, and the byte string
    /// itself holding the PSBT with `extra` after it
    fn cbor_with_extra_bytes(extra: &[u8]) -> [Vec<u8>; 2] {
        let cbor = CryptoPsbt::from_psbt_bytes(psbt_bytes()).unwrap().encode().unwrap();
        let after_cbor = [cbor.as_slice(), extra].concat();

        let mut after_psbt = minicbor::Encoder::new(Vec::new());
        after_psbt.bytes(&[psbt_bytes().as_slice(), extra].concat()).unwrap();

        [after_cbor, after_psbt.into_writer()]
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

    #[test]
    fn an_invalid_ur_part_fails_the_join_before_or_after_completion() {
        let frames = ur_frames(3);

        for (bad, after_completion) in [
            (edited_part(&frames[1], |_, data| data.clear()), "Invalid fountain part"),
            (edited_part(&frames[1], |_, data| data.push(0)), "but the first part to"),
            (edited_part(&frames[1], |part, _| part.sequence = 0), "Invalid fountain part"),
            (with_bytes_after_the_part(&frames[1]), "bytes after it"),
        ] {
            let mut first = vec![bad.clone()];
            first.extend(frames.iter().cloned());
            assert!(join_frames(first).is_err(), "`{bad}` joined as the first part");

            let mut last = frames.clone();
            last.push(bad.clone());
            let error = join_frames(last).unwrap_err();
            assert!(format!("{error:#}").contains(after_completion), "{error:#}");
        }
    }

    #[test]
    fn a_ur_part_with_the_wrong_data_fails_even_after_completion() {
        let frames = ur_frames(6);
        let flip_first_byte = |_: &mut Part, data: &mut Vec<u8>| data[0] ^= 1;

        // a plain part repeated with other data, and a mixed part the decoder no longer reads
        for (bad, expected) in [
            (edited_part(&frames[1], flip_first_byte), "a second part 2 with other data"),
            (edited_part(&frames[5], flip_first_byte), "doesn't carry the message"),
        ] {
            let mut late = frames[..3].to_vec();
            late.push(bad);

            let error = join_frames(late).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }

    #[test]
    fn a_repeated_single_part_ur_joins() {
        let single = CryptoPsbt::from_psbt_bytes(psbt_bytes()).unwrap().to_ur_string().unwrap();

        let joined = join_frames(vec![single.clone(), single.clone(), single.to_uppercase()]);
        assert_eq!(joined.unwrap()["psbt"], hex::encode(psbt_bytes()));
    }

    #[test]
    fn bytes_after_the_psbt_fail_the_join() {
        let [after_cbor, after_psbt] = cbor_with_extra_bytes(&[0]);

        let error = join_frames(vec![single_part_ur(&after_cbor)]).unwrap_err();
        assert!(format!("{error:#}").contains("bytes after its PSBT"), "{error:#}");

        let error = join_frames(vec![single_part_ur(&after_psbt)]).unwrap_err();
        assert!(format!("{error:#}").contains("bytes after the PSBT"), "{error:#}");

        let frames = base32_parts(&[psbt_bytes().as_slice(), &[0]].concat(), 2);
        let error = join_frames(frames).unwrap_err();
        assert!(format!("{error:#}").contains("bytes after the PSBT"), "{error:#}");
    }
}

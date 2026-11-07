//! Joins the QR frames a UI test read off the app's PSBT export, as a hardware
//! wallet's camera would
//!
//! The test decodes each frame it sees in a screenshot and sends every distinct
//! one so far. [`join`] answers how many parts are still missing, or, once the
//! frames are complete, the PSBT they carry, which the test checks against the
//! draft and then signs through `/sign-psbt`

use std::collections::BTreeSet;

use bbqr::continuous_join::{ContinuousJoinResult, ContinuousJoiner};
use bbqr::file_type::FileType;
use color_eyre::{
    eyre::{bail, eyre},
    Result,
};
use cove_ur::CryptoPsbt;
use foundation_ur::{Decoder, UR};
use serde::Deserialize;
use serde_json::{json, Value};

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
        join_bbqr(request.frames)
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
    let mut total = 0;

    for frame in &frames {
        let ur = UR::parse(frame).map_err(|error| eyre!("`{frame}` is not a UR: {error}"))?;
        if ur.as_type() != "crypto-psbt" {
            bail!("`{frame}` is a {}, not a crypto-psbt", ur.as_type());
        }

        sequences.extend(ur.sequence());
        total = ur.sequence_count().unwrap_or_default();
        decoder.receive(ur).map_err(|error| eyre!("`{frame}` doesn't join the others: {error}"))?;
    }

    let Some(cbor) = decoder.message().map_err(|error| eyre!("UR decoding failed: {error}"))?
    else {
        // the export shows only the plain fragments 1 to N, so each one missing is a part left
        let parts_left = (1..=total).filter(|sequence| !sequences.contains(sequence)).count();
        return Ok(json!({ "complete": false, "parts_left": parts_left.max(1) }));
    };

    let psbt = CryptoPsbt::decode(cbor.to_vec())
        .map_err(|error| eyre!("The UR is not a crypto-psbt: {error}"))?;
    Ok(complete("ur", &psbt.to_psbt_bytes()))
}

fn join_bbqr(frames: Vec<String>) -> Result<Value> {
    let mut joiner = ContinuousJoiner::new();
    let mut result = ContinuousJoinResult::NotStarted;

    for frame in frames {
        let shown = frame.clone();
        result = joiner
            .add_part(frame)
            .map_err(|error| eyre!("`{shown}` doesn't join the others: {error}"))?;
    }

    match result {
        ContinuousJoinResult::Complete(joined) if joined.file_type == FileType::Psbt => {
            Ok(complete("bbqr", &joined.data))
        }
        ContinuousJoinResult::Complete(joined) => {
            Err(eyre!("The BBQr holds a {:?}, not a PSBT", joined.file_type))
        }
        ContinuousJoinResult::InProgress { parts_left } => {
            Ok(json!({ "complete": false, "parts_left": parts_left }))
        }
        ContinuousJoinResult::NotStarted => Err(eyre!("No BBQr frame parsed")),
    }
}

fn complete(format: &str, psbt: &[u8]) -> Value {
    json!({ "complete": true, "format": format, "psbt": hex::encode(psbt) })
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime, hashes::Hash as _, transaction, Amount, OutPoint, Psbt, ScriptBuf,
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

    fn join_frames(frames: Vec<String>) -> Value {
        join(JoinRequest { frames }).unwrap()
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
            assert_eq!(join_frames(frames.clone()), json!({ "complete": false, "parts_left": 1 }));

            frames.push(missing);
            let joined = join_frames(frames);
            assert_eq!(joined["format"], format);
            assert_eq!(joined["psbt"], hex::encode(&psbt));
        }
    }

    #[test]
    fn a_frame_from_another_export_fails_the_join() {
        let mut frames = util::bbqr_parts(&psbt_bytes(), 3).unwrap();
        frames[1] = util::bbqr_parts(b"psbt\xff", 1).unwrap().remove(0);

        assert!(join(JoinRequest { frames }).is_err());
    }
}

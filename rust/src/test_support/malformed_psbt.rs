//! PSBTs the parser rejects with errors that quote their bytes
//!
//! Each fixture comes with the text the raw parser error would contain, so a privacy test can
//! check both that the fixture really makes the parser echo it and that Cove's error and logs
//! do not

use bitcoin::hashes::{Hash as _, sha256};

pub(crate) struct MalformedPsbt {
    pub(crate) bytes: Vec<u8>,

    /// Text from the payload that the raw `bitcoin` parser error contains
    pub(crate) echoed_payload: &'static str,
}

impl MalformedPsbt {
    /// A SHA256 preimage that does not hash to its key, the parser error lists every preimage byte
    pub(crate) fn preimage_mismatch() -> Self {
        let preimage = [0xab; 20];
        let wrong_hash = sha256::Hash::hash(b"not the preimage");

        // key type 0x0b is PSBT_IN_SHA256, keyed by the hash and valued by the preimage
        let mut key = vec![0x0b];
        key.extend_from_slice(wrong_hash.as_byte_array());

        Self { bytes: psbt_with_input_pair(&key, &preimage), echoed_payload: "171, 171" }
    }

    /// Key data on a key type that takes none, the parser error prints the key bytes as hex
    pub(crate) fn invalid_key() -> Self {
        // key type 0x00 in an input map is PSBT_IN_NON_WITNESS_UTXO, which has no key data
        let key = [0x00, 0xde, 0xad, 0xbe, 0xef];
        Self { bytes: psbt_with_input_pair(&key, &[0x00]), echoed_payload: "deadbeef" }
    }

    pub(crate) fn all() -> [Self; 2] {
        [Self::preimage_mismatch(), Self::invalid_key()]
    }

    /// Fails the test unless the raw parser error really quotes the payload
    pub(crate) fn assert_parser_echoes_payload(&self) {
        let raw_error = bitcoin::Psbt::deserialize(&self.bytes).expect_err("fixture is malformed");
        assert!(
            raw_error.to_string().contains(self.echoed_payload),
            "fixture must make the parser echo its payload: {raw_error}"
        );
    }

    /// Fails the test if `text` contains the payload
    pub(crate) fn assert_absent_from(&self, label: &str, text: &str) {
        assert!(!text.contains(self.echoed_payload), "{label} leaks the PSBT payload: {text}");
    }
}

// one input and no outputs, with the given pair as the only entry in the input map
fn psbt_with_input_pair(key: &[u8], value: &[u8]) -> Vec<u8> {
    // version 2, one input spending 00..00:0 with an empty script sig, no outputs, locktime 0
    let mut unsigned_tx = vec![0x02, 0x00, 0x00, 0x00, 0x01];
    unsigned_tx.extend_from_slice(&[0x00; 36]);
    unsigned_tx.push(0x00);
    unsigned_tx.extend_from_slice(&[0xff; 4]);
    unsigned_tx.push(0x00);
    unsigned_tx.extend_from_slice(&[0x00; 4]);

    let mut bytes = b"psbt\xff".to_vec();
    push_pair(&mut bytes, &[0x00], &unsigned_tx);
    bytes.push(0x00);
    push_pair(&mut bytes, key, value);
    bytes.push(0x00);
    bytes
}

fn push_pair(bytes: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    bytes.push(u8::try_from(key.len()).expect("short key"));
    bytes.extend_from_slice(key);
    bytes.push(u8::try_from(value.len()).expect("short value"));
    bytes.extend_from_slice(value);
}

use std::{fmt, str::FromStr};

use bip39::Mnemonic;
use bitcoin::{
    NetworkKind,
    bip32::{ChildNumber, Fingerprint, Xpriv},
    secp256k1::SecretKey,
};
use serde::{Deserialize, Deserializer};
use zeroize::{Zeroize, Zeroizing};

use crate::{Error, Result};

const MAINNET_XPRV_VERSION: [u8; 4] = [0x04, 0x88, 0xad, 0xe4];

// KeyTeleport payload type codes (first byte of the decrypted body)
const PAYLOAD_CODE_STASH: u8 = b's';
const PAYLOAD_CODE_XPRV: u8 = b'x';
const PAYLOAD_CODE_NOTES: u8 = b'n';
const PAYLOAD_CODE_VAULT: u8 = b'v';
const PAYLOAD_CODE_PSBT: u8 = b'p';
const PAYLOAD_CODE_BACKUP: u8 = b'b';

// COLDCARD 72-byte stash layout (payload type `s`): first body byte is a marker
//
// - 0x01: master xprv as chain_code || private_key
// - 0x10..=0x40: raw BIP32 master secret; marker is the secret length in bytes
// - high bit set: BIP39 entropy; low bits encode length as ((marker & 0x03) + 2) * 8
const STASH_LEN: usize = 72;
const STASH_MARKER_XPRV: u8 = 0x01;
const STASH_MARKER_MNEMONIC_FLAG: u8 = 0x80;
const STASH_MNEMONIC_ENTROPY_UNITS_MASK: u8 = 0x03;
const STASH_RAW_MASTER_SECRET_LEN: std::ops::RangeInclusive<u8> = 0x10..=0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedPayloadKind {
    Vault,
    Psbt,
    Backup,
    Unknown(u8),
}

impl fmt::Display for UnsupportedPayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vault => write!(f, "{}", PAYLOAD_CODE_VAULT as char),
            Self::Psbt => write!(f, "{}", PAYLOAD_CODE_PSBT as char),
            Self::Backup => write!(f, "{}", PAYLOAD_CODE_BACKUP as char),
            Self::Unknown(code) => write!(f, "0x{code:02x}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum PayloadKind {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

/// A secret payload that can be transferred by COLDCARD KeyTeleport
#[derive(Clone, PartialEq, Eq)]
pub struct Payload(PayloadKind);

impl Payload {
    /// Creates a mnemonic payload when its word count is supported by COLDCARD
    pub fn mnemonic(mnemonic: Mnemonic) -> Result<Self> {
        let word_count = mnemonic.word_count();
        if !matches!(word_count, 12 | 18 | 24) {
            return Err(Error::UnsupportedMnemonicWordCount(word_count));
        }

        Ok(Self(PayloadKind::Mnemonic(mnemonic)))
    }

    /// Creates an xprv payload
    pub fn xprv(value: impl AsRef<str>) -> Result<Self> {
        Ok(Self(PayloadKind::Xprv(XprvPayload::parse(value.as_ref())?)))
    }

    pub(crate) fn encode(&self) -> Result<Zeroizing<Vec<u8>>> {
        match &self.0 {
            PayloadKind::Mnemonic(mnemonic) => encode_mnemonic_payload(mnemonic),
            PayloadKind::Xprv(xprv) => encode_xprv_payload(xprv),
        }
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            PayloadKind::Mnemonic(_) => f.write_str("Payload::Mnemonic(****)"),
            PayloadKind::Xprv(_) => f.write_str("Payload::Xprv(****)"),
        }
    }
}

/// A validated master extended private key transferred by KeyTeleport
#[derive(Clone, PartialEq, Eq)]
pub struct XprvPayload {
    value: String,
}

impl XprvPayload {
    /// Parses and validates a master extended private key
    ///
    /// # Errors
    ///
    /// Returns an error when the value is invalid, is not a mainnet key, or represents a derived
    /// child key
    pub fn parse(value: &str) -> Result<Self> {
        let xprv = Xpriv::from_str(value).map_err(|_| Error::InvalidXprvPayload)?;
        if xprv.network != NetworkKind::Main {
            return Err(Error::NonMainnetXprvPayload);
        }

        if !is_master_xprv(&xprv) {
            return Err(Error::NonMasterXprvPayload);
        }

        Ok(Self { value: xprv.to_string() })
    }

    /// Exposes the encoded extended private key
    pub fn expose_string(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for XprvPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("XprvPayload(****)")
    }
}

impl Drop for XprvPayload {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

/// A decoded collection of COLDCARD Secure Notes & Passwords records
#[derive(Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct NotesPayload(Vec<NotesRecord>);

impl NotesPayload {
    /// Returns the decoded records in their transmitted order
    pub fn records(&self) -> &[NotesRecord] {
        &self.0
    }
}

impl fmt::Debug for NotesPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotesPayload").field("record_count", &self.0.len()).finish()
    }
}

/// A decoded COLDCARD secure note or password record
#[derive(Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub enum NotesRecord {
    /// A free-form secure note
    Note(NoteRecord),
    /// A structured password record
    Password(PasswordRecord),
}

impl NotesRecord {
    /// Returns the title shown for this record
    pub fn title(&self) -> &str {
        match self {
            Self::Note(note) => note.title(),
            Self::Password(password) => password.title(),
        }
    }

    /// Returns the optional group shown for this record
    pub fn group(&self) -> &str {
        match self {
            Self::Note(note) => note.group(),
            Self::Password(password) => password.group(),
        }
    }
}

impl fmt::Debug for NotesRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Note(_) => f.write_str("NotesRecord::Note(****)"),
            Self::Password(_) => f.write_str("NotesRecord::Password(****)"),
        }
    }
}

/// A decoded COLDCARD free-form secure note
#[derive(Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct NoteRecord {
    title: String,
    text: String,
    group: String,
}

impl NoteRecord {
    /// Returns the note title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the free-form note text
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the optional group, or an empty string when absent
    pub fn group(&self) -> &str {
        &self.group
    }
}

impl fmt::Debug for NoteRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoteRecord(****)")
    }
}

/// A decoded COLDCARD structured password record
#[derive(Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct PasswordRecord {
    title: String,
    username: String,
    password: String,
    site: String,
    notes: String,
    group: String,
}

impl PasswordRecord {
    /// Returns the password record title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the username, or an empty string when absent
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the password, or an empty string when absent
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Returns the site, or an empty string when absent
    pub fn site(&self) -> &str {
        &self.site
    }

    /// Returns the free-form notes, or an empty string when absent
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// Returns the optional group, or an empty string when absent
    pub fn group(&self) -> &str {
        &self.group
    }
}

impl fmt::Debug for PasswordRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordRecord(****)")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNotesRecord {
    title: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    user: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    password: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    site: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    misc: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    group: Option<String>,
}

impl Drop for WireNotesRecord {
    fn drop(&mut self) {
        self.title.zeroize();
        self.user.zeroize();
        self.password.zeroize();
        self.site.zeroize();
        self.misc.zeroize();
        self.group.zeroize();
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum DecodedPayload {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
    /// COLDCARD Secure Notes & Passwords records
    Notes(NotesPayload),
}

impl DecodedPayload {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        let (&code, body) = bytes.split_first().ok_or(Error::InvalidPacket)?;

        match code {
            PAYLOAD_CODE_STASH => decode_stash_payload(body),
            PAYLOAD_CODE_XPRV => decode_xprv_body(body),
            PAYLOAD_CODE_NOTES => decode_notes_body(body),
            PAYLOAD_CODE_VAULT => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Vault)),
            PAYLOAD_CODE_PSBT => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Psbt)),
            PAYLOAD_CODE_BACKUP => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Backup)),
            other => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Unknown(other))),
        }
    }
}

impl fmt::Debug for DecodedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mnemonic(_) => f.write_str("DecodedPayload::Mnemonic(****)"),
            Self::Xprv(_) => f.write_str("DecodedPayload::Xprv(****)"),
            Self::Notes(_) => f.write_str("DecodedPayload::Notes(****)"),
        }
    }
}

fn encode_mnemonic_payload(mnemonic: &Mnemonic) -> Result<Zeroizing<Vec<u8>>> {
    let entropy = Zeroizing::new(mnemonic.to_entropy());
    if !matches!(entropy.len(), 16 | 24 | 32) {
        return Err(Error::UnsupportedMnemonicWordCount(mnemonic.word_count()));
    }

    let marker = STASH_MARKER_MNEMONIC_FLAG | ((entropy.len() / 8) - 2) as u8;
    let mut encoded = Zeroizing::new(Vec::with_capacity(1 + 1 + entropy.len()));
    encoded.push(PAYLOAD_CODE_STASH);
    encoded.push(marker);
    encoded.extend_from_slice(&entropy);
    trim_stash_padding(&mut encoded);

    Ok(encoded)
}

fn encode_xprv_payload(xprv: &XprvPayload) -> Result<Zeroizing<Vec<u8>>> {
    let xprv = Xpriv::from_str(&xprv.value).map_err(|_| Error::InvalidXprvPayload)?;
    let private_key = Zeroizing::new(xprv.private_key.secret_bytes());

    let mut encoded = Zeroizing::new(Vec::with_capacity(66));
    encoded.push(PAYLOAD_CODE_STASH);
    encoded.push(STASH_MARKER_XPRV);
    encoded.extend_from_slice(xprv.chain_code.as_bytes());
    encoded.extend_from_slice(private_key.as_ref());
    trim_stash_padding(&mut encoded);

    Ok(encoded)
}

fn is_master_xprv(xprv: &Xpriv) -> bool {
    xprv.depth == 0
        && xprv.parent_fingerprint == Fingerprint::default()
        && xprv.child_number == ChildNumber::Normal { index: 0 }
}

fn decode_stash_payload(body: &[u8]) -> Result<DecodedPayload> {
    if body.is_empty() || body.len() > STASH_LEN {
        return Err(Error::InvalidMnemonicPayload);
    }

    // COLDCARD strips trailing zeroes from its 72-byte stash before transport
    let mut stash = Zeroizing::new([0_u8; STASH_LEN]);
    stash[..body.len()].copy_from_slice(body);
    let marker = stash[0];
    let rest = &stash[1..];

    if marker == STASH_MARKER_XPRV {
        return decode_stash_xprv(rest);
    }

    // COLDCARD raw BIP32 master secret: marker is the byte length (16-64),
    // body is the raw seed, and the wallet key is the BIP32 master derived
    // from it (HMAC-SHA512 "Bitcoin seed"), matching COLDCARD's hd.from_master
    if STASH_RAW_MASTER_SECRET_LEN.contains(&marker) {
        let xprv = Xpriv::new_master(NetworkKind::Main, &rest[..usize::from(marker)])
            .map_err(|_| Error::InvalidXprvPayload)?;
        return Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }));
    }

    if marker & STASH_MARKER_MNEMONIC_FLAG == 0 {
        return Err(Error::InvalidMnemonicPayload);
    }

    let entropy_len = usize::from((marker & STASH_MNEMONIC_ENTROPY_UNITS_MASK) + 2) * 8;
    if !matches!(entropy_len, 16 | 24 | 32) || rest.len() < entropy_len {
        return Err(Error::InvalidMnemonicPayload);
    }

    let mnemonic = Mnemonic::from_entropy(&rest[..entropy_len])?;

    Ok(DecodedPayload::Mnemonic(mnemonic))
}

fn decode_stash_xprv(body: &[u8]) -> Result<DecodedPayload> {
    if body.len() != 71 {
        return Err(Error::InvalidXprvPayload);
    }

    let chain_code = &body[..32];
    let private_key = &body[32..64];
    SecretKey::from_slice(private_key).map_err(|_| Error::InvalidXprvPayload)?;

    let mut encoded = Zeroizing::new([0_u8; 78]);
    encoded[0..4].copy_from_slice(&MAINNET_XPRV_VERSION);
    encoded[13..45].copy_from_slice(chain_code);
    encoded[45] = 0;
    encoded[46..78].copy_from_slice(private_key);

    let xprv = Xpriv::decode(&encoded[..])?;

    Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }))
}

fn trim_stash_padding(encoded: &mut Vec<u8>) {
    while encoded.last() == Some(&0) {
        encoded.pop();
    }
}

fn decode_xprv_body(body: &[u8]) -> Result<DecodedPayload> {
    let xprv = Xpriv::decode(body).map_err(|_| Error::InvalidXprvPayload)?;
    if xprv.network != NetworkKind::Main {
        return Err(Error::NonMainnetXprvPayload);
    }

    let master = Xpriv {
        network: xprv.network,
        depth: 0,
        parent_fingerprint: Fingerprint::default(),
        child_number: ChildNumber::Normal { index: 0 },
        private_key: xprv.private_key,
        chain_code: xprv.chain_code,
    };

    Ok(DecodedPayload::Xprv(XprvPayload { value: master.to_string() }))
}

fn decode_notes_body(body: &[u8]) -> Result<DecodedPayload> {
    let mut records: Vec<WireNotesRecord> =
        serde_json::from_slice(body).map_err(|_| Error::InvalidNotesPayload)?;
    if records.is_empty() {
        return Err(Error::InvalidNotesPayload);
    }

    let records = records.iter_mut().map(NotesRecord::try_from).collect::<Result<Vec<_>>>()?;

    Ok(DecodedPayload::Notes(NotesPayload(records)))
}

impl TryFrom<&mut WireNotesRecord> for NotesRecord {
    type Error = Error;

    fn try_from(record: &mut WireNotesRecord) -> Result<Self> {
        if record.title.is_empty() {
            return Err(Error::InvalidNotesPayload);
        }

        let is_password = record.user.is_some();
        if !is_password && (record.password.is_some() || record.site.is_some()) {
            return Err(Error::InvalidNotesPayload);
        }

        let group = record.group.take().unwrap_or_default();
        if let Some(username) = record.user.take() {
            return Ok(Self::Password(PasswordRecord {
                title: std::mem::take(&mut record.title),
                username,
                password: record.password.take().unwrap_or_default(),
                site: record.site.take().unwrap_or_default(),
                notes: record.misc.take().unwrap_or_default(),
                group,
            }));
        }

        Ok(Self::Note(NoteRecord {
            title: std::mem::take(&mut record.title),
            text: record.misc.take().unwrap_or_default(),
            group,
        }))
    }
}

fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XPRV: &str = "xprv9s21ZrQH143K4BwRCYKSEPwcAMYweWkfKLURabnnv2GLNhJN1LSCgDQyGWyNcat72najQKwyshCBXWfHHVbcdxPAZPqByMyWDbWp5SjCfEa";

    #[test]
    fn mnemonic_stash_roundtrips_coldcard_trailing_zero_trimming() {
        let mnemonic = Mnemonic::from_entropy(&[0_u8; 16]).unwrap();
        let encoded = Payload::mnemonic(mnemonic.clone()).unwrap().encode().unwrap();

        assert_eq!(encoded.as_slice(), &[PAYLOAD_CODE_STASH, STASH_MARKER_MNEMONIC_FLAG]);
        assert_eq!(DecodedPayload::decode(&encoded).unwrap(), DecodedPayload::Mnemonic(mnemonic));
    }

    #[test]
    fn xprv_encoder_uses_coldcard_stash_layout() {
        let xprv = Xpriv::from_str(XPRV).unwrap();
        let encoded = Payload::xprv(XPRV).unwrap().encode().unwrap();

        assert_eq!(&encoded[..2], &[PAYLOAD_CODE_STASH, STASH_MARKER_XPRV]);
        assert_eq!(&encoded[2..34], xprv.chain_code.as_bytes());
        assert_eq!(&encoded[34..66], &xprv.private_key.secret_bytes());
    }

    #[test]
    fn xprv_stash_decodes_after_coldcard_trims_private_key_zero() {
        let chain_code = [2_u8; 32];
        let mut private_key = [1_u8; 32];
        private_key[31] = 0;
        SecretKey::from_slice(&private_key).unwrap();
        let mut encoded = vec![PAYLOAD_CODE_STASH, STASH_MARKER_XPRV];
        encoded.extend_from_slice(&chain_code);
        encoded.extend_from_slice(&private_key);
        trim_stash_padding(&mut encoded);

        let DecodedPayload::Xprv(decoded) = DecodedPayload::decode(&encoded).unwrap() else {
            panic!("expected xprv")
        };
        let decoded = Xpriv::from_str(decoded.expose_string()).unwrap();

        assert_eq!(decoded.chain_code.as_bytes(), &chain_code);
        assert_eq!(decoded.private_key.secret_bytes(), private_key);
    }

    #[test]
    fn raw_master_secret_stash_decodes_as_bip32_master() {
        for seed_len in [16_usize, 32, 64] {
            let seed = vec![7_u8; seed_len];
            let mut encoded = vec![PAYLOAD_CODE_STASH, seed_len as u8];
            encoded.extend_from_slice(&seed);

            let DecodedPayload::Xprv(decoded) = DecodedPayload::decode(&encoded).unwrap() else {
                panic!("expected xprv for seed length {seed_len}")
            };
            let expected = Xpriv::new_master(NetworkKind::Main, &seed).unwrap();

            assert_eq!(decoded.expose_string(), expected.to_string());
        }
    }

    #[test]
    fn raw_master_secret_stash_restores_trimmed_trailing_zeros() {
        let mut seed = [9_u8; 24];
        seed[20..].fill(0);
        let mut encoded = vec![PAYLOAD_CODE_STASH, 24];
        encoded.extend_from_slice(&seed);
        trim_stash_padding(&mut encoded);
        assert!(encoded.len() < 26);

        let DecodedPayload::Xprv(decoded) = DecodedPayload::decode(&encoded).unwrap() else {
            panic!("expected xprv")
        };
        let expected = Xpriv::new_master(NetworkKind::Main, &seed).unwrap();

        assert_eq!(decoded.expose_string(), expected.to_string());
    }

    #[test]
    fn raw_master_secret_stash_rejects_out_of_range_lengths() {
        for marker in [0x02_u8, 0x0f, 0x41, 0x7f] {
            let mut encoded = vec![PAYLOAD_CODE_STASH, marker];
            encoded.extend_from_slice(&[3_u8; 70]);

            assert!(
                matches!(DecodedPayload::decode(&encoded), Err(Error::InvalidMnemonicPayload)),
                "marker 0x{marker:02x} should be rejected"
            );
        }
    }

    #[test]
    fn full_xprv_payload_discards_child_metadata_like_coldcard() {
        let master = Xpriv::from_str(XPRV).unwrap();
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let child = master.derive_priv(&secp, &[ChildNumber::Hardened { index: 7 }]).unwrap();
        let mut payload = vec![PAYLOAD_CODE_XPRV];
        payload.extend_from_slice(&child.encode());

        let DecodedPayload::Xprv(decoded) = DecodedPayload::decode(&payload).unwrap() else {
            panic!("expected xprv")
        };
        let decoded = Xpriv::from_str(decoded.expose_string()).unwrap();

        assert_eq!(decoded.depth, 0);
        assert_eq!(decoded.parent_fingerprint, Fingerprint::default());
        assert_eq!(decoded.child_number, ChildNumber::Normal { index: 0 });
        assert_eq!(decoded.chain_code, child.chain_code);
        assert_eq!(decoded.private_key, child.private_key);
    }

    #[test]
    fn full_xprv_payload_rejects_non_mainnet_keys() {
        let testnet = Xpriv::new_master(NetworkKind::Test, &[42; 32]).unwrap();
        let mut payload = Zeroizing::new(vec![PAYLOAD_CODE_XPRV]);
        payload.extend_from_slice(&testnet.encode());

        assert!(matches!(DecodedPayload::decode(&payload), Err(Error::NonMainnetXprvPayload)));
    }

    #[test]
    fn decodes_quick_text_note() {
        let decoded =
            DecodedPayload::decode(br#"n[{"title":"Quick Note","misc":"Meet at the park"}]"#)
                .unwrap();
        let DecodedPayload::Notes(notes) = decoded else {
            panic!("expected notes payload");
        };
        let [NotesRecord::Note(note)] = notes.records() else {
            panic!("expected one secure note");
        };

        assert_eq!(note.title(), "Quick Note");
        assert_eq!(note.text(), "Meet at the park");
        assert_eq!(note.group(), "");
    }

    #[test]
    fn decodes_note_and_password_display_fields() {
        let decoded = DecodedPayload::decode(
            br#"n[
                {"title":"Recovery","misc":"stored offsite","group":"Bitcoin"},
                {"title":"Server","user":"alice","password":"correct horse","site":"example.com","misc":"rotate yearly","group":"Work"}
            ]"#,
        )
        .unwrap();
        let DecodedPayload::Notes(notes) = decoded else {
            panic!("expected notes payload");
        };
        let [NotesRecord::Note(note), NotesRecord::Password(password)] = notes.records() else {
            panic!("expected a note followed by a password");
        };

        assert_eq!(note.title(), "Recovery");
        assert_eq!(note.text(), "stored offsite");
        assert_eq!(note.group(), "Bitcoin");
        assert_eq!(password.title(), "Server");
        assert_eq!(password.username(), "alice");
        assert_eq!(password.password(), "correct horse");
        assert_eq!(password.site(), "example.com");
        assert_eq!(password.notes(), "rotate yearly");
        assert_eq!(password.group(), "Work");
    }

    #[test]
    fn password_record_uses_user_field_as_protocol_discriminator() {
        let decoded =
            DecodedPayload::decode(br#"n[{"title":"Empty password","user":""}]"#).unwrap();
        let DecodedPayload::Notes(notes) = decoded else {
            panic!("expected notes payload");
        };
        let [NotesRecord::Password(password)] = notes.records() else {
            panic!("expected one password record");
        };

        assert_eq!(password.username(), "");
        assert_eq!(password.password(), "");
    }

    #[test]
    fn rejects_malformed_notes_as_payload_errors() {
        let malformed = [
            &b"n\xff"[..],
            &b"nnot-json"[..],
            &b"n{}"[..],
            &b"n[]"[..],
            &br#"n[{"misc":"missing title"}]"#[..],
            &br#"n[{"title":""}]"#[..],
            &br#"n[{"title":"wrong type","misc":7}]"#[..],
            &br#"n[{"title":"null field","misc":null}]"#[..],
            &br#"n[{"title":"missing user","password":"secret"}]"#[..],
            &br#"n[{"title":"unknown field","other":"secret"}]"#[..],
        ];

        for payload in malformed {
            assert!(matches!(DecodedPayload::decode(payload), Err(Error::InvalidNotesPayload)));
        }
    }

    #[test]
    fn other_unsupported_payload_types_remain_typed() {
        for (code, expected) in [
            (PAYLOAD_CODE_VAULT, UnsupportedPayloadKind::Vault),
            (PAYLOAD_CODE_PSBT, UnsupportedPayloadKind::Psbt),
            (PAYLOAD_CODE_BACKUP, UnsupportedPayloadKind::Backup),
            (b'?', UnsupportedPayloadKind::Unknown(b'?')),
        ] {
            assert!(matches!(
                DecodedPayload::decode(&[code]),
                Err(Error::UnsupportedPayload(kind)) if kind == expected
            ));
        }
    }
}

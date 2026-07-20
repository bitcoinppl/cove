use std::{fmt, str::FromStr};

use bip39::Mnemonic;
use bitcoin::{base58, bip32::Xpriv, secp256k1::SecretKey};
use serde::{Deserialize, Deserializer};
use zeroize::Zeroize;

use crate::{Error, Result};

const MAINNET_XPRV_VERSION: [u8; 4] = [0x04, 0x88, 0xad, 0xe4];

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
            Self::Vault => f.write_str("v"),
            Self::Psbt => f.write_str("p"),
            Self::Backup => f.write_str("b"),
            Self::Unknown(code) => write!(f, "0x{code:02x}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Payload {
    Mnemonic(Mnemonic),
    Xprv(XprvPayload),
}

impl Payload {
    pub fn mnemonic(mnemonic: Mnemonic) -> Self {
        Self::Mnemonic(mnemonic)
    }

    pub fn xprv(value: impl AsRef<str>) -> Result<Self> {
        Ok(Self::Xprv(XprvPayload::parse(value.as_ref())?))
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Mnemonic(mnemonic) => encode_mnemonic_payload(mnemonic),
            Self::Xprv(xprv) => encode_xprv_payload(xprv),
        }
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mnemonic(_) => f.write_str("Payload::Mnemonic(****)"),
            Self::Xprv(_) => f.write_str("Payload::Xprv(****)"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct XprvPayload {
    value: String,
}

impl XprvPayload {
    pub fn parse(value: &str) -> Result<Self> {
        let xprv = Xpriv::from_str(value).map_err(|_| Error::InvalidXprvPayload)?;

        Ok(Self { value: xprv.to_string() })
    }

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
            b's' => decode_stash_payload(body),
            b'x' => decode_xprv_body(body),
            b'n' => decode_notes_body(body),
            b'v' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Vault)),
            b'p' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Psbt)),
            b'b' => Err(Error::UnsupportedPayload(UnsupportedPayloadKind::Backup)),
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

fn encode_mnemonic_payload(mnemonic: &Mnemonic) -> Result<Vec<u8>> {
    let entropy = mnemonic.to_entropy();
    let marker = 0x80 | ((entropy.len() / 8) - 2) as u8;
    let mut encoded = Vec::with_capacity(1 + 1 + entropy.len());
    encoded.push(b's');
    encoded.push(marker);
    encoded.extend_from_slice(&entropy);

    Ok(encoded)
}

fn encode_xprv_payload(xprv: &XprvPayload) -> Result<Vec<u8>> {
    let decoded = base58::decode_check(&xprv.value)?;
    Xpriv::decode(&decoded)?;

    let mut encoded = Vec::with_capacity(1 + decoded.len());
    encoded.push(b'x');
    encoded.extend_from_slice(&decoded);

    Ok(encoded)
}

fn decode_stash_payload(body: &[u8]) -> Result<DecodedPayload> {
    let (&marker, rest) = body.split_first().ok_or(Error::InvalidMnemonicPayload)?;

    if marker == 0x01 {
        return decode_stash_xprv(rest);
    }

    if marker & 0x80 == 0 {
        return Err(Error::InvalidMnemonicPayload);
    }

    let entropy_len = usize::from((marker & 0x03) + 2) * 8;
    if !matches!(entropy_len, 16 | 24 | 32) || rest.len() < entropy_len {
        return Err(Error::InvalidMnemonicPayload);
    }

    let mnemonic = Mnemonic::from_entropy(&rest[..entropy_len])?;

    Ok(DecodedPayload::Mnemonic(mnemonic))
}

fn decode_stash_xprv(body: &[u8]) -> Result<DecodedPayload> {
    if body.len() < 64 {
        return Err(Error::InvalidXprvPayload);
    }

    let mut encoded = [0_u8; 78];
    encoded[0..4].copy_from_slice(&MAINNET_XPRV_VERSION);
    encoded[13..45].copy_from_slice(&body[0..32]);
    encoded[45] = 0;
    encoded[46..78].copy_from_slice(&body[32..64]);

    let xprv = Xpriv::decode(&encoded)?;
    SecretKey::from_slice(&body[32..64]).map_err(|_| Error::InvalidXprvPayload)?;

    Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }))
}

fn decode_xprv_body(body: &[u8]) -> Result<DecodedPayload> {
    let xprv = Xpriv::decode(body).map_err(|_| Error::InvalidXprvPayload)?;

    Ok(DecodedPayload::Xprv(XprvPayload { value: xprv.to_string() }))
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
            (b'v', UnsupportedPayloadKind::Vault),
            (b'p', UnsupportedPayloadKind::Psbt),
            (b'b', UnsupportedPayloadKind::Backup),
            (b'?', UnsupportedPayloadKind::Unknown(b'?')),
        ] {
            assert!(matches!(
                DecodedPayload::decode(&[code]),
                Err(Error::UnsupportedPayload(kind)) if kind == expected
            ));
        }
    }
}

/// This entire file should be moved into rust-cktap crate
use std::{collections::HashMap, num::ParseIntError, sync::Arc};

use bitcoin::{
    hashes::Hash as _,
    key::Secp256k1,
    secp256k1::{
        Message, PublicKey,
        ecdsa::{RecoverableSignature, RecoveryId},
        hashes::sha256::Hash,
    },
};
use serde::Deserialize;
use tracing::debug;

use super::{SatsCard, SatsCardState, TapCard, TapSigner, TapSignerState};

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(
    Debug, Clone, Copy, derive_more::Display, PartialEq, Eq, Hash, Deserialize, uniffi::Enum,
)]
pub enum Field {
    #[display("signature field: 's'")]
    Signature,

    #[display("ident field: 'c'")]
    Ident,

    #[display("state field: 'u'")]
    State,

    #[display("nonce field: 'n'")]
    Nonce,

    #[display("slot number field: 'o'")]
    SlotNumber,

    #[display("address field: 'r'")]
    Address,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("not a valid url, must start with tapsigner.com or getsatscard.com: {0}")]
    InvalidUrl(String),

    #[error("not in encoded url format: {0}")]
    NotUrlEncoded(#[from] serde_urlencoded::de::Error),

    #[error("missing field {0}")]
    MissingField(Field),

    #[error("unknown card state {0}")]
    UnknownCardState(char),

    #[error("card state is empty")]
    EmptyCardState,

    #[error("unable to parse slot number: {0}")]
    UnableToParseSlot(#[from] ParseIntError),

    #[error("unable to parse signature: {0}")]
    UnableToParseSignature(#[from] SignatureParseError),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SignatureParseError {
    #[error("unable to parse signature: {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),

    #[error("signatrue is not 64 bytes, found {0} bytes")]
    InvalidSignatureLength(u32),

    #[error("invalid pubkey length, found {0} bytes, expected 33 bytes")]
    InvalidPubkeyLength(u32),

    #[error("unable to recover pubkey from signature, tries all recovery ids")]
    UnableToRecoverPubkey,

    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),

    #[error("pubkey digest does not match card ident")]
    PubkeyIdentMismatch,
}

// Parse URL-encoded string into a Card
pub fn parse_card(url_encoded: &str) -> Result<TapCard> {
    let url_encoded = url_encoded
        .trim()
        .trim_start_matches(|p: char| !p.is_ascii_alphabetic())
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let url_encoded = url_encoded
        .strip_prefix("tapsigner.com/start#")
        .or_else(|| url_encoded.strip_prefix("getsatscard.com/start#"))
        .ok_or_else(|| Error::InvalidUrl(url_encoded.to_string()))?;

    // Parse URL-encoded string into a HashMap
    let params: HashMap<&str, &str> = serde_urlencoded::from_str(url_encoded)?;

    let nonce = params
        .get("n")
        .ok_or(Error::MissingField(Field::Nonce))?
        .to_string();

    let signature = params
        .get("s")
        .ok_or(Error::MissingField(Field::Signature))?
        .to_string();

    let state_field = params.get("u").ok_or(Error::MissingField(Field::State))?;

    // Check if it's a TapSigner (has t=1)
    if is_tap_signer(&params) {
        let card_ident = params
            .get("c")
            .ok_or(Error::MissingField(Field::Ident))?
            .to_string();

        let state = parse_tap_signer_state(state_field)?;

        let url_message_digest = full_message_digest(url_encoded);
        let pubkey = message_and_signature_to_pubkey(url_message_digest, &card_ident, &signature)?;

        let tap_signer = TapSigner {
            state,
            card_ident,
            nonce,
            signature,
            pubkey: Arc::new(pubkey),
        };

        return Ok(TapCard::TapSigner(tap_signer.into()));
    }

    // It's a SatsCard
    let slot_number = params
        .get("o")
        .ok_or(Error::MissingField(Field::SlotNumber))?
        .parse::<u32>()?;

    let address_suffix = params
        .get("r")
        .ok_or(Error::MissingField(Field::Address))?
        .to_string();

    let state = parse_sats_card_state(state_field)?;

    Ok(TapCard::SatsCard(SatsCard {
        state,
        slot_number,
        address_suffix,
        nonce,
        signature,
    }))
}

// Helper function to parse state
fn parse_tap_signer_state(state_str: &str) -> Result<TapSignerState> {
    match state_str {
        "s" | "S" => Ok(TapSignerState::Sealed),
        "u" | "U" => Ok(TapSignerState::Unused),
        "e" | "E" => Ok(TapSignerState::Error),
        "" => Err(Error::EmptyCardState),
        unknown => Err(Error::UnknownCardState(
            unknown.chars().next().expect("just checked"),
        )),
    }
}

fn parse_sats_card_state(state_str: &str) -> Result<SatsCardState> {
    match state_str {
        "s" | "S" => Ok(SatsCardState::Sealed),
        "u" | "U" => Ok(SatsCardState::Unsealed),
        "e" | "E" => Ok(SatsCardState::Error),
        "" => Err(Error::EmptyCardState),
        unknown => Err(Error::UnknownCardState(
            unknown.chars().next().expect("just checked"),
        )),
    }
}

fn is_tap_signer(params: &HashMap<&str, &str>) -> bool {
    params.get("t").is_some_and(|v| *v == "1")
}

fn message_and_signature_to_pubkey(
    message: Message,
    card_ident: &str,
    signature: &str,
) -> Result<PublicKey, SignatureParseError> {
    let card_ident_bytes = hex::decode(card_ident).map_err(SignatureParseError::HexDecode)?;
    let pubkeys = message_and_signature_to_pubkeys(message, signature)?;

    for pubkey in pubkeys {
        let pubkey_message_digest = Hash::hash(&pubkey.serialize());
        if pubkey_message_digest[..8] == card_ident_bytes {
            return Ok(pubkey);
        }
    }

    Err(SignatureParseError::PubkeyIdentMismatch.into())
}

fn message_and_signature_to_pubkeys(
    message: Message,
    signature: &str,
) -> Result<Vec<PublicKey>, SignatureParseError> {
    let signature = hex::decode(signature.as_bytes()).map_err(SignatureParseError::HexDecode)?;
    let mut pubkeys = Vec::with_capacity(4);

    for rec_id in 0..4 {
        let recovery_id = RecoveryId::from_i32(rec_id as i32).expect("recovery id is a valid i32");

        match try_for_recovery_id(&message, &signature, recovery_id) {
            Ok(pubkey) => pubkeys.push(pubkey),
            Err(e) => {
                debug!("unable to recover pubkey from signature: {e}, recovery id: {rec_id}");
                continue;
            }
        }
    }

    if pubkeys.is_empty() {
        return Err(SignatureParseError::UnableToRecoverPubkey.into());
    }

    Ok(pubkeys)
}

fn full_message_digest(url_encoded: &str) -> Message {
    let message = url_message_for_digest(url_encoded);
    crate::util::message_digest(message.as_bytes())
}

fn url_message_for_digest(url_encoded: &str) -> &str {
    let start_of_sig = url_encoded.rfind('=').map_or(0, |pos| pos + 1);
    &url_encoded[0..start_of_sig]
}

fn try_for_recovery_id(
    message: &Message,
    signature: &[u8],
    recovery_id: RecoveryId,
) -> Result<PublicKey, SignatureParseError> {
    let secp = Secp256k1::new();
    let signature = RecoverableSignature::from_compact(signature, recovery_id)?;
    let pubkey = secp.recover_ecdsa(message, &signature)?;

    Ok(pubkey)
}

pub fn card_pubkey_to_full_ident(card_pubkey: &[u8]) -> Result<String, SignatureParseError> {
    // convert pubkey into a hash formated for humans
    // - sha256(compressed-pubkey)
    // - skip first 8 bytes of that (because that's revealed in NFC URL)
    // - base32 and take first 20 chars in 4 groups of five
    // - insert dashes
    // - result is 23 chars long
    use data_encoding::BASE32;

    if card_pubkey.len() != 33 {
        return Err(SignatureParseError::InvalidPubkeyLength(card_pubkey.len() as u32).into());
    }

    let pubkey_hash = Hash::hash(card_pubkey);
    let encoded = BASE32.encode(&pubkey_hash[8..]);

    let full_ident = format!(
        "{}-{}-{}-{}",
        &encoded[0..5],
        &encoded[5..10],
        &encoded[10..15],
        &encoded[15..20]
    );

    Ok(full_ident)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_tap_signer() {
        let url = "https://tapsigner.com/start#t=1&u=S&c=04d74fb1dfee7a4d&n=8940dc9808088820&s=6bda376546b7074b5a52f3264fe118d38889f49501b591b0b9e90a2ff2e07d26572898aaeb0f963a52cf707e7483203520ce40bdf5071e8f80262d587b41b99f";
        let tap_card = TapCard::parse(url);
        assert!(tap_card.is_ok());

        assert!(matches!(tap_card.clone().unwrap(), TapCard::TapSigner(_)));
        let TapCard::TapSigner(tap_signer) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(tap_signer.state, TapSignerState::Sealed);
        assert_eq!(tap_signer.card_ident, "04d74fb1dfee7a4d");
        assert_eq!(tap_signer.nonce, "8940dc9808088820");
    }

    #[test]
    fn test_parses_sats_card() {
        let card = "https://getsatscard.com/start#u=S&o=0&r=95kesdwq&n=ab78fd50637f8f5a&s=26d1a0684f99fe43b223dca75081bb05bd0233b901139cdd33a4d0a2e61666ed1470d7c53d90f6ae4c60a6cbc7a0f4ded5f13461092b24604ad476bbcf1dd913";
        let tap_card = TapCard::parse(card);

        assert!(tap_card.is_ok());
        assert!(matches!(tap_card.clone().unwrap(), TapCard::SatsCard(_)));

        let TapCard::SatsCard(sats_card) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(sats_card.state, SatsCardState::Sealed);
        assert_eq!(sats_card.nonce, "ab78fd50637f8f5a");
        assert_eq!(
            sats_card.signature,
            "26d1a0684f99fe43b223dca75081bb05bd0233b901139cdd33a4d0a2e61666ed1470d7c53d90f6ae4c60a6cbc7a0f4ded5f13461092b24604ad476bbcf1dd913"
        );
    }

    #[test]
    fn test_get_url_msg_for_digest() {
        let url = "t=1&u=S&c=04d74fb1dfee7a4d&n=8940dc9808088820&s=6bda376546b7074b5a52f3264fe118d38889f49501b591b0b9e90a2ff2e07d26572898aaeb0f963a52cf707e7483203520ce40bdf5071e8f80262d587b41b99f";
        let expected = "t=1&u=S&c=04d74fb1dfee7a4d&n=8940dc9808088820&s=";
        let msg = url_message_for_digest(url);
        assert_eq!(msg, expected);
    }

    #[test]
    fn test_tap_signer_readable_ident_string() {
        let url = "https://tapsigner.com/start#t=1&u=S&c=04d74fb1dfee7a4d&n=8940dc9808088820&s=6bda376546b7074b5a52f3264fe118d38889f49501b591b0b9e90a2ff2e07d26572898aaeb0f963a52cf707e7483203520ce40bdf5071e8f80262d587b41b99f";
        let tap_card = TapCard::parse(url);
        assert!(tap_card.is_ok());

        let ts = match tap_card.unwrap() {
            TapCard::TapSigner(ts) => ts,
            _ => panic!("not a tap signer"),
        };

        let readable_ident = &ts.readable_ident();
        assert_eq!(readable_ident, "XUFC5-2SWY2-PX24Q-IZC7W")
    }
}

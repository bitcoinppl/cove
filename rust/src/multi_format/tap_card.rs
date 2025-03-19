use std::{collections::HashMap, num::ParseIntError};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TapSignerState {
    Sealed,
    Unused,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SatsCardState {
    Sealed,
    Unsealed,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SatsCard {
    pub state: SatsCardState,
    pub slot_number: u32,
    pub address_suffix: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TapSigner {
    pub state: TapSignerState,
    pub card_ident: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TapCard {
    SatsCard(SatsCard),
    TapSigner(TapSigner),
}

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
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl TapCard {
    pub fn parse(url: &str) -> Result<TapCard> {
        parse_card(url)
    }
}

// Parse URL-encoded string into a Card
fn parse_card(url_encoded: &str) -> Result<TapCard> {
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
    let params: HashMap<&str, &str> = serde_urlencoded::from_str(&url_encoded)?;

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

        return Ok(TapCard::TapSigner(TapSigner {
            state,
            card_ident,
            nonce,
            signature,
        }));
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

// For uniffi
pub mod ffi {
    use super::*;

    #[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, uniffi::Error)]
    pub enum TapCardParseError {
        #[error("not a valid url: {0}")]
        InvalidUrl(String),

        #[error("not in encoded url format: {0}")]
        NotUrlEncoded(String),

        #[error("missing field {0}")]
        MissingField(Field),

        #[error("unknown card state {0}")]
        UnknownCardState(String),

        #[error("card state is empty")]
        EmptyCardState,

        #[error("unable to parse slot number: {0}")]
        ParseSlotNumberError(String),
    }

    impl From<Error> for TapCardParseError {
        fn from(error: Error) -> Self {
            match error {
                Error::InvalidUrl(error) => Self::InvalidUrl(error.to_string()),
                Error::NotUrlEncoded(error) => Self::NotUrlEncoded(error.to_string()),
                Error::MissingField(field) => Self::MissingField(field),
                Error::UnknownCardState(state) => Self::UnknownCardState(state.to_string()),
                Error::EmptyCardState => Self::EmptyCardState,
                Error::UnableToParseSlot(error) => Self::ParseSlotNumberError(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_tap_signer() {
        let card = "tapsigner.com/start#t=1&u=U&c=0000000000000000&n=0000000000000000&s=00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
        let tap_card = TapCard::parse(card);

        println!("{tap_card:?}");
        assert!(tap_card.is_ok());

        assert!(matches!(tap_card.clone().unwrap(), TapCard::TapSigner(_)));
        let TapCard::TapSigner(tap_signer) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(tap_signer.state, TapSignerState::Unused);
        assert_eq!(tap_signer.card_ident, "0000000000000000");
        assert_eq!(tap_signer.nonce, "0000000000000000");
        assert_eq!(
            tap_signer.signature,
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn test_parses_tap_signer_order() {
        let card = "tapsigner.com/start#u=U&c=0000000000000000&n=0000000000000000&s=00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000&t=1";
        let tap_card = TapCard::parse(card);

        println!("{tap_card:?}");
        assert!(tap_card.is_ok());

        assert!(matches!(tap_card.clone().unwrap(), TapCard::TapSigner(_)));
        let TapCard::TapSigner(tap_signer) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(tap_signer.state, TapSignerState::Unused);
        assert_eq!(tap_signer.card_ident, "0000000000000000");
        assert_eq!(tap_signer.nonce, "0000000000000000");
        assert_eq!(
            tap_signer.signature,
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn test_parses_sats_card() {
        let card = "getsatscard.com/start#t=0&u=U&c=0000000000000000&n=0000000000000000&s=00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000&r=bcajrh2jdk&o=1";
        let tap_card = TapCard::parse(card);

        println!("{tap_card:?}");
        assert!(tap_card.is_ok());

        assert!(matches!(tap_card.clone().unwrap(), TapCard::SatsCard(_)));
        let TapCard::SatsCard(sats_card) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(sats_card.state, SatsCardState::Unsealed);
        assert_eq!(sats_card.nonce, "0000000000000000");
        assert_eq!(
            sats_card.signature,
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
    }
}

mod ffi_preview {
    use super::*;

    #[uniffi::export]
    pub fn tap_signer_preview_new(preview: bool) -> TapSigner {
        assert!(preview);
        TapSigner {
                state: TapSignerState::Unused,
                card_ident: "0000000000000000".to_string(),
                nonce: "0000000000000000".to_string(),
                signature: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000".to_string(),
            }
    }
}

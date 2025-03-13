use std::{collections::HashMap, num::ParseIntError};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CardState {
    Sealed,
    Unsealed,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SatsCard {
    pub state: CardState,
    pub slot_number: u32,
    pub address_suffix: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TapSigner {
    pub state: CardState,
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
    #[error("not in encoded url format: {0}")]
    NotUrlEncoded(#[from] serde_urlencoded::de::Error),

    #[error("missing field {0}")]
    MissingField(Field),

    #[error("unknown card state {0}")]
    UnknownCardState(char),

    #[error("card state is empty")]
    EmptyCardState,

    #[error("unable to parse slot number: {0}")]
    ParseSlotNumberError(#[from] ParseIntError),
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl TapCard {
    pub fn parse(url: &str) -> Result<TapCard> {
        parse_card(url)
    }
}

// Parse URL-encoded string into a Card
fn parse_card(url_encoded: &str) -> Result<TapCard> {
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
    let state = parse_state(state_field)?;

    // Check if it's a TapSigner (has t=1)
    if is_tap_signer(url_encoded, &params) {
        let card_ident = params
            .get("c")
            .ok_or(Error::MissingField(Field::Ident))?
            .to_string();

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

    Ok(TapCard::SatsCard(SatsCard {
        state,
        slot_number,
        address_suffix,
        nonce,
        signature,
    }))
}

// Helper function to parse state
fn parse_state(state_str: &str) -> Result<CardState> {
    match state_str {
        "s" | "S" => Ok(CardState::Sealed),
        "u" | "U" => Ok(CardState::Unsealed),
        "e" | "E" => Ok(CardState::Error),
        "" => Err(Error::EmptyCardState),
        unknown => Err(Error::UnknownCardState(
            unknown.chars().next().expect("just checked"),
        )),
    }
}

fn is_tap_signer(url_encoded: &str, params: &HashMap<&str, &str>) -> bool {
    static PATTERN: &str = "#t=";
    if let Some(position) = memchr::memmem::find(url_encoded.as_bytes(), PATTERN.as_bytes()) {
        let start_position = position + PATTERN.len();
        let end_position = start_position + 1;

        let t = &url_encoded[start_position..end_position];
        return t == "1";
    }

    params.get("t").map_or(false, |v| *v == "1")
}

// For uniffi
pub mod ffi {
    use super::*;

    #[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, uniffi::Error)]
    pub enum TapCardParseError {
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
                Error::NotUrlEncoded(error) => Self::NotUrlEncoded(error.to_string()),
                Error::MissingField(field) => Self::MissingField(field),
                Error::UnknownCardState(state) => Self::UnknownCardState(state.to_string()),
                Error::EmptyCardState => Self::EmptyCardState,
                Error::ParseSlotNumberError(error) => Self::ParseSlotNumberError(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_tap_card() {
        let card = "tapsigner.com/start#t=1&u=U&c=0000000000000000&n=0000000000000000&s=00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
        let tap_card = TapCard::parse(card);

        println!("{tap_card:?}");
        assert!(tap_card.is_ok());

        assert!(matches!(tap_card.clone().unwrap(), TapCard::TapSigner(_)));
        let TapCard::TapSigner(tap_signer) = tap_card.unwrap() else {
            panic!("not a tap signer")
        };

        assert_eq!(tap_signer.state, CardState::Unsealed);
        assert_eq!(tap_signer.card_ident, "0000000000000000");
        assert_eq!(tap_signer.nonce, "0000000000000000");
        assert_eq!(
            tap_signer.signature,
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
    }
}

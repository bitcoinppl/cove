pub mod parse;

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum TapSignerState {
    Sealed,
    Unused,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum SatsCardState {
    Sealed,
    Unsealed,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Record)]
pub struct SatsCard {
    pub state: SatsCardState,
    pub slot_number: u32,
    pub address_suffix: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Object)]
pub struct TapSigner {
    pub state: TapSignerState,
    pub card_ident: String,
    pub nonce: String,
    pub signature: String,
    pub pubkey: Arc<PublicKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum TapCard {
    SatsCard(SatsCard),
    TapSigner(Arc<TapSigner>),
}

impl TapCard {
    /// Parse a tap card URL
    ///
    /// # Errors
    /// Returns an error if the URL is invalid or missing required fields
    pub fn parse(url: &str) -> Result<Self, parse::Error> {
        parse::parse_card(url)
    }
}

#[uniffi::export]
impl TapSigner {
    /// Get the full card identifier string
    ///
    /// # Panics
    /// Panics if the pubkey is invalid (should not happen as it's already validated)
    pub fn full_card_ident(&self) -> String {
        let pubkey_bytes = PublicKey::serialize(&self.pubkey);
        parse::card_pubkey_to_full_ident(&pubkey_bytes).expect("already validated pubkey")
    }

    pub fn ident_file_name_prefix(&self) -> String {
        self.full_card_ident().replace('-', "").to_ascii_lowercase()
    }

    pub fn is_equal(&self, rhs: &Self) -> bool {
        self == rhs
    }
}

// For uniffi
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, uniffi::Error)]
pub enum TapCardParseError {
    #[error("not a valid url: {0}")]
    InvalidUrl(String),

    #[error("not in encoded url format: {0}")]
    NotUrlEncoded(String),

    #[error("missing field {0}")]
    MissingField(parse::Field),

    #[error("unknown card state {0}")]
    UnknownCardState(String),

    #[error("card state is empty")]
    EmptyCardState,

    #[error("unable to parse slot number: {0}")]
    ParseSlotNumberError(String),

    #[error("unable to parse signature: {0}")]
    UnableToParseSignature(String),

    #[error("unable to recover pubkey from signature")]
    UnableToRecoverPubkey,
}

impl From<parse::Error> for TapCardParseError {
    fn from(error: parse::Error) -> Self {
        use parse::Error;
        match error {
            Error::InvalidUrl(error) => Self::InvalidUrl(error),
            Error::NotUrlEncoded(error) => Self::NotUrlEncoded(error.to_string()),
            Error::MissingField(field) => Self::MissingField(field),
            Error::UnknownCardState(state) => Self::UnknownCardState(state.to_string()),
            Error::EmptyCardState => Self::EmptyCardState,
            Error::UnableToParseSlot(error) => Self::ParseSlotNumberError(error.to_string()),
            Error::UnableToParseSignature(error) => Self::UnableToParseSignature(error.to_string()),
        }
    }
}

/// Create a preview `TapSigner` for testing/UI purposes
///
/// # Panics
/// Panics if `preview` is false
#[uniffi::export]
pub fn tap_signer_preview_new(preview: bool) -> TapSigner {
    assert!(preview);
    TapSigner {
            state: TapSignerState::Unused,
            card_ident: "0000000000000000".to_string(),
            nonce: "0000000000000000".to_string(),
            signature: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000".to_string(),
            pubkey: Arc::new(PublicKey::from_slice(&[0u8; 33]).unwrap()),
        }
}

uniffi::setup_scaffolding!();

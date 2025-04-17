pub mod parse;

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TapSigner {
    pub state: TapSignerState,
    pub card_ident: String,
    pub nonce: String,
    pub signature: String,
    pub pubkey: Arc<PublicKey>,
}

// Manual implementation of Serialize for TapSigner
impl Serialize for TapSigner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        
        let pubkey_hex = hex::encode(PublicKey::serialize(&self.pubkey));
        
        let mut state = serializer.serialize_struct("TapSigner", 5)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("card_ident", &self.card_ident)?;
        state.serialize_field("nonce", &self.nonce)?;
        state.serialize_field("signature", &self.signature)?;
        state.serialize_field("pubkey_hex", &pubkey_hex)?;
        state.end()
    }
}

// Manual implementation of Deserialize for TapSigner
impl<'de> Deserialize<'de> for TapSigner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct TapSignerVisitor;

        impl<'de> Visitor<'de> for TapSignerVisitor {
            type Value = TapSigner;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct TapSigner")
            }

            fn visit_map<V>(self, mut map: V) -> Result<TapSigner, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut state = None;
                let mut card_ident = None;
                let mut nonce = None;
                let mut signature = None;
                let mut pubkey_hex = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        "state" => {
                            if state.is_some() {
                                return Err(de::Error::duplicate_field("state"));
                            }
                            state = Some(map.next_value()?);
                        }
                        "card_ident" => {
                            if card_ident.is_some() {
                                return Err(de::Error::duplicate_field("card_ident"));
                            }
                            card_ident = Some(map.next_value()?);
                        }
                        "nonce" => {
                            if nonce.is_some() {
                                return Err(de::Error::duplicate_field("nonce"));
                            }
                            nonce = Some(map.next_value()?);
                        }
                        "signature" => {
                            if signature.is_some() {
                                return Err(de::Error::duplicate_field("signature"));
                            }
                            signature = Some(map.next_value()?);
                        }
                        "pubkey_hex" => {
                            if pubkey_hex.is_some() {
                                return Err(de::Error::duplicate_field("pubkey_hex"));
                            }
                            pubkey_hex = Some(map.next_value()?);
                        }
                        _ => {
                            // Ignore unknown fields
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let state = state.ok_or_else(|| de::Error::missing_field("state"))?;
                let card_ident = card_ident.ok_or_else(|| de::Error::missing_field("card_ident"))?;
                let nonce = nonce.ok_or_else(|| de::Error::missing_field("nonce"))?;
                let signature = signature.ok_or_else(|| de::Error::missing_field("signature"))?;
                let pubkey_hex: String = pubkey_hex.ok_or_else(|| de::Error::missing_field("pubkey_hex"))?;
                
                // Convert pubkey_hex to PublicKey
                let pubkey_bytes = hex::decode(&pubkey_hex).map_err(|e| {
                    de::Error::custom(format!("Invalid pubkey hex: {}", e))
                })?;
                
                let pubkey = PublicKey::from_slice(&pubkey_bytes).map_err(|e| {
                    de::Error::custom(format!("Invalid pubkey: {}", e))
                })?;

                Ok(TapSigner {
                    state,
                    card_ident,
                    nonce,
                    signature,
                    pubkey: Arc::new(pubkey),
                })
            }
        }

        deserializer.deserialize_map(TapSignerVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TapCard {
    SatsCard(SatsCard),
    TapSigner(Arc<TapSigner>),
}

impl TapCard {
    pub fn parse(url: &str) -> Result<TapCard, parse::Error> {
        parse::parse_card(url)
    }
}

#[uniffi::export]
impl TapSigner {
    pub fn full_card_ident(&self) -> String {
        let pubkey_bytes = PublicKey::serialize(&self.pubkey);
        parse::card_pubkey_to_full_ident(&pubkey_bytes).expect("already validated pubkey")
    }

    pub fn ident_file_name_prefix(&self) -> String {
        self.full_card_ident().replace("-", "").to_ascii_lowercase()
    }

    pub fn is_equal(&self, rhs: &Self) -> bool {
        self == rhs
    }
}

// For uniffi
pub mod ffi {
    use super::{parse::Field, *};

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

        #[error("unable to parse signature: {0}")]
        UnableToParseSignature(String),

        #[error("unable to recover pubkey from signature")]
        UnableToRecoverPubkey,
    }

    impl From<parse::Error> for TapCardParseError {
        fn from(error: parse::Error) -> Self {
            use parse::Error;
            match error {
                Error::InvalidUrl(error) => Self::InvalidUrl(error.to_string()),
                Error::NotUrlEncoded(error) => Self::NotUrlEncoded(error.to_string()),
                Error::MissingField(field) => Self::MissingField(field),
                Error::UnknownCardState(state) => Self::UnknownCardState(state.to_string()),
                Error::EmptyCardState => Self::EmptyCardState,
                Error::UnableToParseSlot(error) => Self::ParseSlotNumberError(error.to_string()),
                Error::UnableToParseSignature(error) => {
                    Self::UnableToParseSignature(error.to_string())
                }
            }
        }
    }
}

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
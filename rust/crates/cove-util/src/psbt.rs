//! Payload-free PSBT parsing errors
//!
//! The parser's own error text quotes the input, for example raw key bytes, hash preimages or
//! xpubs, so every caller that logs or returns a PSBT parse failure goes through [`deserialize`]
//! and only ever sees a [`PsbtDecodeError`]

use bitcoin::psbt::{Error as BitcoinPsbtError, Psbt};

/// Why serialized PSBT bytes could not be parsed
///
/// Each variant carries no data from the input, so its `Display` is safe to log and show
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum PsbtDecodeError {
    #[error("invalid PSBT header")]
    InvalidHeader,

    #[error("invalid PSBT key")]
    InvalidKey,

    #[error("duplicate PSBT key")]
    DuplicateKey,

    #[error("invalid unsigned transaction")]
    InvalidUnsignedTransaction,

    #[error("preimage does not match its hash")]
    PreimageHashMismatch,

    #[error("invalid hash")]
    InvalidHash,

    #[error("invalid public key")]
    InvalidPublicKey,

    #[error("invalid xpub")]
    InvalidXpub,

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid taproot data")]
    InvalidTaproot,

    #[error("non-standard sighash type")]
    NonStandardSighashType,

    #[error("unsupported PSBT version")]
    UnsupportedVersion,

    #[error("malformed PSBT encoding")]
    MalformedEncoding,

    #[error("invalid PSBT")]
    Other,
}

impl PsbtDecodeError {
    /// A stable snake_case name for the failure, for structured log fields
    pub const fn kind(self) -> &'static str {
        match self {
            Self::InvalidHeader => "invalid_header",
            Self::InvalidKey => "invalid_key",
            Self::DuplicateKey => "duplicate_key",
            Self::InvalidUnsignedTransaction => "invalid_unsigned_transaction",
            Self::PreimageHashMismatch => "preimage_hash_mismatch",
            Self::InvalidHash => "invalid_hash",
            Self::InvalidPublicKey => "invalid_public_key",
            Self::InvalidXpub => "invalid_xpub",
            Self::InvalidSignature => "invalid_signature",
            Self::InvalidTaproot => "invalid_taproot",
            Self::NonStandardSighashType => "non_standard_sighash_type",
            Self::UnsupportedVersion => "unsupported_version",
            Self::MalformedEncoding => "malformed_encoding",
            Self::Other => "other",
        }
    }
}

impl From<&BitcoinPsbtError> for PsbtDecodeError {
    fn from(error: &BitcoinPsbtError) -> Self {
        use BitcoinPsbtError as E;

        match error {
            E::InvalidMagic | E::InvalidSeparator => Self::InvalidHeader,
            E::InvalidKey(_) | E::InvalidProprietaryKey => Self::InvalidKey,
            E::DuplicateKey(_) => Self::DuplicateKey,
            E::UnsignedTxHasScriptSigs
            | E::UnsignedTxHasScriptWitnesses
            | E::MustHaveUnsignedTx => Self::InvalidUnsignedTransaction,
            E::InvalidPreimageHashPair { .. } => Self::PreimageHashMismatch,
            E::InvalidHash(_) => Self::InvalidHash,
            E::InvalidPublicKey(_) | E::InvalidSecp256k1PublicKey(_) | E::InvalidXOnlyPublicKey => {
                Self::InvalidPublicKey
            }
            E::XPubKey(_) | E::CombineInconsistentKeySources(_) => Self::InvalidXpub,
            E::InvalidEcdsaSignature(_) | E::InvalidTaprootSignature(_) => Self::InvalidSignature,
            E::InvalidControlBlock | E::InvalidLeafVersion | E::Taproot(_) | E::TapTree(_) => {
                Self::InvalidTaproot
            }
            E::NonStandardSighashType(_) => Self::NonStandardSighashType,
            E::Version(_) => Self::UnsupportedVersion,
            E::ConsensusEncoding(_) | E::NoMorePairs | E::PartialDataConsumption | E::Io(_) => {
                Self::MalformedEncoding
            }

            // the remaining variants come from combining or fee checks, not parsing, and the
            // enum is non_exhaustive, so anything new maps to a generic reason
            _ => Self::Other,
        }
    }
}

impl From<BitcoinPsbtError> for PsbtDecodeError {
    fn from(error: BitcoinPsbtError) -> Self {
        Self::from(&error)
    }
}

/// Parse serialized PSBT bytes, reducing any failure to a payload-free reason
///
/// # Errors
/// Returns the [`PsbtDecodeError`] matching the parser failure
pub fn deserialize(bytes: &[u8]) -> Result<Psbt, PsbtDecodeError> {
    Psbt::deserialize(bytes).map_err(PsbtDecodeError::from)
}

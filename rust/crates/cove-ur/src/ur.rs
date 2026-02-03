//! UR (Uniform Resource) wrapper with case-insensitive parsing
//! per BCR-2020-005 spec
use crate::error::{Result, UrError};
use foundation_ur::UR as FoundationUr;

/// UR enum - either direct (already lowercase) or normalized (was uppercase)
pub enum Ur<'a> {
    /// Input was uppercase - stores normalized string
    Normalized(UrNormalized),
    /// Input was already lowercase - borrows directly
    Direct(FoundationUr<'a>),
}

impl<'a> Ur<'a> {
    /// Parse a UR string (case-insensitive scheme per spec)
    ///
    /// # Errors
    /// Returns error if UR parsing fails
    pub fn parse(ur_string: &'a str) -> Result<Self> {
        if ur_string.starts_with("ur:") {
            // already lowercase, use directly
            let ur =
                FoundationUr::parse(ur_string).map_err(|e| UrError::UrParseError(e.to_string()))?;

            return Ok(Self::Direct(ur));
        }

        // uppercase or mixed case, normalize
        Ok(Self::Normalized(UrNormalized::parse(ur_string)?))
    }

    /// Get the UR type (e.g., "crypto-account", "crypto-psbt")
    #[must_use]
    pub fn ur_type(&self) -> &str {
        match self {
            Self::Normalized(inner) => inner.ur_type(),
            Self::Direct(ur) => ur.as_type(),
        }
    }

    /// Get message/fragment bytes (decoded from bytewords)
    #[must_use]
    pub fn message_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Normalized(inner) => inner.message_bytes(),
            Self::Direct(ur) => Self::extract_message_bytes(ur),
        }
    }

    fn extract_message_bytes(ur: &FoundationUr<'_>) -> Option<Vec<u8>> {
        match ur {
            FoundationUr::SinglePart { message, .. } => {
                foundation_ur::bytewords::decode(message, foundation_ur::bytewords::Style::Minimal)
                    .ok()
            }
            FoundationUr::SinglePartDeserialized { message, .. } => Some(message.to_vec()),
            FoundationUr::MultiPart { fragment, .. } => {
                foundation_ur::bytewords::decode(fragment, foundation_ur::bytewords::Style::Minimal)
                    .ok()
            }
            FoundationUr::MultiPartDeserialized { fragment, .. } => Some(fragment.data.to_vec()),
        }
    }

    /// Get a `foundation_ur::UR` reference
    ///
    /// # Errors
    /// Returns error if UR parsing fails
    pub fn to_foundation_ur(&self) -> Result<FoundationUr<'_>> {
        match self {
            Self::Normalized(inner) => inner.to_foundation_ur(),
            Self::Direct(ur) => Ok(ur.clone()),
        }
    }
}

/// Parsed UR that owns its data (for uppercase input that needed normalization)
#[derive(Debug, Clone)]
pub struct UrNormalized {
    /// The normalized (lowercased) UR string
    normalized: String,
    /// The UR type extracted at parse time
    ur_type: String,
}

impl UrNormalized {
    /// Parse a UR string, normalizing to lowercase
    ///
    /// # Errors
    /// Returns error if UR parsing fails or type is missing
    pub fn parse(ur_string: &str) -> Result<Self> {
        let normalized = ur_string.to_ascii_lowercase();
        FoundationUr::parse(&normalized).map_err(|e| UrError::UrParseError(e.to_string()))?;

        // extract type at parse time - fail here if malformed
        let ur_type = normalized
            .strip_prefix("ur:")
            .and_then(|s| s.split('/').next())
            .ok_or_else(|| UrError::InvalidField("Missing UR type".into()))?
            .to_string();

        Ok(Self { normalized, ur_type })
    }

    /// Get the UR type (e.g., "crypto-account", "crypto-psbt")
    #[must_use]
    pub fn ur_type(&self) -> &str {
        &self.ur_type
    }

    /// Get a `foundation_ur::UR` that borrows from self
    ///
    /// # Errors
    /// Returns error if UR parsing fails
    pub fn to_foundation_ur(&self) -> Result<FoundationUr<'_>> {
        FoundationUr::parse(&self.normalized).map_err(|e| UrError::UrParseError(e.to_string()))
    }

    /// Get message/fragment bytes (decoded from bytewords)
    #[must_use]
    pub fn message_bytes(&self) -> Option<Vec<u8>> {
        self.to_foundation_ur().ok().and_then(|ur| Ur::extract_message_bytes(&ur))
    }
}

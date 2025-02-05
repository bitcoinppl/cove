use pubport::descriptor;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum XpubError {
    #[error("Invalid descriptor {0}")]
    InvalidDescriptor(#[from] DescriptorError),

    #[error("Invalid JSON {0}")]
    InvalidJson(String),

    #[error("Invalid descriptor in JSON")]
    InvalidDescriptorInJson,

    #[error("JSON has no descriptor")]
    JsonNoDecriptor,

    #[error("Missing xpub: {0}")]
    MissingXpub(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum DescriptorError {
    #[error("Invalid descriptor: {0:?}")]
    InvalidDescriptor(String),

    #[error("Single descriptor line did not contain both external and internal keys")]
    MissingKeys,

    #[error("Too many keys in descriptor, only supports 1 external and 1 internal key")]
    TooManyKeys(u32),

    #[error("Unable to parse descriptor: {0}")]
    InvalidDescriptorParse(String),

    #[error("Missing descriptor")]
    MissingDescriptor,

    #[error("Missing xpub")]
    MissingXpub,

    #[error("Missing derivation path")]
    MissingDerivationPath,

    #[error("Missing script type")]
    MissingScriptType,

    #[error("Missing fingerprint (xfp)")]
    MissingFingerprint,

    #[error("Unable to parse xpub: {0:?}")]
    InvalidXpub(String),

    #[error("Unable to parse xpub: {0}")]
    UnableToParseXpub(String),

    #[error("Unable to get xpub from descriptor")]
    NoXpubInDescriptor,

    #[error("Single pubkey is not supported, must be an extended key")]
    SinglePubkeyNotSupported,
}

impl From<descriptor::Error> for DescriptorError {
    fn from(error: descriptor::Error) -> Self {
        type DS = descriptor::Error;

        match error {
            DS::InvalidDescriptor(error) => Self::InvalidDescriptor(error.to_string()),
            DS::MissingKeys => Self::MissingKeys,
            DS::TooManyKeys(keys) => Self::TooManyKeys(keys as u32),
            DS::InvalidDescriptorParse(error) => Self::InvalidDescriptorParse(error.to_string()),
            DS::MissingDescriptor => Self::MissingDescriptor,
            DS::MissingXpub => Self::MissingXpub,
            DS::MissingDerivationPath => Self::MissingDerivationPath,
            DS::MissingScriptType => Self::MissingScriptType,
            DS::MissingFingerprint => Self::MissingFingerprint,
            DS::InvalidXpub(error) => Self::InvalidXpub(error.to_string()),
            DS::SinglePubkeyNotSupported => Self::SinglePubkeyNotSupported,
            DS::UnableToParseXpub(error) => Self::UnableToParseXpub(error.to_string()),
            DS::NoXpubInDescriptor => Self::NoXpubInDescriptor,
            DS::InvalidJsonDescriptor(..) => {
                Self::InvalidDescriptor("invalid json descriptor".to_string())
            }
        }
    }
}

impl From<pubport::Error> for XpubError {
    fn from(error: pubport::Error) -> Self {
        use pubport::Error;

        match error {
            Error::InvalidDescriptor(error) => Self::InvalidDescriptor(error.into()),
            Error::InvalidJsonParse(error) => Self::InvalidJson(error.to_string()),
            Error::InvalidDescriptorInJson => Self::InvalidDescriptorInJson,
            Error::JsonNoDecriptor => Self::JsonNoDecriptor,
        }
    }
}

impl From<descriptor::Error> for XpubError {
    fn from(error: descriptor::Error) -> Self {
        Self::InvalidDescriptor(error.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum ResumeError {
    /// The starting block of the new message is not the same as the one in the old message
    #[error("Blocks do not match")]
    BlocksDoNotMatch,

    #[error("The reader had already parsed the message")]
    AlreadyParsed,

    #[error("Parsing error, error getting message info: {0}")]
    ParsingError(String),

    /// Block size mismatch, expected {expected}, got {actual}
    ///
    /// The bytes passed in needs to be a multiple of crate::cove_nfc::BYTES_PER_BLOCK
    /// The bytes passed in needs to be the same size as the bytes in the old message (NUMBER_OF_BLOCKS_PER_CHUNK * BYTES_PER_BLOCK)
    #[error("Block size mismatch, expected {expected}, got {actual})")]
    BlockSizeMismatch { expected: u16, actual: u16 },

    #[error("Unable to get first block hash")]
    UnableToGetFirstBlockHash,
}
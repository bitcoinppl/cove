use derive_more::derive::Display;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error, Display)]
pub enum ResumeError {
    /// Blocks do not match
    ///
    /// The starting block of the new message is not the same as the one in the old message
    BlocksDoNotMatch,

    /// The reader had already parsed the message
    AlreadyParsed,

    /// Parsing error, error getting message info: {0}
    ParsingError(String),

    /// Block size mismatch, expected {expected}, got {actual}
    ///
    /// The bytes passed in needs to be a multiple of crate::cove_nfc::BYTES_PER_BLOCK
    /// The bytes passed in needs to be the same size as the bytes in the old message (NUMBER_OF_BLOCKS_PER_CHUNK * BYTES_PER_BLOCK)
    #[display("Block size mismatch, expected {expected}, got {actual})")]
    BlockSizeMismatch { expected: u16, actual: u16 },

    /// Unable to get first block hash
    UnableToGetFirstBlockHash,
}

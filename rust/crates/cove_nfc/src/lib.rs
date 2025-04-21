use message_info::MessageInfo;
use parser::{parse_message_info, stream::StreamExt};
use record::NdefRecord;
use resume::ResumeError;
use sha2::{Digest, Sha256};
use tracing::warn;
use winnow::error::Needed;

uniffi::setup_scaffolding!();

pub mod ffi;
pub mod header;
pub mod message;
pub mod message_info;
pub mod ndef_type;
pub mod parser;
pub mod payload;
pub mod record;
pub mod resume;

/// Number of blocks read at a time from the NFC chip
pub const NUMBER_OF_BLOCKS_PER_CHUNK: u16 = 32;

/// Number of bytes per block read from the NFC chip
pub const BYTES_PER_BLOCK: u16 = 4;

#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum NfcReaderError {
    #[error("Error parsing the NDEF message")]
    ParsingError(String),

    #[error("Not enough data to parse, need atleast enough to parse the message info")]
    NotEnoughData,

    #[error("Trying to parse a message that has already been parsed")]
    AlreadyParsed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ParseResult {
    /// Completed The message is a NDEF message
    Complete(MessageInfo, Vec<NdefRecord>),

    /// Incomplete, need more data to parse the message
    Incomplete(ParsingMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ParsingMessage {
    pub message_info: MessageInfo,
    pub left_over_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Enum)]
pub enum ParserState {
    #[default]
    NotStarted,
    Parsing(ParsingContext),
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ParsingContext {
    pub message_info: MessageInfo,
    pub needed: u16,
    first_block_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfcReader {
    state: ParserState,
}

impl Default for NfcReader {
    fn default() -> Self {
        Self::new()
    }
}

impl NfcReader {
    pub fn new() -> NfcReader {
        NfcReader {
            state: ParserState::default(),
        }
    }

    /// Parse the entire message if possible, if not return the unused data and the message info
    pub fn parse(&mut self, data: Vec<u8>) -> Result<ParseResult, NfcReaderError> {
        match &mut self.state {
            ParserState::NotStarted => {
                let mut stream = parser::stream::new(&data);
                let message_info =
                    parse_message_info(&mut stream).map_err(|_e| NfcReaderError::NotEnoughData)?;

                let parsed = data.len() - stream.len();

                self.state = ParserState::Parsing(ParsingContext {
                    message_info,
                    needed: message_info.full_message_length - parsed as u16,
                    first_block_hash: get_first_block_hash(&data),
                });

                self.parse_incomplete(stream)
            }

            ParserState::Parsing(_) => self.parse_incomplete(data),

            ParserState::Complete => Err(NfcReaderError::AlreadyParsed),
        }
    }

    fn parse_incomplete<'a>(
        &mut self,
        data: impl StreamExt + 'a,
    ) -> Result<ParseResult, NfcReaderError> {
        let parsing = match &mut self.state {
            ParserState::Parsing(parsing_state) => parsing_state,
            _ => panic!("not in parsing state"),
        };

        // need more data to parse the message
        if (parsing.needed as usize) >= data.len() {
            tracing::debug!("not enough data to parse message, continuing");
            let left_over_bytes = data.to_vec();

            // return incomplete
            let result = ParseResult::Incomplete(ParsingMessage {
                message_info: parsing.message_info,
                left_over_bytes,
            });

            return Ok(result);
        }

        // have enough data to parse the message
        tracing::debug!("enough data to parse message, trying to parse");

        let mut stream = data.to_stream();
        match parser::parse_ndef_records(&mut stream, &parsing.message_info) {
            Ok(result) => {
                let result = ParseResult::Complete(parsing.message_info, result);
                self.state = ParserState::Complete;
                Ok(result)
            }

            Err(winnow::error::ErrMode::Incomplete(Needed::Size(need_more))) => {
                tracing::warn!("incomplete, need more data, incorrect payload length was provided");

                // add the number of missing bytes to the full message length
                parsing.message_info.full_message_length += need_more.get() as u16;

                let result = ParseResult::Incomplete(ParsingMessage {
                    message_info: parsing.message_info,
                    left_over_bytes: data.to_vec(),
                });

                Ok(result)
            }

            Err(error) => Err(NfcReaderError::ParsingError(format!(
                "error parsing message: {error}"
            ))),
        }
    }

    /// Try resuming on a partially parsed message
    pub fn is_resumeable(&mut self, data: Vec<u8>) -> Result<(), ResumeError> {
        let expected_bytes = BYTES_PER_BLOCK * NUMBER_OF_BLOCKS_PER_CHUNK;
        let data_len = data.len() as u16;

        if data_len < expected_bytes {
            return Err(ResumeError::BlockSizeMismatch {
                expected: expected_bytes,
                actual: data_len,
            });
        }

        let parsing_state = match &mut self.state {
            ParserState::Parsing(parsing_state) => parsing_state,
            ParserState::Complete => return Err(ResumeError::AlreadyParsed),
            ParserState::NotStarted => {
                warn!(
                    "resuming on a message that has not been parsed, starting from the beginning"
                );

                return Ok(());
            }
        };

        let Some(first_block_hash) = &get_first_block_hash(&data) else {
            return Err(ResumeError::UnableToGetFirstBlockHash);
        };

        let Some(existing_first_block_hash) = &parsing_state.first_block_hash else {
            return Err(ResumeError::UnableToGetFirstBlockHash);
        };

        // scanning a different message
        if first_block_hash != existing_first_block_hash {
            return Err(ResumeError::BlocksDoNotMatch);
        }

        // scanning the same message
        Ok(())
    }

    /// Check if the reader is started
    pub fn is_started(&self) -> bool {
        matches!(self.state, ParserState::Parsing(_))
    }

    /// Get the message info, if we have that info
    pub fn message_info(&self) -> Option<&MessageInfo> {
        match &self.state {
            ParserState::Parsing(parsing_state) => Some(&parsing_state.message_info),
            ParserState::Complete | ParserState::NotStarted => None,
        }
    }
}

fn get_first_block_hash(data: &[u8]) -> Option<String> {
    let hash_bytes_length = (BYTES_PER_BLOCK * NUMBER_OF_BLOCKS_PER_CHUNK) as usize;
    if data.len() < hash_bytes_length {
        return None;
    }

    let data = &data[..hash_bytes_length];

    let mut sha256 = Sha256::new();
    sha256.update(data);
    let hash = sha256.finalize();

    let hash = hex::encode(hash);
    Some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export_bytes() -> Vec<u8> {
        let file_contents = include_bytes!("../../../../test/data/export_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        bytes
    }

    #[test]
    fn test_parsing_in_chunks() {
        let mut data = Vec::new();
        let mut reader = NfcReader::new();

        let mut chunks_processed = 0;

        let export_bytes = export_bytes();
        assert_eq!(export_bytes.len(), 3044);

        for chunk in export_bytes.chunks(100) {
            let mut chunk_data = std::mem::take(&mut data);
            chunk_data.extend_from_slice(chunk);

            let result = reader.parse(chunk_data.to_vec()).unwrap();

            match result {
                ParseResult::Complete(info, records) => {
                    assert_eq!(info.full_message_length, 3043);
                    assert_eq!(records.len(), 1);
                    break;
                }
                ParseResult::Incomplete(incomplete) => {
                    data = incomplete.left_over_bytes;
                    chunks_processed += 1;
                }
            }
        }

        assert_eq!(chunks_processed, 30);
    }
}
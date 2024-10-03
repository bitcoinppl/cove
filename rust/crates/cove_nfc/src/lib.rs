use derive_more::derive::Display;
use message_info::MessageInfo;
use parser::{parse_message_info, stream_ext::ParserStreamExt};
use record::NdefRecord;

pub mod header;
pub mod message_info;
pub mod ndef_type;
pub mod parser;
pub mod payload;
pub mod record;

#[derive(Debug, PartialEq, Eq, thiserror::Error, Display)]
pub enum NfcReaderError {
    /// Error parsing the NDEF message
    ParsingError(String),

    /// Not enough data to parse, need atleast enough to parse the message info
    NotEnoughData,

    /// Trying to parse a message that has already been parsed
    AlreadyParsed,
}

#[derive(Debug)]
pub enum ParseResult {
    /// Completed The message is a NDEF message
    Complete(MessageInfo, Vec<NdefRecord>),

    /// Incomplete, need more data to parse the message
    Incomplete(ParsingMessage),
}

#[derive(Debug, Clone)]
pub struct ParsingMessage {
    pub message_info: MessageInfo,
    pub left_over_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub enum ParserState {
    #[default]
    NotStarted,
    Parsing(ParsingContext),
    Complete,
}

#[derive(Debug, Clone)]
pub struct ParsingContext {
    pub message_info: MessageInfo,
    pub needed: u16,
}

#[derive(Debug, Clone)]
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
                let mut stream = parser::stream(&data);
                let message_info =
                    parse_message_info(&mut stream).map_err(|_e| NfcReaderError::NotEnoughData)?;

                let parsed = data.len() - stream.len();

                self.state = ParserState::Parsing(ParsingContext {
                    message_info,
                    needed: message_info.total_payload_length - parsed as u16,
                });

                self.parse_incomplete(&stream)
            }

            ParserState::Parsing(_) => {
                let data = data.as_slice();
                self.parse_incomplete(&data)
            }

            ParserState::Complete => Err(NfcReaderError::AlreadyParsed),
        }
    }

    fn parse_incomplete<'a, 'b, S>(&'a mut self, data: &'b S) -> Result<ParseResult, NfcReaderError>
    where
        S: ParserStreamExt<'b>,
    {
        let parsing = match &mut self.state {
            ParserState::Parsing(parsing_state) => parsing_state,
            _ => panic!("not in parsing state"),
        };

        // need more data to parse the message
        if (parsing.needed as usize) >= data.len() {
            let left_over_bytes = data.to_vec();

            // return incomplete
            let result = ParseResult::Incomplete(ParsingMessage {
                message_info: parsing.message_info,
                left_over_bytes,
            });

            return Ok(result);
        }

        // have enough data to parse the message
        let mut stream = data.to_stream();
        let result = parser::parse_ndef_records(&mut stream, &parsing.message_info)
            .map_err(|e| NfcReaderError::ParsingError(format!("error parsing message: {e}")))?;

        let result = ParseResult::Complete(parsing.message_info, result);
        self.state = ParserState::Complete;

        Ok(result)
    }
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;

    fn export_bytes() -> Vec<u8> {
        let file_contents = include_bytes!("../test/data/export_bytes.txt");
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
                    assert_eq!(info.total_payload_length, 3031);
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

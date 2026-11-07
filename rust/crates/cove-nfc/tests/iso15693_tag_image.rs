//! Reads hardware wallet exports from ISO 15693 tag images the way `ios/Cove/NFCReader.swift`
//! reads a tag block by block, through the same `FfiNfcReader` and `NfcConst` calls
//!
//! The fixtures are tag images as they sit in tag memory: the capability container, the NDEF TLV
//! and its records, then the rest of the memory. The Swift block loop itself, CoreNFC and a real
//! tag aren't reached from here

use cove_nfc::{
    NfcReaderError, ParseResult,
    ffi::{FfiNfcReader, NfcConst},
    message::NfcMessage,
    message_info::MessageInfo,
    resume::ResumeError,
};

/// Size the tag images are zero-padded to, so a read past the message's last block returns the
/// tag's empty memory as a real tag does instead of running off the image
const TAG_MEMORY_BYTES: usize = 8 * 1024;

/// A ColdCard generic JSON export (`application/json` record) for master fingerprint `817e7be0`
const EXPORT_TAG: &[u8] = include_bytes!("../test/data/export_bytes.txt");

/// A text record holding that wallet's BIP84 descriptor, followed by stale bytes from an earlier,
/// longer message
const DESCRIPTOR_TAG: &[u8] = include_bytes!("../test/data/descriptor_bytes.txt");

const EXPORT_JSON: &[u8] = include_bytes!("../test/data/export.json");
const DESCRIPTOR: &str = include_str!("../test/data/descriptor.txt");

fn tag_image(fixture: &[u8]) -> Vec<u8> {
    let mut image: Vec<u8> = std::str::from_utf8(fixture)
        .unwrap()
        .split(',')
        .map(|byte| byte.trim().parse().unwrap())
        .collect();

    image.resize(TAG_MEMORY_BYTES, 0);
    image
}

/// How a tag session ends
#[derive(Debug)]
enum SessionEnd {
    /// The message was read and turned into the `NfcMessage` the app handles
    Complete(NfcMessage),

    /// The tag left the field after the given number of chunk reads, before the message was read
    Lost,
}

/// The state `NFCReader.swift` keeps across tag sessions, driven the way its `readBlocks`,
/// `prepareToResumeReading`, `handleSuccessfulBlockRead` and `parseAndHandleResult` drive it
struct IosBlockReader {
    consts: NfcConst,
    reader: FfiNfcReader,
    read_bytes: Vec<u8>,
    message_info: Option<MessageInfo>,
    current_block: usize,

    /// The answer to the last resume check, if a session made one
    last_resume: Option<Result<(), ResumeError>>,

    /// Chunk reads in the last session, including the resume check's read of the first chunk
    reads: usize,
}

impl IosBlockReader {
    fn new() -> Self {
        Self {
            consts: NfcConst::new(),
            reader: FfiNfcReader::new(),
            read_bytes: Vec::new(),
            message_info: None,
            current_block: 0,
            last_resume: None,
            reads: 0,
        }
    }

    /// `resetReader()`, which keeps `messageInfo`
    fn reset(&mut self) {
        self.reader = FfiNfcReader::new();
        self.read_bytes.clear();
        self.current_block = 0;
    }

    /// `extendedReadMultipleBlocks` of `numberOfBlocksPerChunk` blocks from `first_block`
    fn read_chunk(&mut self, tag: &[u8], first_block: usize) -> Vec<u8> {
        let start = first_block * usize::from(self.consts.bytes_per_block());
        let end = start + usize::from(self.consts.total_bytes_per_chunk());
        self.reads += 1;

        tag.get(start..end)
            .unwrap_or_else(|| panic!("read past the end of the tag at block {first_block}"))
            .to_vec()
    }

    /// One tag session; with `lost_after`, the tag leaves the field after that many chunk reads
    fn session(
        &mut self,
        tag: &[u8],
        lost_after: Option<usize>,
    ) -> Result<SessionEnd, NfcReaderError> {
        self.reads = 0;
        self.last_resume = None;

        // a started reader holding bytes is the same tag held again or a different one
        if self.reader.is_started() && !self.read_bytes.is_empty() {
            let first_chunk = self.read_chunk(tag, 0);
            let resume = self.reader.is_resumeable(first_chunk);

            if resume.is_err() {
                self.reset();
            }

            self.last_resume = Some(resume);
        }

        let blocks_per_chunk = usize::from(self.consts.number_of_blocks_per_chunk());

        loop {
            if lost_after.is_some_and(|lost_after| self.reads >= lost_after) {
                return Ok(SessionEnd::Lost);
            }

            let chunk = self.read_chunk(tag, self.current_block);
            self.current_block += blocks_per_chunk;
            self.read_bytes.extend_from_slice(&chunk);

            let should_parse = match self.message_info {
                Some(info) => self.read_bytes.len() >= usize::from(info.full_message_length),
                None => true,
            };

            if !should_parse {
                continue;
            }

            match self.reader.parse(self.read_bytes.clone()) {
                Ok(ParseResult::Incomplete(parsing)) => {
                    self.message_info = Some(parsing.message_info);
                    self.read_bytes = parsing.left_over_bytes;
                }

                Ok(ParseResult::Complete(_, records)) => {
                    self.reset();
                    return Ok(SessionEnd::Complete(self.message_from(records)));
                }

                Err(error) => {
                    self.reset();
                    return Err(error);
                }
            }
        }
    }

    /// The `NfcMessage` the `.complete` arm of `parseAndHandleResult` builds from the records
    fn message_from(&self, records: Vec<cove_nfc::record::NdefRecord>) -> NfcMessage {
        let string = self.reader.string_from_record(records[0].clone());
        let mut data = None;

        if string.is_none()
            && let cove_nfc::payload::NdefPayload::Data(bytes) = &records[0].payload
        {
            data = Some(bytes.clone());
        }

        if records.len() > 1 {
            data = Some(self.reader.data_from_records(records));
        }

        NfcMessage::try_new(string, data).unwrap()
    }
}

fn expect_complete(end: SessionEnd) -> NfcMessage {
    match end {
        SessionEnd::Complete(message) => message,
        SessionEnd::Lost => panic!("session ended without reading the message"),
    }
}

fn assert_is_export_json(message: &NfcMessage) {
    let Some(text) = message.string() else { panic!("expected a text message, got {message:?}") };
    let read: serde_json::Value = serde_json::from_str(&text).expect("tag message is not json");
    let expected: serde_json::Value = serde_json::from_slice(EXPORT_JSON).unwrap();

    assert_eq!(read, expected);
}

/// Reads before the message completes on an uninterrupted read of the export tag
fn uninterrupted_reads(tag: &[u8]) -> usize {
    let mut reader = IosBlockReader::new();
    assert_is_export_json(&expect_complete(reader.session(tag, None).unwrap()));
    reader.reads
}

#[test]
fn export_tag_read_in_chunks_gives_the_wallet_json() {
    let tag = tag_image(EXPORT_TAG);
    let mut reader = IosBlockReader::new();

    let message = expect_complete(reader.session(&tag, None).unwrap());
    assert_is_export_json(&message);
}

#[test]
fn export_tag_held_again_after_each_lost_read_resumes_where_it_stopped() {
    let tag = tag_image(EXPORT_TAG);
    let total_reads = uninterrupted_reads(&tag);

    for lost_after in 1..total_reads {
        let mut reader = IosBlockReader::new();

        let end = reader.session(&tag, Some(lost_after)).unwrap();
        assert!(matches!(end, SessionEnd::Lost), "lost after {lost_after}: {end:?}");

        let message = expect_complete(reader.session(&tag, None).unwrap());
        assert_is_export_json(&message);

        assert_eq!(reader.last_resume, Some(Ok(())), "lost after {lost_after}");

        // one read for the resume check, then only the chunks the first session didn't read
        assert_eq!(reader.reads, 1 + total_reads - lost_after, "lost after {lost_after}");
    }
}

#[test]
fn different_tag_held_after_a_lost_read_starts_over() {
    let export_tag = tag_image(EXPORT_TAG);
    let descriptor_tag = tag_image(DESCRIPTOR_TAG);
    let mut reader = IosBlockReader::new();

    let end = reader.session(&export_tag, Some(3)).unwrap();
    assert!(matches!(end, SessionEnd::Lost), "{end:?}");

    let message = expect_complete(reader.session(&descriptor_tag, None).unwrap());
    assert_eq!(reader.last_resume, Some(Err(ResumeError::BlocksDoNotMatch)));
    assert_eq!(message.string().as_deref(), Some(DESCRIPTOR.trim()));
}

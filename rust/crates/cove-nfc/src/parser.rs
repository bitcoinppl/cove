pub mod stream;

use stream::Stream;
use winnow::{
    ModalResult, Parser,
    binary::{
        Endianness, be_u8, be_u16,
        bits::{bits, bool as take_bool, take as take_bits},
    },
    error::{ContextError, ErrMode},
    token::{any, literal, take},
};

use crate::{
    header::NdefHeader,
    message_info::MessageInfo,
    ndef_type::NdefType,
    payload::{NdefPayload, TextPayload, TextPayloadFormat},
    record::NdefRecord,
};

pub fn parse_ndef_records(
    input: &mut Stream<'_>,
    info: &MessageInfo,
) -> ModalResult<Vec<NdefRecord>> {
    let mut records = Vec::new();
    let payload_length = info.payload_length as usize;
    let mut total_parsed_bytes = 0;

    loop {
        let input_start_bytes = input.len();
        let record = parse_ndef_record.parse_next(input)?;
        records.push(record);

        let parsed_bytes = input_start_bytes - input.len();
        total_parsed_bytes += parsed_bytes;

        // parsed all bytes dictated by payload_length
        if total_parsed_bytes >= payload_length && !records.is_empty() {
            break;
        }
    }

    Ok(records)
}

pub fn parse_ndef_record(input: &mut Stream<'_>) -> ModalResult<NdefRecord> {
    let header = parse_header.parse_next(input)?;
    let type_ = parse_type(input, header.type_length)?;
    let id = parse_id(input, header.id_length)?;
    let payload = parse_payload(input, header.payload_length, &type_)?;

    Ok(NdefRecord {
        header,
        type_,
        id,
        payload,
    })
}

pub fn parse_message_info(input: &mut Stream<'_>) -> ModalResult<MessageInfo> {
    let _ = literal([226, 67, 0, 1, 0, 0, 4, 0, 3]).parse_next(input)?;

    let length_indicator = be_u8.parse_next(input)?;

    let total_payload_length = if length_indicator == 255 {
        be_u16.parse_next(input)?
    } else {
        length_indicator as u16
    };

    Ok(MessageInfo::new(total_payload_length))
}

// private
fn parse_header_byte(input: &mut Stream<'_>) -> ModalResult<(bool, bool, bool, bool, bool, u8)> {
    bits::<_, _, ErrMode<ContextError>, _, _>((
        take_bool,
        take_bool,
        take_bool,
        take_bool,
        take_bool,
        take_bits(3_u8),
    ))
    .parse_next(input)
}

fn parse_header(input: &mut Stream<'_>) -> ModalResult<NdefHeader> {
    let (message_begin, message_end, chunked, short_record, has_id_length, type_name_format) =
        parse_header_byte(input)?;

    let type_length = winnow::binary::u8.parse_next(input)?;

    let type_name_format = match type_name_format {
        0 => NdefType::Empty,
        1 => NdefType::WellKnown,
        2 => NdefType::Mime,
        3 => NdefType::AbsoluteUri,
        4 => NdefType::External,
        5 => NdefType::Unknown,
        6 => NdefType::Unchanged,
        7 => NdefType::Reserved,
        _ => panic!("number out of range, impossible, only 3 bits"),
    };

    let payload_length = if short_record {
        any.map(|x: u8| x as u32).parse_next(input)?
    } else {
        winnow::binary::u32(Endianness::Big).parse_next(input)?
    };

    let id_length = if has_id_length {
        Some(any.parse_next(input)?)
    } else {
        None
    };

    Ok(NdefHeader {
        message_begin,
        message_end,
        chunked,
        short_record,
        has_id_length,
        type_name_format,
        type_length,
        payload_length,
        id_length,
    })
}

fn parse_type(input: &mut Stream<'_>, type_length: u8) -> ModalResult<Vec<u8>> {
    take(type_length as usize)
        .map(|s: &[u8]| s.to_vec())
        .parse_next(input)
}

fn parse_id(input: &mut Stream<'_>, id_length: Option<u8>) -> ModalResult<Option<Vec<u8>>> {
    if let Some(id_len) = id_length {
        take(id_len as usize)
            .map(|s: &[u8]| Some(s.to_vec()))
            .parse_next(input)
    } else {
        Ok(None)
    }
}

fn parse_payload(
    input: &mut Stream<'_>,
    payload_length: u32,
    type_: &[u8],
) -> ModalResult<NdefPayload> {
    if type_ == b"T" {
        let (is_utf16, language_code_length): (bool, u8) =
            bits::<_, _, ErrMode<ContextError>, _, _>((take_bool, take_bits(7_u8)))
                .parse_next(input)?;

        let language_code = take(language_code_length as usize).parse_next(input)?;

        let remaining_length = payload_length - language_code_length as u32 - 1;
        let text = take(remaining_length as usize).parse_next(input)?;

        let parsed_text = if is_utf16 {
            String::from_utf16_lossy(
                &text
                    .chunks_exact(2)
                    .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                    .collect::<Vec<u16>>(),
            )
        } else {
            String::from_utf8_lossy(text).to_string()
        };

        let parsed_text = TextPayload {
            format: if is_utf16 {
                TextPayloadFormat::Utf16
            } else {
                TextPayloadFormat::Utf8
            },
            language: String::from_utf8_lossy(language_code).to_string(),
            text: parsed_text,
        };

        Ok(NdefPayload::Text(parsed_text))
    } else {
        take(payload_length as usize)
            .map(|s: &[u8]| NdefPayload::Data(s.to_vec()))
            .parse_next(input)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use winnow::{
        Bytes,
        error::{ErrMode, Needed},
    };

    use super::*;

    use crate::{header::NdefHeader, ndef_type::NdefType, payload::NdefPayload};

    fn owned_stream(bytes: Vec<u8>) -> Stream<'static> {
        let bytes = Box::leak(bytes.into_boxed_slice());
        Stream::new(Bytes::new(bytes))
    }

    fn parse_ndef_message(input: &mut Stream<'_>) -> ModalResult<Vec<NdefRecord>> {
        let info = parse_message_info.parse_next(input)?;
        parse_ndef_records(input, &info)
    }

    static EXPORT: LazyLock<Stream<'static>> = LazyLock::new(|| {
        let file_contents = include_bytes!("../../../../test/data/export_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        owned_stream(bytes)
    });

    static DESCRIPTOR: LazyLock<Stream<'static>> = LazyLock::new(|| {
        let file_contents = include_bytes!("../../../../test/data/descriptor_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        owned_stream(bytes)
    });

    static SEED_WORDS_BYTES: LazyLock<Stream<'static>> = LazyLock::new(|| {
        let file_contents = include_bytes!("../../../../test/data/seed_words_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        owned_stream(bytes)
    });

    fn export_bytes() -> Stream<'static> {
        let mut data = *EXPORT;
        let _payload_length = parse_message_info(&mut data).unwrap();
        data
    }

    fn descriptor_bytes() -> Stream<'static> {
        let mut data = *DESCRIPTOR;
        let _payload_length = parse_message_info(&mut data).unwrap();
        data
    }

    #[test]
    fn known_header_parse() {
        let mut header_bytes = owned_stream(vec![0xD1, 0x01, 0x0D, 0x55, 0x02]);
        let header: NdefHeader = parse_header(&mut header_bytes).unwrap();

        assert!(header.message_begin);
        assert!(header.message_end);
        assert!(!header.chunked);
        assert!(header.short_record);
        assert!(!header.has_id_length);
        assert_eq!(header.type_name_format, NdefType::WellKnown);
        assert_eq!(header.type_length, 1);
        assert_eq!(header.payload_length, 13);
    }

    #[test]
    fn test_header_parsing_with_complete_data() {
        // export
        let export = &EXPORT[0..3043];
        let mut data = super::stream::new(export);
        assert_eq!(data.len(), export.len());

        let message_info = parse_message_info(&mut data).unwrap();
        assert_eq!(message_info.full_message_length, 3043);

        assert_eq!(export.len(), message_info.full_message_length as usize);

        let header = parse_header(&mut data).unwrap();
        assert!(header.message_begin);
        assert!(header.message_end);
        assert!(!header.chunked);
        assert!(!header.short_record);
        assert!(!header.has_id_length);
        assert_eq!(header.type_name_format, NdefType::Mime);
        assert_eq!(header.type_length, 16);
        assert_eq!(header.payload_length, 3009);

        // descriptor
        let mut data = *DESCRIPTOR;
        let message_info = parse_message_info(&mut data).unwrap();
        assert_eq!(message_info.full_message_length, 171);

        let header = parse_header(&mut data).unwrap();
        assert!(header.message_begin);
        assert!(header.message_end);
        assert!(!header.chunked);
        assert!(header.short_record);
        assert!(!header.has_id_length);
        assert_eq!(header.type_name_format, NdefType::WellKnown);
        assert_eq!(header.type_length, 1);
        assert_eq!(header.payload_length, 157)
    }

    #[test]
    fn test_header_parsing_with_incomplete_data() {
        let data = descriptor_bytes();
        let header = parse_header(&mut super::stream::new(&data[0..6])).unwrap();

        assert!(header.message_end);
        assert!(header.message_begin);
        assert!(header.message_end);
        assert!(!header.chunked);
        assert!(header.short_record);
        assert!(!header.has_id_length);
        assert_eq!(header.type_name_format, NdefType::WellKnown);
        assert_eq!(header.type_length, 1);
        assert_eq!(header.payload_length, 157)
    }

    #[test]
    fn parse_type_with_complete_data() {
        // export
        let mut data = export_bytes();
        let header = parse_header(&mut data).unwrap();
        let type_ = parse_type(&mut data, header.type_length).unwrap();
        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "application/json");

        // descriptor
        let mut data = descriptor_bytes();
        let header = parse_header(&mut data).unwrap();
        let type_ = parse_type(&mut data, header.type_length).unwrap();
        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "T");
    }
    //
    #[test]
    fn parse_payload_with_complete_data_descriptor() {
        // let record_1
        let mut data = descriptor_bytes();
        let data = &mut data;

        let header = parse_header(data).unwrap();
        let type_ = parse_type(data, header.type_length).unwrap();
        let id = parse_id(data, header.id_length).unwrap();
        let payload = parse_payload(data, header.payload_length, &type_).unwrap();

        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "T".to_string());
        assert_eq!(id, None);

        let NdefPayload::Text(payload_string) = payload else {
            panic!("payload is not text")
        };

        let descriptor_string = std::fs::read_to_string("../../../../test/data/descriptor.txt")
            .unwrap()
            .trim()
            .to_string();

        assert_eq!(payload_string.text, descriptor_string);
    }

    #[test]
    fn parse_payload_with_complete_data_export() {
        let mut data = export_bytes();
        let data = &mut data;

        let header = parse_header(data).unwrap();
        let type_ = parse_type(data, header.type_length).unwrap();
        let _id = parse_id(data, header.id_length).unwrap();
        let payload = parse_payload(data, header.payload_length, &type_).unwrap();

        let NdefPayload::Data(payload) = payload else { panic!("payload is not data") };

        let payload_string = String::from_utf8(payload).unwrap();
        let export_string = std::fs::read_to_string("../../../../test/data/export.json").unwrap();

        let payload_json = serde_json::from_str::<serde_json::Value>(&payload_string).unwrap();
        let export_json = serde_json::from_str::<serde_json::Value>(&export_string).unwrap();

        assert_eq!(payload_json, export_json);
    }

    #[test]
    fn verify_parsing_keeps_track_of_bytes_left_over() {
        let mut export = *EXPORT;
        let export_len = export.len();
        let _message = parse_ndef_message(&mut export).unwrap();

        assert_ne!(export.len(), export_len);
        assert!(export_len > export.len());
    }

    #[test]
    fn test_getting_entire_ndef_message_export() {
        let mut export = *EXPORT;
        let message = parse_ndef_message(&mut export).unwrap();
        assert_eq!(message.len(), 1);

        let record = &message[0];
        assert_eq!(record.type_, b"application/json");

        let NdefPayload::Data(payload) = &record.payload else {
            panic!("payload is not data")
        };

        let export_string = std::fs::read_to_string("../../../../test/data/export.json").unwrap();
        let export_json = serde_json::from_str::<serde_json::Value>(&export_string).unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(payload).unwrap(),
            export_json
        );
    }

    #[test]
    fn test_getting_entire_ndef_message_descriptor() {
        let mut descriptor = *DESCRIPTOR;
        let message = parse_ndef_message(&mut descriptor).unwrap();

        assert_eq!(message.len(), 1);

        let record = &message[0];
        assert_eq!(record.type_, b"T");

        let known_descriptor_string = std::fs::read_to_string("../../../../test/data/descriptor.txt")
            .unwrap()
            .trim()
            .to_string();

        let NdefPayload::Text(payload_string) = &record.payload else {
            panic!("payload is not text")
        };

        assert_eq!(payload_string.text, known_descriptor_string);
    }

    #[test]
    fn test_partial_parsing() {
        let original_length = EXPORT.len();
        let mut export = owned_stream((*EXPORT)[..100].to_vec());
        let message = parse_ndef_message(&mut export);

        assert!(message.is_err());
        assert!(matches!(message, Err(ErrMode::Incomplete(Needed::Size(_)))));

        if let Err(ErrMode::Incomplete(needed)) = message {
            assert_eq!(needed, Needed::new(original_length - 101));
        }
    }

    #[test]
    fn test_parse_partial_in_chunks() {
        let mut data = Vec::new();
        let mut chunks_processed = 0;

        EXPORT.chunks(100).for_each(|chunk| {
            let mut chunk_data = std::mem::take(&mut data);
            chunk_data.extend_from_slice(chunk);

            let mut stream = Stream::new(Bytes::new(&chunk_data));

            match parse_ndef_message(&mut stream) {
                Ok(message) => {
                    assert_eq!(message.len(), 1)
                }

                Err(e) if e.is_incomplete() => {
                    chunks_processed += 1;
                    data = chunk_data;
                }

                Err(e) => panic!("unexpected error: {e}"),
            };
        });

        assert_eq!(chunks_processed, 30);
    }

    #[test]
    fn test_message_info_payload_length_accuracy() {
        // seed
        let mut data = *SEED_WORDS_BYTES;
        let info = parse_message_info(&mut data).unwrap();
        let payload_length = info.full_message_length as usize;

        let export_vec = SEED_WORDS_BYTES.to_vec();
        let mut data_trunc = stream::new(&export_vec[..payload_length]);
        let info = parse_message_info(&mut data_trunc).unwrap();
        let parsed = parse_ndef_records(&mut data_trunc, &info);
        assert!(parsed.is_ok());

        // export
        let mut data = *EXPORT;
        let info = parse_message_info(&mut data).unwrap();
        let payload_length = info.full_message_length as usize;

        let export_vec = EXPORT.to_vec();
        let mut data_trunc = stream::new(&export_vec[..payload_length]);
        let info = parse_message_info(&mut data_trunc).unwrap();
        let parsed = parse_ndef_records(&mut data_trunc, &info);
        assert!(parsed.is_ok());

        // descriptor
        let mut data = *DESCRIPTOR;
        let info = parse_message_info(&mut data).unwrap();
        let payload_length = info.full_message_length as usize;

        let export_vec = DESCRIPTOR.to_vec();
        let mut data_trunc = stream::new(&export_vec[..payload_length]);
        let info = parse_message_info(&mut data_trunc).unwrap();
        let parsed = parse_ndef_records(&mut data_trunc, &info);
        assert!(parsed.is_ok());
    }
}
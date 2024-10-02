use winnow::{
    binary::{
        be_u16, be_u8,
        bits::{bool as take_bool, take as take_bits},
        Endianness,
    },
    error::{ErrMode, ErrorKind, ParserError as _},
    stream::{Bytes, Partial},
    token::{any, literal, take},
    IResult, PResult, Parser,
};

use crate::{
    header::NdefHeader,
    ndef_type::NdefType,
    payload::{NdefPayload, TextPayload, TextPayloadFormat},
    record::NdefRecord,
};

#[derive(Debug)]
pub struct MessageInfo {
    pub total_payload_length: u16,
}

type Stream<'i> = Partial<&'i Bytes>;

pub fn stream(b: &[u8]) -> Stream<'_> {
    Partial::new(Bytes::new(b))
}

pub fn parse_message_info(input: Stream<'_>) -> IResult<Stream<'_>, MessageInfo> {
    let (input, _) = literal([226, 67, 0, 1, 0, 0, 4, 0, 3]).parse_peek(input)?;

    let (input, length_indicator) = be_u8.parse_peek(input)?;

    let (input, total_payload_length) = if length_indicator == 255 {
        be_u16.parse_peek(input)?
    } else {
        (input, length_indicator as u16)
    };

    Ok((
        input,
        MessageInfo {
            total_payload_length,
        },
    ))
}

pub fn parse_ndef_record(input: Stream<'_>) -> IResult<Stream<'_>, NdefRecord> {
    let (input, header) = parse_header(input)?;

    let (input, type_) = parse_type(input, header.type_length)?;
    let (input, id) = parse_id(input, header.id_length)?;
    let (input, payload) = parse_payload(input, header.payload_length, &type_)?;

    Ok((
        input,
        NdefRecord {
            header,
            type_,
            id,
            payload,
        },
    ))
}

pub fn parse_ndef_message(input: Stream<'_>) -> IResult<Stream<'_>, Vec<NdefRecord>> {
    let (input, info) = parse_message_info(input)?;

    let mut records = Vec::new();

    let payload_length = info.total_payload_length as usize;
    let mut total_parsed_bytes = 0;

    loop {
        let input_start_bytes = input.len();
        let (input, record) = parse_ndef_record(input)?;
        records.push(record);

        let parsed_bytes = input_start_bytes - input.len();
        total_parsed_bytes += parsed_bytes;

        // parsed all bytes dictated by payload_length
        if total_parsed_bytes >= payload_length && !records.is_empty() {
            break;
        }
    }

    Ok((input, records))
}

// private

fn parse_header(input: Stream<'_>) -> IResult<Stream<'_>, NdefHeader> {
    let (input, first_byte) = any.parse_peek(input)?;

    let first_byte = [first_byte];
    let first_byte = stream(&first_byte);

    let parse_byte = |first_byte: Stream<'_>| -> PResult<(bool, bool, bool, bool, bool, u8)> {
        let (first_byte, message_begin) = take_bool.parse_peek((first_byte, 0))?;
        let (first_byte, message_end) = take_bool.parse_peek(first_byte)?;
        let (first_byte, chunked) = take_bool.parse_peek(first_byte)?;
        let (first_byte, short_record) = take_bool.parse_peek(first_byte)?;
        let (first_byte, has_id_length) = take_bool.parse_peek(first_byte)?;
        let (_, type_name_format): (_, u8) = take_bits(3_u8).parse_peek(first_byte)?;

        Ok((
            message_begin,
            message_end,
            chunked,
            short_record,
            has_id_length,
            type_name_format,
        ))
    };

    let (message_begin, message_end, chunked, short_record, has_id_length, type_name_format) =
        parse_byte(first_byte).map_err(|_e| {
            let error_kind = ErrorKind::Fail;
            let error = ErrMode::from_error_kind(&input, error_kind);
            error
        })?;

    let (input, type_length) = winnow::binary::u8.parse_peek(input)?;

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

    let (input, payload_length) = if short_record {
        any.map(|x: u8| x as u32).parse_peek(input)?
    } else {
        winnow::binary::u32(Endianness::Big).parse_peek(input)?
    };

    let (input, id_length) = if has_id_length {
        let (input, id_length) = any.parse_peek(input)?;
        (input, Some(id_length))
    } else {
        (input, None)
    };

    Ok((
        input,
        NdefHeader {
            message_begin,
            message_end,
            chunked,
            short_record,
            has_id_length,
            type_name_format,
            type_length,
            payload_length,
            id_length,
        },
    ))
}

fn parse_type(input: Stream<'_>, type_length: u8) -> IResult<Stream<'_>, Vec<u8>> {
    take(type_length as usize)
        .map(|s: &[u8]| s.to_vec())
        .parse_peek(input)
}

fn parse_id(input: Stream<'_>, id_length: Option<u8>) -> IResult<Stream<'_>, Option<Vec<u8>>> {
    if let Some(id_len) = id_length {
        take(id_len as usize)
            .map(|s: &[u8]| Some(s.to_vec()))
            .parse_peek(input)
    } else {
        Ok((input, None))
    }
}

fn parse_payload<'a, 'b>(
    input: Stream<'a>,
    payload_length: u32,
    type_: &'b [u8],
) -> IResult<Stream<'a>, NdefPayload> {
    if type_ == b"T" {
        let (input, next_byte) = any.parse_peek(input)?;
        let next_byte = [next_byte];
        let next_byte = stream(&next_byte);

        let parse_byte = |next_byte: Stream<'_>| -> PResult<(bool, u8)> {
            let (mut next_byte, is_utf16) = take_bool.parse_peek((next_byte, 0_usize))?;
            let language_code_length = take_bits(7_u8).parse_next(&mut next_byte)?;

            Ok((is_utf16, language_code_length))
        };

        let (is_utf16, language_code_length) = parse_byte(next_byte).map_err(|_e| {
            let error_kind = ErrorKind::Fail;
            let error = ErrMode::from_error_kind(&input, error_kind);
            error
        })?;

        let (input, language_code) = take(language_code_length as usize).parse_peek(input)?;
        let remaining_length = payload_length - language_code_length as u32 - 1;
        let (input, text) = take(remaining_length as usize).parse_peek(input)?;

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

        Ok((input, NdefPayload::Text(parsed_text)))
    } else {
        let (input, payload) = take(payload_length)
            .map(|s: &[u8]| NdefPayload::Data(s.to_vec()))
            .parse_peek(input)?;

        Ok((input, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ndef_type::NdefType, payload::NdefPayload};
    use std::sync::LazyLock;
    use winnow::error::Needed;

    fn owned_stream(bytes: Vec<u8>) -> Stream<'static> {
        let bytes = Box::leak(bytes.into_boxed_slice());
        Stream::new(Bytes::new(bytes))
    }

    static EXPORT: LazyLock<Stream<'static>> = LazyLock::new(|| {
        let file_contents = include_bytes!("../test/data/export_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        owned_stream(bytes)
    });

    static DESCRIPTOR: LazyLock<Stream<'static>> = LazyLock::new(|| {
        let file_contents = include_bytes!("../test/data/descriptor_bytes.txt");
        let file_string = String::from_utf8(file_contents.to_vec()).unwrap();

        let bytes: Vec<u8> = file_string
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<u8>().unwrap())
            .collect();

        owned_stream(bytes)
    });

    fn export_bytes() -> Stream<'static> {
        let (data, _payload_length) = parse_message_info(*EXPORT).unwrap();
        data
    }

    fn descriptor_bytes() -> Stream<'static> {
        let data = *DESCRIPTOR;
        let (data, _payload_length) = parse_message_info(data).unwrap();
        data
    }

    #[test]
    fn known_header_parse() {
        let header_bytes = owned_stream(vec![0xD1, 0x01, 0x0D, 0x55, 0x02]);
        let (_, header) = parse_header(header_bytes).unwrap();

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
        let data = *EXPORT;
        let (data, message_info) = parse_message_info(data).unwrap();
        assert_eq!(message_info.total_payload_length, 3031);

        let (_, header) = parse_header(data).unwrap();
        assert!(header.message_begin);
        assert!(header.message_end);
        assert!(!header.chunked);
        assert!(!header.short_record);
        assert!(!header.has_id_length);
        assert_eq!(header.type_name_format, NdefType::Mime);
        assert_eq!(header.type_length, 16);
        assert_eq!(header.payload_length, 3009);

        // descriptor
        let data = *DESCRIPTOR;
        let (data, message_info) = parse_message_info(data).unwrap();
        assert_eq!(message_info.total_payload_length, 161);

        let (_data, header) = parse_header(data).unwrap();
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
        let (_, header) = parse_header(stream(&data[0..6])).unwrap();

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
        let data = export_bytes();
        let (data, header) = parse_header(data).unwrap();
        let (_data, type_) = parse_type(data, header.type_length).unwrap();
        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "application/json");

        // descriptor
        let data = descriptor_bytes();
        let (data, header) = parse_header(data).unwrap();
        let (_data, type_) = parse_type(data, header.type_length).unwrap();
        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "T");
    }
    //
    #[test]
    fn parse_payload_with_complete_data_descriptor() {
        // let record_1
        let data = descriptor_bytes();

        let (data, header) = parse_header(data).unwrap();
        let (data, type_) = parse_type(data, header.type_length).unwrap();
        let (data, id) = parse_id(data, header.id_length).unwrap();
        let (_data, payload) = parse_payload(data, header.payload_length, &type_).unwrap();

        let type_string = String::from_utf8(type_).unwrap();
        assert_eq!(type_string, "T".to_string());
        assert_eq!(id, None);

        let NdefPayload::Text(payload_string) = payload else {
            panic!("payload is not text")
        };

        let descriptor_string = std::fs::read_to_string("test/data/descriptor.txt")
            .unwrap()
            .trim()
            .to_string();

        assert_eq!(payload_string.text, descriptor_string);
    }

    #[test]
    fn parse_payload_with_complete_data_export() {
        let data = export_bytes();

        let (data, header) = parse_header(data).unwrap();
        let (data, type_) = parse_type(data, header.type_length).unwrap();
        let (data, _id) = parse_id(data, header.id_length).unwrap();
        let (_data, payload) = parse_payload(data, header.payload_length, &type_).unwrap();

        let NdefPayload::Data(payload) = payload else {
            panic!("payload is not data")
        };

        let payload_string = String::from_utf8(payload).unwrap();
        let export_string = std::fs::read_to_string("test/data/export.json").unwrap();

        let payload_json = serde_json::from_str::<serde_json::Value>(&payload_string).unwrap();
        let export_json = serde_json::from_str::<serde_json::Value>(&export_string).unwrap();

        assert_eq!(payload_json, export_json);
    }

    #[test]
    fn verify_parsing_keeps_track_of_bytes_left_over() {
        let export = *EXPORT;
        let export_len = export.len();
        let (data, _message) = parse_ndef_message(export).unwrap();

        assert_ne!(export.len(), data.len());
        assert!(export_len > data.len());
    }

    #[test]
    fn test_getting_entire_ndef_message_export() {
        let export = *EXPORT;
        let (_data, message) = parse_ndef_message(export).unwrap();
        assert_eq!(message.len(), 1);

        let record = &message[0];
        assert_eq!(record.type_, b"application/json");

        let NdefPayload::Data(payload) = &record.payload else {
            panic!("payload is not data")
        };

        let export_string = std::fs::read_to_string("test/data/export.json").unwrap();
        let export_json = serde_json::from_str::<serde_json::Value>(&export_string).unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(payload).unwrap(),
            export_json
        );
    }

    #[test]
    fn test_getting_entire_ndef_message_descriptor() {
        let descriptor = *DESCRIPTOR;
        let (_data, message) = parse_ndef_message(descriptor).unwrap();

        assert_eq!(message.len(), 1);

        let record = &message[0];
        assert_eq!(record.type_, b"T");

        let known_descriptor_string = std::fs::read_to_string("test/data/descriptor.txt")
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
        let export = owned_stream((*EXPORT)[..100].to_vec());
        let message = parse_ndef_message(export);

        assert!(matches!(
            message,
            Err(winnow::error::ErrMode::Incomplete(Needed::Size(_)))
        ));

        if let Err(winnow::error::ErrMode::Incomplete(needed)) = message {
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

            let stream = Stream::new(Bytes::new(&chunk_data));

            match parse_ndef_message(stream) {
                Ok((_, message)) => {
                    assert_eq!(message.len(), 1);
                    return;
                }
                Err(winnow::error::ErrMode::Incomplete(Needed::Size(_))) => {
                    chunks_processed += 1;
                    data = chunk_data;
                }

                Err(e) => panic!("unexpected error: {e}"),
            };
        });

        assert_eq!(chunks_processed, 30);
    }
}

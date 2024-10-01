use nom::{
    bits::{bits, streaming::take as take_bits},
    combinator::map,
    number::streaming::{be_u32, be_u8},
    sequence::tuple,
    IResult,
};

#[derive(Debug)]
struct NdefRecord<'a> {
    message_begin: bool,
    message_end: bool,
    chunk_flag: bool,
    short_record: bool,
    id_length_present: bool,
    type_name_format: u8,
    type_length: u8,
    payload_length: u32,
    id_length: Option<u8>,
    type_: &'a [u8],
    id: Option<&'a [u8]>,
    payload: &'a [u8],
}

fn parse_header(input: &[u8]) -> IResult<&[u8], (bool, bool, bool, bool, bool, u8)> {
    bits::<_, _, nom::error::Error<(&[u8], usize)>, _, _>(map(
        tuple((
            take_bits(1u8),
            take_bits(1u8),
            take_bits(1u8),
            take_bits(1u8),
            take_bits(1u8),
            take_bits(3u8),
        )),
        |(a, b, c, d, e, f): (u8, u8, u8, u8, u8, u8)| (a != 0, b != 0, c != 0, d != 0, e != 0, f),
    ))(input)
}

fn parse_ndef_record(input: &[u8]) -> IResult<&[u8], NdefRecord> {
    let (
        input,
        (message_begin, message_end, chunk_flag, short_record, id_length_present, type_name_format),
    ) = parse_header(input)?;

    let (input, type_length) = be_u8(input)?;

    let (input, payload_length) = if short_record {
        let (input, len) = be_u8(input)?;
        (input, len as u32)
    } else {
        be_u32(input)?
    };

    let (input, id_length) = if id_length_present {
        let (input, len) = be_u8(input)?;
        (input, Some(len))
    } else {
        (input, None)
    };

    let (input, type_) = nom::bytes::streaming::take(type_length as usize)(input)?;

    let (input, id) = if let Some(len) = id_length {
        let (input, id) = nom::bytes::streaming::take(len as usize)(input)?;
        (input, Some(id))
    } else {
        (input, None)
    };

    let (input, payload) = nom::bytes::streaming::take(payload_length as usize)(input)?;

    Ok((
        input,
        NdefRecord {
            message_begin,
            message_end,
            chunk_flag,
            short_record,
            id_length_present,
            type_name_format,
            type_length,
            payload_length,
            id_length,
            type_,
            id,
            payload,
        },
    ))
}

fn parse_ndef_message(input: &[u8]) -> IResult<&[u8], Vec<NdefRecord>> {
    nom::multi::many1(parse_ndef_record)(input)
}

fn main() {
    // Example usage with incomplete data
    let incomplete_ndef_data = vec![
        0xD1, 0x01, 0x0E, 0x55, 0x03, 0x67, 0x6F, 0x6F,
        // ... missing rest of the data
    ];

    match parse_ndef_message(&incomplete_ndef_data) {
        Ok((remaining, records)) => {
            println!("Parsed NDEF records: {:?}", records);
            println!("Remaining data: {:?}", remaining);
        }
        Err(nom::Err::Incomplete(needed)) => {
            println!("Need more data: {:?}", needed);
        }
        Err(e) => println!("Error: {:?}", e),
    }
}

fn process_stream(mut data: &[u8]) {
    loop {
        match parse_ndef_message(data) {
            Ok((remaining, records)) => {
                println!("Parsed records: {:?}", records);
                data = remaining;
            }
            Err(nom::Err::Incomplete(_)) => {
                println!("Need more data");
                break; // Wait for more data
            }
            Err(e) => {
                println!("Error: {:?}", e);
                break; // Handle error
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const DESCRIPTOR: [u8; 1020] = [
        226, 67, 0, 1, 0, 0, 4, 0, 3, 161, 209, 1, 157, 84, 2, 101, 110, 119, 112, 107, 104, 40,
        91, 56, 49, 55, 101, 55, 98, 101, 48, 47, 56, 52, 104, 47, 48, 104, 47, 48, 104, 93, 120,
        112, 117, 98, 54, 67, 105, 75, 110, 87, 118, 55, 80, 80, 121, 121, 101, 98, 52, 107, 67,
        119, 75, 52, 102, 105, 100, 75, 113, 86, 106, 80, 102, 68, 57, 84, 80, 54, 77, 105, 88,
        110, 122, 66, 86, 71, 90, 89, 78, 97, 110, 78, 100, 89, 51, 109, 77, 118, 121, 119, 99,
        114, 100, 68, 99, 54, 119, 75, 56, 50, 106, 121, 66, 83, 100, 57, 53, 118, 115, 107, 50,
        54, 81, 117, 106, 110, 74, 87, 80, 114, 83, 97, 80, 102, 89, 101, 121, 87, 55, 78, 121, 88,
        51, 55, 72, 72, 71, 116, 102, 81, 77, 47, 60, 48, 59, 49, 62, 47, 42, 41, 35, 54, 48, 116,
        106, 115, 52, 99, 55, 254, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 114, 105, 118, 34, 58, 32, 34, 109, 47, 52, 52, 104, 47, 48, 104, 47, 48, 104, 34,
        44, 32, 34, 120, 112, 117, 98, 34, 58, 32, 34, 120, 112, 117, 98, 54, 66, 111, 75, 78, 49,
        52, 74, 122, 83, 70, 78, 49, 84, 51, 99, 113, 101, 57, 70, 110, 114, 119, 110, 88, 71, 65,
        115, 109, 98, 103, 69, 84, 74, 121, 101, 97, 122, 111, 97, 51, 70, 55, 97, 77, 88, 104, 52,
        88, 110, 100, 118, 86, 114, 74, 65, 89, 121, 77, 49, 50, 55, 70, 115, 114, 72, 56, 75, 70,
        118, 53, 88, 70, 88, 68, 114, 111, 113, 88, 78, 102, 90, 77, 102, 115, 105, 110, 111, 119,
        55, 120, 112, 57, 51, 117, 101, 89, 83, 112, 110, 114, 106, 66, 66, 70, 115, 52, 34, 44,
        32, 34, 100, 101, 115, 99, 34, 58, 32, 34, 112, 107, 104, 40, 91, 56, 49, 55, 101, 55, 98,
        101, 48, 47, 52, 52, 104, 47, 48, 104, 47, 48, 104, 93, 120, 112, 117, 98, 54, 66, 111, 75,
        78, 49, 52, 74, 122, 83, 70, 78, 49, 84, 51, 99, 113, 101, 57, 70, 110, 114, 119, 110, 88,
        71, 65, 115, 109, 98, 103, 69, 84, 74, 121, 101, 97, 122, 111, 97, 51, 70, 55, 97, 77, 88,
        104, 52, 88, 110, 100, 118, 86, 114, 74, 65, 89, 121, 77, 49, 50, 55, 70, 115, 114, 72, 56,
        75, 70, 118, 53, 88, 70, 88, 68, 114, 111, 113, 88, 78, 102, 90, 77, 102, 115, 105, 110,
        111, 119, 55, 120, 112, 57, 51, 117, 101, 89, 83, 112, 110, 114, 106, 66, 66, 70, 115, 52,
        47, 60, 48, 59, 49, 62, 47, 42, 41, 35, 116, 100, 116, 114, 108, 51, 121, 57, 34, 44, 32,
        34, 102, 105, 114, 115, 116, 34, 58, 32, 34, 49, 70, 74, 82, 52, 68, 70, 78, 87, 69, 110,
        75, 71, 81, 87, 72, 106, 109, 53, 121, 88, 75, 72, 97, 111, 56, 52, 74, 80, 71, 103, 71,
        80, 110, 34, 125, 44, 32, 34, 98, 105, 112, 52, 57, 34, 58, 32, 123, 34, 110, 97, 109, 101,
        34, 58, 32, 34, 112, 50, 115, 104, 45, 112, 50, 119, 112, 107, 104, 34, 44, 32, 34, 120,
        102, 112, 34, 58, 32, 34, 57, 55, 56, 51, 67, 68, 53, 69, 34, 44, 32, 34, 100, 101, 114,
        105, 118, 34, 58, 32, 34, 109, 47, 52, 57, 104, 47, 48, 104, 47, 48, 104, 34, 44, 32, 34,
        120, 112, 117, 98, 34, 58, 32, 34, 120, 112, 117, 98, 54, 67, 67, 75, 65, 118, 85, 84, 78,
        117, 114, 115, 69, 110, 97, 74, 56, 107, 49, 100, 50, 55, 76, 102, 113, 69, 85, 122, 101,
        65, 120, 50, 78, 57, 119, 70, 113, 89, 69, 51, 87, 49, 120, 104, 55, 110, 113, 103, 74, 69,
        66, 69, 98, 76, 83, 83, 109, 111, 104, 119, 68, 120, 122, 115, 83, 118, 99, 115, 89, 113,
        105, 81, 113, 70, 122, 82, 118, 116, 97, 54, 53, 78, 106, 98, 101, 53, 111, 56, 52, 98, 70,
        53, 89, 88, 72, 70, 113, 102, 83, 72, 50, 68, 107, 104, 111, 110, 109, 34, 44, 32, 34, 100,
        101, 115, 99, 34, 58, 32, 34, 115, 104, 40, 119, 112, 107, 104, 40, 91, 56, 49, 55, 101,
        55, 98, 101, 48, 47, 52, 57, 104, 47, 48, 104, 47, 48, 104, 93, 120, 112, 117, 98, 54, 67,
        67, 75, 65, 118, 85, 84, 78, 117, 114, 115, 69, 110, 97, 74, 56, 107, 49, 100, 50, 55, 76,
        102, 113, 69, 85, 122, 101, 65, 120, 50, 78, 57, 119, 70, 113, 89, 69, 51, 87, 49, 120,
        104, 55, 110, 113, 103, 74, 69, 66, 69, 98, 76, 83, 83, 109, 111, 104, 119, 68, 120, 122,
        115, 83, 118, 99, 115, 89, 113, 105, 81, 113, 70, 122, 82, 118, 116, 97, 54, 53, 78, 106,
        98, 101, 53, 111, 56, 52, 98, 70, 53, 89, 88, 72, 70, 113, 102, 83, 72, 50, 68, 107, 104,
        111, 110, 109, 47, 60, 48, 59, 49, 62, 47, 42, 41, 41, 35, 56, 108, 108, 109, 116, 51, 54,
        120, 34, 44, 32, 34, 95, 112, 117, 98, 34, 58, 32, 34, 121, 112, 117, 98, 54, 88, 50, 97,
        85, 98, 57, 78, 88, 98, 81, 77, 54, 53, 109, 81, 121, 54, 111, 70, 69, 67, 83, 66, 49,
    ];

    // #[test]
    // fn test_read_header_and_payload_length() {
    //     let mut reader = NfcReader::new();
    //
    //     reader
    //         .push_bytes(DESCRIPTOR[..4].try_into().unwrap())
    //         .unwrap();
    //
    //     assert_eq!(
    //         reader.state,
    //         NfcReaderState::ReadingPayloadLength {
    //             header: InitialHeader {
    //                 is_message_begin: true,
    //                 is_message_end: true,
    //                 is_chunked: true,
    //                 is_short_record: false,
    //                 has_id_length: false,
    //                 type_name_format: 2,
    //                 type_length: 67,
    //             },
    //             payload_length: [0, 0, 1, 0],
    //         }
    //     );
    // }
    //
    // #[test]
    // fn test_read_complete_payload_length() {
    //     let mut reader = NfcReader::new();
    //
    //     reader
    //         .push_bytes(DESCRIPTOR[..4].try_into().unwrap())
    //         .unwrap();
    //
    //     reader
    //         .push_bytes(DESCRIPTOR[4..8].try_into().unwrap())
    //         .unwrap();
    //
    //     assert_eq!(
    //         reader.state,
    //         NfcReaderState::ReadingType {
    //             header: NfcHeader {
    //                 is_message_begin: true,
    //                 is_message_end: true,
    //                 is_chunked: true,
    //                 is_short_record: false,
    //                 has_id_length: false,
    //                 type_name_format: 2,
    //                 type_length: 67,
    //                 payload_length: 256,
    //                 id_length: None,
    //             },
    //             type_name: vec![4, 0]
    //         }
    //     );
    // }
    //
    // #[test]
    // fn test_reader_initial_header() {
    //     let initial_header = InitialHeader::try_new_from_bytes(DESCRIPTOR.as_slice());
    //
    //     assert!(initial_header.is_ok());
    //     let initial_header = initial_header.unwrap();
    //
    //     assert!(initial_header.is_message_begin);
    //     assert!(initial_header.is_message_end);
    //     assert!(initial_header.is_chunked);
    //     assert!(!initial_header.is_short_record);
    //     assert!(!initial_header.has_id_length);
    //     assert_eq!(initial_header.type_name_format, 2);
    //     assert_eq!(initial_header.type_length, 67);
    // }
}

use nom::{
    bits::streaming::{tag, take},
    branch::alt,
    number::streaming::{be_u16, be_u32, be_u8},
    sequence::tuple,
    IResult, Needed,
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
    nom::bits::<_, _, nom::error::Error<(&[u8], usize)>, _, _>(tuple((
        take(1usize),
        take(1usize),
        take(1usize),
        take(1usize),
        take(1usize),
        take(3_u8),
    )))(input)
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

//! Animated multi-part URs that keep emitting fountain parts
//!
//! BCR-2020-005 numbers a message's plain fragments `1-N` to `N-N`, and lets a
//! sender go on past `N` with parts that XOR a pseudo-random mix of fragments.
//! A sender that loops only the plain fragments stalls a receiver that keeps
//! missing one of them, since nothing but that frame can fill its gap, while a
//! mixed part that holds the missing fragment fills it from the fragments the
//! receiver already has. See
//! <https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2024-001-multipart-ur.md>

use std::collections::BTreeSet;

use foundation_ur::{
    UR,
    fountain::{chooser::Alloc, part::Part},
};

use crate::error::{Result, UrError};

/// The frames of an animated UR, any of which can be computed from its index
///
/// Frames `0..sequence_count` carry the plain fragments in order, and every
/// later frame a fountain-mixed part, so a display shows frame `i` on its
/// `i`th tick without end rather than looping the plain fragments
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct UrFountain {
    ur_type: String,
    /// The message, zero-padded to whole fragments as the fountain encoder pads it
    padded_message: Vec<u8>,
    message_length: usize,
    fragment_length: usize,
    sequence_count: u32,
    checksum: u32,
}

impl UrFountain {
    /// Splits `message` into fragments of at most `max_fragment_length` bytes
    ///
    /// # Errors
    /// Returns `UrError::InvalidField` if `ur_type` or `message` is empty or
    /// `max_fragment_length` is zero, which the fountain encoder can't split
    pub fn new(ur_type: &str, message: &[u8], max_fragment_length: usize) -> Result<Self> {
        if ur_type.is_empty() || message.is_empty() || max_fragment_length == 0 {
            return Err(UrError::InvalidField(format!(
                "a UR needs a type, a message and a fragment length, got type `{ur_type}`, \
                 {} bytes and fragment length {max_fragment_length}",
                message.len()
            )));
        }

        // the first part carries what every part of the message shares, as foundation-ur derives
        // it, so the parts computed here match its encoder's byte for byte
        let mut encoder = foundation_ur::Encoder::new();
        encoder.start(ur_type, message, max_fragment_length);

        let UR::MultiPartDeserialized { fragment: first, .. } = encoder.next_part() else {
            unreachable!("the fountain encoder emits multi-part URs");
        };

        let fragment_length = first.data.len();
        let sequence_count = first.sequence_count;
        let checksum = first.checksum;

        let mut padded_message = message.to_vec();
        padded_message.resize(fragment_length * sequence_count as usize, 0);

        Ok(Self {
            ur_type: ur_type.to_string(),
            padded_message,
            message_length: message.len(),
            fragment_length,
            sequence_count,
            checksum,
        })
    }

    /// The fountain part at a 1-based sequence number
    fn part_data(&self, sequence: u32) -> Vec<u8> {
        let part = Part {
            sequence,
            sequence_count: self.sequence_count,
            message_length: self.message_length,
            checksum: self.checksum,
            data: &[],
        };

        let mut data = vec![0; self.fragment_length];
        for index in part.indexes::<Alloc, BTreeSet<usize>>() {
            let start = index * self.fragment_length;
            let fragment = &self.padded_message[start..start + self.fragment_length];

            for (byte, fragment_byte) in data.iter_mut().zip(fragment) {
                *byte ^= fragment_byte;
            }
        }

        data
    }
}

#[uniffi::export]
impl UrFountain {
    /// How many plain fragments the message splits into, the frames one pass of a display needs
    #[must_use]
    pub const fn sequence_count(&self) -> u32 {
        self.sequence_count
    }

    /// The UR to show on a display's `index`th tick, counting from 0
    ///
    /// The frame is uppercase, as BCR-2020-005 recommends for QR transport, so a QR encoder can
    /// draw it in the denser alphanumeric mode instead of byte mode. A message that fits one
    /// fragment shows its one `1-1` part on every tick. The sequence number wraps after
    /// `u32::MAX` ticks, the largest a UR carries
    #[must_use]
    pub fn frame(&self, index: u32) -> String {
        let sequence = if self.sequence_count == 1 { 1 } else { index % u32::MAX + 1 };
        let data = self.part_data(sequence);

        let ur = UR::MultiPartDeserialized {
            ur_type: &self.ur_type,
            fragment: Part {
                sequence,
                sequence_count: self.sequence_count,
                message_length: self.message_length,
                checksum: self.checksum,
                data: &data,
            },
        };

        ur.to_string().to_ascii_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use foundation_ur::Decoder;

    use super::*;

    fn message(len: usize) -> Vec<u8> {
        (0..len).map(|byte| (byte * 31 % 251) as u8).collect()
    }

    #[test]
    fn frames_match_the_foundation_encoder_past_the_plain_fragments() {
        let message = message(1_000);
        let fountain = UrFountain::new("bytes", &message, 60).unwrap();
        assert_eq!(fountain.sequence_count(), 17);

        let mut encoder = foundation_ur::Encoder::new();
        encoder.start("bytes", &message, 60);

        for index in 0..4 * fountain.sequence_count() {
            let expected = encoder.next_part().to_string().to_ascii_uppercase();
            assert_eq!(fountain.frame(index), expected, "frame {index}");
        }
    }

    #[test]
    fn a_receiver_that_never_sees_one_plain_fragment_completes_from_mixed_parts() {
        let message = message(1_000);
        let fountain = UrFountain::new("bytes", &message, 60).unwrap();
        let count = fountain.sequence_count();

        let mut decoder = Decoder::default();
        let mut frames = 0;

        // three passes' worth of ticks, every one but the one showing fragment 5
        for index in (0..3 * count).filter(|index| index != &4) {
            let frame = fountain.frame(index).to_ascii_lowercase();
            decoder.receive(UR::parse(&frame).unwrap()).unwrap();
            frames += 1;

            if decoder.is_complete() {
                break;
            }
        }

        assert!(decoder.is_complete(), "the decoder never filled the missing fragment");
        assert!(frames > count, "a missing plain fragment takes a mixed part, took {frames}");
        assert_eq!(decoder.message().unwrap().unwrap(), message.as_slice());
    }

    #[test]
    fn frames_are_uppercase_qr_alphanumeric_text() {
        let fountain = UrFountain::new("crypto-psbt", &message(500), 50).unwrap();

        // QR alphanumeric mode holds 0-9, A-Z, space and $%*+-./:
        let alphanumeric = |byte: u8| {
            byte.is_ascii_digit() || byte.is_ascii_uppercase() || b" $%*+-./:".contains(&byte)
        };

        for index in 0..3 * fountain.sequence_count() {
            let frame = fountain.frame(index);
            assert!(frame.starts_with("UR:CRYPTO-PSBT/"), "{frame}");
            assert!(
                frame.bytes().all(alphanumeric),
                "{frame} has a byte outside QR alphanumeric mode"
            );
        }
    }

    #[test]
    fn a_message_in_one_fragment_shows_its_one_part_on_every_tick() {
        let fountain = UrFountain::new("bytes", &message(40), 200).unwrap();

        assert_eq!(fountain.sequence_count(), 1);
        assert!(fountain.frame(0).starts_with("UR:BYTES/1-1/"));
        assert_eq!(fountain.frame(0), fountain.frame(7));
    }

    #[test]
    fn the_sequence_number_wraps_back_to_the_first_fragment() {
        let fountain = UrFountain::new("bytes", &message(1_000), 60).unwrap();

        assert_eq!(fountain.frame(u32::MAX), fountain.frame(0));
        assert!(fountain.frame(u32::MAX - 1).starts_with(&format!("UR:BYTES/{}-17/", u32::MAX)));
    }

    #[test]
    fn an_empty_message_or_fragment_length_is_an_error() {
        assert!(UrFountain::new("bytes", &[], 60).is_err());
        assert!(UrFountain::new("bytes", &message(10), 0).is_err());
        assert!(UrFountain::new("", &message(10), 60).is_err());
    }
}

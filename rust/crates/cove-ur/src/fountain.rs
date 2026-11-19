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

/// How many plain fragments a display shows between mixed parts after its first pass
///
/// The plain fragments keep cycling so a receiver that starts late reads each of them within a
/// pass and a quarter, as cheap single-fragment parts. A receiver that keeps missing one still
/// gets a mixed part every few frames, and about one in five of those holds any given
/// fragment. Keystone charges a mixed part about 8 bytes of its 512 KiB scan budget per
/// fragment of the message, so a stream of nothing but mixed parts could exhaust it before a
/// late receiver had enough of them
const PLAIN_FRAGMENTS_PER_MIXED_PART: u64 = 4;

/// The frames of an animated UR, any of which can be computed from its index
///
/// Frames `0..sequence_count` carry the plain fragments in order. After that first pass, every
/// fifth frame is a fountain-mixed part and the rest go on cycling the plain fragments, so a
/// display shows frame `i` on its `i`th tick without end
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

/// A frame of the display, a plain fragment or a fountain-mixed part
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The plain fragment at a 0-based index
    Plain(u32),
    /// A mixed part, shown after the plain fragment at the 0-based index `after`
    Mixed { sequence: u32, after: u32 },
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

    /// What the display shows on its `index`th tick
    fn slot(&self, index: u32) -> Slot {
        let count = u64::from(self.sequence_count);
        let index = u64::from(index);

        // a message in one fragment shows its one part on every tick
        if index < count || count == 1 {
            return Slot::Plain((index % count) as u32);
        }

        let group_len = PLAIN_FRAGMENTS_PER_MIXED_PART + 1;
        let since_first_pass = index - count;
        let group = since_first_pass / group_len;
        let offset = since_first_pass % group_len;

        // plain fragments shown since the first pass, counting this frame if it is one
        let plain_shown = group * PLAIN_FRAGMENTS_PER_MIXED_PART + offset;
        if offset < PLAIN_FRAGMENTS_PER_MIXED_PART {
            return Slot::Plain((plain_shown % count) as u32);
        }

        // mixed parts take the sequence numbers after the plain fragments in order, and start
        // over at the first once they reach the largest a UR carries
        let mixed_sequences = u64::from(u32::MAX) - count;
        let sequence = count + 1 + group % mixed_sequences;

        Slot::Mixed { sequence: sequence as u32, after: ((plain_shown - 1) % count) as u32 }
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

    /// Which plain fragment a counter of one pass shows on a display's `index`th tick, from 0
    ///
    /// A plain fragment's frame shows its own index, and a mixed part holds the counter on the
    /// plain fragment shown before it, so the counter steps through `0..sequence_count` once
    /// per pass of the plain fragments
    #[must_use]
    pub fn pass_position(&self, index: u32) -> u32 {
        match self.slot(index) {
            Slot::Plain(fragment) | Slot::Mixed { after: fragment, .. } => fragment,
        }
    }

    /// The UR to show on a display's `index`th tick, counting from 0
    ///
    /// The frame is uppercase, as BCR-2020-005 recommends for QR transport, so a QR encoder can
    /// draw it in the denser alphanumeric mode instead of byte mode. A message that fits one
    /// fragment shows its one `1-1` part on every tick
    #[must_use]
    pub fn frame(&self, index: u32) -> String {
        let sequence = match self.slot(index) {
            Slot::Plain(fragment) => fragment + 1,
            Slot::Mixed { sequence, .. } => sequence,
        };

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
    use std::collections::BTreeMap;

    use foundation_ur::Decoder;

    use super::*;

    fn message(len: usize) -> Vec<u8> {
        (0..len).map(|byte| (byte * 31 % 251) as u8).collect()
    }

    /// The 1-based sequence number a frame's `seq-count` header names
    fn sequence(frame: &str) -> u32 {
        let header = frame.split('/').nth(1).unwrap();
        header.split('-').next().unwrap().parse().unwrap()
    }

    /// Feeds a receiver the frames from `start` on, skipping those whose sequence number
    /// `missed` names, and returns how many it read to complete, or `None` within `limit`
    fn frames_to_complete(
        fountain: &UrFountain,
        start: u32,
        limit: u32,
        missed: impl Fn(u32) -> bool,
    ) -> Option<u32> {
        let mut decoder = Decoder::default();
        let mut read = 0;

        for index in start..start + limit {
            let frame = fountain.frame(index);
            if missed(sequence(&frame)) {
                continue;
            }

            decoder.receive(UR::parse(&frame.to_ascii_lowercase()).unwrap()).unwrap();
            read += 1;

            if decoder.is_complete() {
                return Some(read);
            }
        }

        None
    }

    #[test]
    fn frames_match_the_foundation_encoder_part_with_their_sequence_number() {
        let message = message(1_000);
        let fountain = UrFountain::new("bytes", &message, 60).unwrap();
        assert_eq!(fountain.sequence_count(), 17);

        let frames: Vec<_> =
            (0..6 * fountain.sequence_count()).map(|i| fountain.frame(i)).collect();
        let last = frames.iter().map(|frame| sequence(frame)).max().unwrap();

        let mut encoder = foundation_ur::Encoder::new();
        encoder.start("bytes", &message, 60);

        let parts: BTreeMap<_, _> = (1..=last)
            .map(|sequence| (sequence, encoder.next_part().to_string().to_ascii_uppercase()))
            .collect();

        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame, &parts[&sequence(frame)], "frame {index}");
        }

        assert!(last > fountain.sequence_count(), "the frames go on to mixed parts");
    }

    #[test]
    fn the_first_pass_shows_the_plain_fragments_in_order() {
        let fountain = UrFountain::new("bytes", &message(1_000), 60).unwrap();
        let count = fountain.sequence_count();

        let sequences: Vec<_> = (0..count).map(|index| sequence(&fountain.frame(index))).collect();
        assert_eq!(sequences, (1..=count).collect::<Vec<_>>());
    }

    #[test]
    fn a_receiver_that_never_sees_one_plain_fragment_completes_from_mixed_parts() {
        let message = message(1_000);
        let fountain = UrFountain::new("bytes", &message, 60).unwrap();
        let count = fountain.sequence_count();

        let read = frames_to_complete(&fountain, 0, 3 * count, |sequence| sequence == 5);
        assert!(read.is_some(), "a mixed part fills the fragment the receiver never sees");
    }

    #[test]
    fn a_receiver_that_starts_after_the_first_pass_reads_every_plain_fragment_again() {
        let fountain = UrFountain::new("bytes", &message(1_000), 60).unwrap();
        let count = fountain.sequence_count();

        // within a pass and a quarter of any later tick, every plain fragment shows again
        for start in [count, count + 3, 5 * count + 2] {
            let window = start..start + count + count.div_ceil(4) + 1;
            let plain: BTreeSet<_> = window
                .map(|index| sequence(&fountain.frame(index)))
                .filter(|sequence| *sequence <= count)
                .collect();

            assert_eq!(plain, (1..=count).collect(), "from tick {start}");
        }

        let read = frames_to_complete(&fountain, 3 * count + 2, 2 * count, |_| false);
        assert!(read.is_some(), "a late receiver completes");
    }

    #[test]
    fn a_late_receiver_completes_within_keystones_scan_budget() {
        // Keystone takes up to 256 parts and charges each new part its fragment bytes, 128, and
        // 8 per fragment it may mix, every fragment for a mixed part, against 512 KiB a scan
        let fountain = UrFountain::new("bytes", &message(12_800), 50).unwrap();
        let count = fountain.sequence_count();
        assert_eq!(count, 256);

        let mut decoder = Decoder::default();
        let mut seen = BTreeSet::new();
        let mut work = 0;

        // the receiver points its camera at the screen after the first pass
        for index in count.. {
            let frame = fountain.frame(index);
            let sequence = sequence(&frame);

            if seen.insert(sequence) {
                let mixed = if sequence <= count { 1 } else { count as usize };
                work += 50 + mixed * 8 + 128;
                assert!(work <= 512 * 1024, "budget ran out at tick {index}");
            }

            decoder.receive(UR::parse(&frame.to_ascii_lowercase()).unwrap()).unwrap();
            if decoder.is_complete() {
                break;
            }
        }
    }

    #[test]
    fn the_counter_steps_through_one_pass_and_holds_on_mixed_parts() {
        let fountain = UrFountain::new("bytes", &message(1_000), 60).unwrap();
        let count = fountain.sequence_count();

        for index in 0..6 * count {
            let frame = fountain.frame(index);
            let position = fountain.pass_position(index);
            assert!(position < count, "tick {index} counts {position}");

            if sequence(&frame) <= count {
                assert_eq!(position + 1, sequence(&frame), "tick {index} counts its fragment");
            } else {
                assert_eq!(position, fountain.pass_position(index - 1), "tick {index} holds");
            }
        }
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
        assert_eq!(fountain.pass_position(7), 0);
    }

    #[test]
    fn the_last_tick_still_shows_a_valid_part() {
        let fountain = UrFountain::new("bytes", &message(1_000), 60).unwrap();

        for index in [u32::MAX - 5, u32::MAX - 1, u32::MAX] {
            let frame = fountain.frame(index).to_ascii_lowercase();
            assert!(UR::parse(&frame).is_ok(), "tick {index}: {frame}");
            assert!(fountain.pass_position(index) < fountain.sequence_count());
        }
    }

    #[test]
    fn an_empty_message_or_fragment_length_is_an_error() {
        assert!(UrFountain::new("bytes", &[], 60).is_err());
        assert!(UrFountain::new("bytes", &message(10), 0).is_err());
        assert!(UrFountain::new("", &message(10), 60).is_err());
    }
}

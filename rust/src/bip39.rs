use bip39::Language;
use num_bigint::BigUint;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error(
        "can only generate the last word, must pass in 11,14,17,20 or 23 words, passed in {0} words"
    )]
    InvalidNumberOfWords(usize),
}

pub fn generate_possible_final_words(phrase: &str) -> Result<Vec<String>, Error> {
    let (word_count, encoded_phrase) = split_and_encode_phrase(phrase);

    if ![11, 14, 17, 20, 23].contains(&word_count) {
        return Err(Error::InvalidNumberOfWords(word_count));
    }

    let checksum_width = (word_count + 1) / 3;
    let byte_width = (((word_count + 1) * 11) - checksum_width) / 8;
    let partial_result = encoded_phrase << (11 - checksum_width);

    let wordlist = Language::English.word_list();

    let final_words = (0..(1 << (11 - checksum_width)))
        .map(move |candidate| {
            let encoded = partial_result.clone() | BigUint::from(candidate);

            let encoded_bytes = encoded.to_bytes_be();
            let mut padding = vec![];

            // encoded_bytes less than byte width we are looking for so pad at the front
            let final_byte_array = if encoded_bytes.len() < byte_width {
                padding.resize(byte_width - encoded_bytes.len(), 0);

                padding.extend_from_slice(encoded_bytes.as_slice());
                padding.as_slice()
            } else {
                &encoded_bytes[..byte_width]
            };

            let mut hasher = Sha256::new();
            hasher.update(final_byte_array);

            let checksum = hasher.finalize()[0] >> (8 - checksum_width);
            let word_index = (candidate << checksum_width) + (checksum as u64);

            wordlist[word_index as usize].to_string()
        })
        .collect();

    Ok(final_words)
}

fn split_and_encode_phrase(phrase: &str) -> (usize, BigUint) {
    let words: Vec<&str> = if phrase.contains(' ') {
        phrase.split_whitespace().collect()
    } else {
        vec![phrase]
    };

    let word_count = words.len();
    let mut encoded_phrase = BigUint::from(0u32);

    for word in words {
        if let Some(word_index) = Language::English.find_word(word) {
            encoded_phrase = (encoded_phrase << 11) | BigUint::from(word_index as u64);
        }
    }

    (word_count, encoded_phrase)
}

#[cfg(test)]
mod test {
    use std::str::FromStr as _;

    use bip39::Mnemonic;
    use num_bigint::BigUint;
    use rand::Rng as _;

    use crate::bip39::split_and_encode_phrase;

    use super::generate_possible_final_words;

    #[test]
    fn test_encode_function_simple() {
        let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        let expected = vec![
            "art", "diesel", "false", "kite", "organ", "ready", "surface", "trouble",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>();

        for word in expected.clone() {
            let mut words: Vec<String> = words
                .split_ascii_whitespace()
                .map(ToString::to_string)
                .collect();

            words.push(word.to_string());

            let result = bip39::Mnemonic::parse_normalized(words.join(" ").as_str());
            assert!(result.is_ok());
        }

        assert_eq!(split_and_encode_phrase(words), (23, BigUint::from(0_u64)));
        assert_eq!(generate_possible_final_words(words).unwrap(), expected);
    }

    #[test]
    fn test_encode_function_simple_12() {
        let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        let expected = vec![
            "about", "actual", "age", "alpha", "angle", "argue", "artwork", "attract", "bachelor",
            "bean", "behind", "blind", "bomb", "brand", "broken", "burger", "cactus", "carbon",
            "cereal", "cheese", "city", "click", "coach", "cool", "coyote", "cricket", "cruise",
            "cute", "degree", "describe", "diesel", "disagree", "donor", "drama", "dune", "edit",
            "enemy", "energy", "escape", "exhaust", "express", "fashion", "field", "fiscal",
            "flavor", "food", "fringe", "furnace", "genius", "glue", "goddess", "grocery", "hand",
            "high", "holiday", "huge", "illness", "inform", "insect", "jacket", "kangaroo",
            "knock", "lamp", "lemon", "length", "lobster", "lyrics", "marble", "mass", "member",
            "metal", "moment", "mouse", "near", "noise", "obey", "offer", "once", "organ", "own",
            "parent", "phrase", "pill", "pole", "position", "process", "project", "question",
            "rail", "record", "remind", "render", "return", "ritual", "rubber", "sand", "scout",
            "sell", "share", "shoot", "simple", "slice", "soap", "solid", "speed", "square",
            "stereo", "street", "sugar", "surprise", "tank", "tent", "they", "toddler", "tongue",
            "trade", "truly", "turtle", "umbrella", "urge", "vast", "vendor", "void", "voyage",
            "wear", "wife", "world", "wrap",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>();

        assert_eq!(generate_possible_final_words(words).unwrap(), expected);
        assert_eq!(split_and_encode_phrase(words), (11, BigUint::from(0_u64)));
    }

    #[test]
    fn test_encode_function_real() {
        let words = vec![
            "wrap", "jar", "physical", "abuse", "minimum", "sand", "hair", "pet", "address",
            "alley", "fashion", "thank", "duck", "sound", "budget", "spell", "flush", "knock",
            "source", "novel", "mixed", "detect", "tackle",
        ]
        .join(" ");

        let expected = vec![
            "among", "depart", "estate", "join", "oppose", "penalty", "symbol", "wasp",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>();

        for word in expected.clone() {
            let mut words: Vec<String> = words
                .split_ascii_whitespace()
                .map(ToString::to_string)
                .collect();

            words.push(word.to_string());

            let result = bip39::Mnemonic::parse_normalized(words.join(" ").as_str());
            assert!(result.is_ok());
        }

        assert_eq!(
            split_and_encode_phrase(words.as_str()),
            (
                23,
                BigUint::from_str(
                    "14364227284564615196479332853034805609975917275364971745904421671938475106024"
                )
                .unwrap()
            )
        );
        assert_eq!(generate_possible_final_words(&words).unwrap(), expected);
    }

    #[test]
    fn check_final_words_are_accurate_12_words() {
        // 128 / 8  = 16
        let random_bytes = rand::rng().random::<[u8; 16]>();
        let words = Mnemonic::from_entropy(&random_bytes)
            .expect("failed to create mnemonic")
            .words()
            .collect::<Vec<&'static str>>();

        let first_11 = words[..11].join(" ");
        let last = words[11].to_string();

        let final_possible =
            generate_possible_final_words(&first_11).expect("correct number of words");

        assert!(final_possible.contains(&last))
    }

    #[test]
    fn check_final_words_are_accurate_24_words() {
        // 256 / 8  = 32
        let random_bytes = rand::rng().random::<[u8; 32]>();
        let words = Mnemonic::from_entropy(&random_bytes)
            .expect("failed to create mnemonic")
            .words()
            .collect::<Vec<&'static str>>();

        assert_eq!(words.len(), 24);

        let first_11 = words[..23].join(" ");
        let last = words.last().unwrap().to_string();

        let final_possible =
            generate_possible_final_words(&first_11).expect("correct number of words");

        assert!(final_possible.contains(&last))
    }
}

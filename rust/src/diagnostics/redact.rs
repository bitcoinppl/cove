use std::{cmp::Reverse, collections::BTreeMap, fmt, path::PathBuf, str::FromStr as _};

use bip39::{Language, Mnemonic};
use bitcoin::{Address, PrivateKey};
use rand::RngExt as _;
use sha2::{Digest as _, Sha256};

const BIP39_WORD_COUNTS: [usize; 5] = [24, 21, 18, 15, 12];
const TXID_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SecretKind {
    BitcoinAddress,
    ExtendedKey,
    Mnemonic,
    PrivateKey,
    TransactionId,
}

#[derive(Debug, Clone)]
struct PathPlaceholder {
    path: String,
    placeholder: &'static str,
}

#[derive(Clone)]
pub(crate) struct Redactor {
    paths: Vec<PathPlaceholder>,
    fingerprint_salt: [u8; 32],
    seen: BTreeMap<SecretFingerprint, String>,
    counts: BTreeMap<SecretKind, u32>,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Redactor")
            .field("paths_count", &self.paths.len())
            .field("seen_count", &self.seen.len())
            .field("counts", &self.counts)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SecretFingerprint {
    kind: SecretKind,
    digest: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
struct TokenSpan {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct MnemonicMatch {
    start: usize,
    end: usize,
    next_token_index: usize,
    normalized_phrase: String,
}

impl Redactor {
    pub(crate) fn with_default_paths() -> Self {
        Self::new(default_path_placeholders())
    }

    fn new(paths: Vec<PathPlaceholder>) -> Self {
        let mut paths = paths.into_iter().filter(|path| !path.path.is_empty()).collect::<Vec<_>>();
        paths.sort_by_key(|path| Reverse(path.path.len()));

        let mut fingerprint_salt = [0; 32];
        rand::rng().fill(&mut fingerprint_salt);

        Self { paths, fingerprint_salt, seen: BTreeMap::new(), counts: BTreeMap::new() }
    }

    pub(crate) fn redact(&mut self, text: &str) -> String {
        let path_redacted = self.redact_paths(text);
        self.redact_secrets(&path_redacted)
    }

    fn redact_paths(&self, text: &str) -> String {
        self.paths
            .iter()
            .fold(text.to_string(), |redacted, path| redacted.replace(&path.path, path.placeholder))
    }

    fn redact_secrets(&mut self, text: &str) -> String {
        let tokens = token_spans(text);
        if tokens.is_empty() {
            return text.to_string();
        }

        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        let mut token_index = 0;

        while token_index < tokens.len() {
            if let Some(match_) = mnemonic_match(text, &tokens, token_index) {
                output.push_str(&text[cursor..match_.start]);
                output.push_str(
                    &self.placeholder_for(SecretKind::Mnemonic, &match_.normalized_phrase),
                );
                cursor = match_.end;
                token_index = match_.next_token_index;
                continue;
            }

            let token = tokens[token_index];
            output.push_str(&text[cursor..token.start]);
            output.push_str(&self.redact_token(token.text(text)));
            cursor = token.end;
            token_index += 1;
        }

        output.push_str(&text[cursor..]);

        output
    }

    fn redact_token(&mut self, token: &str) -> String {
        let Some(kind) = classify_secret(token) else {
            return self.redact_embedded_txids(token);
        };

        self.placeholder_for(kind, token)
    }

    fn redact_embedded_txids(&mut self, token: &str) -> String {
        let mut output = String::with_capacity(token.len());
        let mut cursor = 0;

        while cursor < token.len() {
            let Some(run_start_offset) =
                token[cursor..].bytes().position(|byte| byte.is_ascii_hexdigit())
            else {
                output.push_str(&token[cursor..]);
                break;
            };
            let run_start = cursor + run_start_offset;
            output.push_str(&token[cursor..run_start]);

            let run_len =
                token[run_start..].bytes().take_while(|byte| byte.is_ascii_hexdigit()).count();
            let run_end = run_start + run_len;
            let mut run_cursor = run_start;

            while run_cursor + TXID_HEX_LEN <= run_end {
                let txid = &token[run_cursor..run_cursor + TXID_HEX_LEN];
                output.push_str(&self.placeholder_for(SecretKind::TransactionId, txid));
                run_cursor += TXID_HEX_LEN;
            }

            output.push_str(&token[run_cursor..run_end]);
            cursor = run_end;
        }

        output
    }

    fn placeholder_for(&mut self, kind: SecretKind, secret: &str) -> String {
        let fingerprint = SecretFingerprint::new(kind, secret, &self.fingerprint_salt);
        if let Some(placeholder) = self.seen.get(&fingerprint) {
            return placeholder.clone();
        }

        let next = self.counts.entry(kind).and_modify(|count| *count += 1).or_insert(1);
        let placeholder = format!("<redacted-{}-{next}>", kind.placeholder_name());
        self.seen.insert(fingerprint, placeholder.clone());

        placeholder
    }
}

impl SecretFingerprint {
    fn new(kind: SecretKind, secret: &str, salt: &[u8; 32]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update([0]);
        hasher.update(kind.placeholder_name().as_bytes());
        hasher.update([0]);
        hasher.update(secret.as_bytes());
        let digest = hasher.finalize().into();

        Self { kind, digest }
    }
}

impl SecretKind {
    fn placeholder_name(self) -> &'static str {
        match self {
            Self::BitcoinAddress => "bitcoin-address",
            Self::ExtendedKey => "extended-key",
            Self::Mnemonic => "seed-phrase",
            Self::PrivateKey => "wif-private-key",
            Self::TransactionId => "transaction-id",
        }
    }
}

fn default_path_placeholders() -> Vec<PathPlaceholder> {
    let root = cove_common::consts::ROOT_DATA_DIR.clone();
    let wallet = cove_common::consts::WALLET_DATA_DIR.clone();
    let mut placeholders = vec![
        path_placeholder(wallet, "<COVE_WALLET_DATA_DIR>"),
        path_placeholder(root, "<COVE_ROOT_DATA_DIR>"),
    ];

    if let Some(home) = dirs::home_dir() {
        placeholders.push(path_placeholder(home, "<HOME_DIR>"));
    }

    placeholders
}

fn path_placeholder(path: PathBuf, placeholder: &'static str) -> PathPlaceholder {
    PathPlaceholder { path: path.to_string_lossy().to_string(), placeholder }
}

fn classify_secret(token: &str) -> Option<SecretKind> {
    if is_txid(token) {
        return Some(SecretKind::TransactionId);
    }

    if is_extended_key(token) {
        return Some(SecretKind::ExtendedKey);
    }

    if is_wif_private_key(token) {
        return Some(SecretKind::PrivateKey);
    }

    if is_bitcoin_address(token) {
        return Some(SecretKind::BitcoinAddress);
    }

    None
}

fn is_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
}

fn token_spans(text: &str) -> Vec<TokenSpan> {
    let mut tokens = Vec::new();
    let mut token_start = None;
    let mut characters = text.char_indices().peekable();

    while let Some((index, character)) = characters.next() {
        if let Some(escape_len) = escaped_whitespace_len(&text[index..]) {
            if let Some(start) = token_start.take() {
                tokens.push(TokenSpan { start, end: index });
            }

            let escape_end = index + escape_len;
            while characters.peek().is_some_and(|(index, _)| *index < escape_end) {
                characters.next();
            }

            continue;
        }

        if is_token_character(character) {
            token_start.get_or_insert(index);
            continue;
        }

        if let Some(start) = token_start.take() {
            tokens.push(TokenSpan { start, end: index });
        }
    }

    if let Some(start) = token_start {
        tokens.push(TokenSpan { start, end: text.len() });
    }

    tokens
}

impl TokenSpan {
    fn text(self, text: &str) -> &str {
        &text[self.start..self.end]
    }
}

fn mnemonic_match(text: &str, tokens: &[TokenSpan], token_index: usize) -> Option<MnemonicMatch> {
    let candidate = mnemonic_candidate(text, tokens, token_index)?;

    for word_count in BIP39_WORD_COUNTS {
        if word_count > candidate.word_token_indexes.len() {
            continue;
        }

        let word_token_indexes = &candidate.word_token_indexes[..word_count];
        let normalized_phrase = normalized_mnemonic_phrase(text, tokens, word_token_indexes);

        if Mnemonic::parse_in_normalized(Language::English, &normalized_phrase).is_err() {
            continue;
        }

        let last_word_index = word_token_indexes[word_count - 1];

        return Some(MnemonicMatch {
            start: candidate.start,
            end: tokens[last_word_index].end,
            next_token_index: last_word_index + 1,
            normalized_phrase,
        });
    }

    None
}

struct MnemonicCandidate {
    start: usize,
    word_token_indexes: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MnemonicNumbering {
    ZeroBased,
    OneBased,
}

fn mnemonic_candidate(
    text: &str,
    tokens: &[TokenSpan],
    token_index: usize,
) -> Option<MnemonicCandidate> {
    let token = tokens[token_index];
    let (start, mut word_token_index, mut numbering) = if is_mnemonic_word(token.text(text)) {
        (token.start, token_index, mnemonic_numbering_before_word(text, tokens, token_index))
    } else {
        let (word_token_index, numbering) =
            numbered_mnemonic_word_index(text, tokens, token_index, 0, None)?;

        (tokens[word_token_index].start, word_token_index, Some(numbering))
    };
    let mut word_token_indexes = Vec::with_capacity(BIP39_WORD_COUNTS[0]);

    loop {
        let word = tokens[word_token_index].text(text);
        if !is_mnemonic_word(word) {
            break;
        }

        word_token_indexes.push(word_token_index);
        if word_token_indexes.len() == BIP39_WORD_COUNTS[0] {
            break;
        }

        let Some((next_word_token_index, next_numbering)) = next_mnemonic_word_index(
            text,
            tokens,
            word_token_index,
            word_token_indexes.len(),
            numbering,
        ) else {
            break;
        };

        word_token_index = next_word_token_index;
        numbering = next_numbering;
    }

    Some(MnemonicCandidate { start, word_token_indexes })
}

fn next_mnemonic_word_index(
    text: &str,
    tokens: &[TokenSpan],
    word_token_index: usize,
    next_word_index: usize,
    numbering: Option<MnemonicNumbering>,
) -> Option<(usize, Option<MnemonicNumbering>)> {
    let next_token_index = word_token_index + 1;
    let next_token = *tokens.get(next_token_index)?;
    let separator = &text[tokens[word_token_index].end..next_token.start];
    if !is_mnemonic_separator(separator) {
        return None;
    }

    if is_mnemonic_word(next_token.text(text)) {
        return Some((next_token_index, numbering));
    }

    let (word_token_index, numbering) =
        numbered_mnemonic_word_index(text, tokens, next_token_index, next_word_index, numbering)?;

    Some((word_token_index, Some(numbering)))
}

fn numbered_mnemonic_word_index(
    text: &str,
    tokens: &[TokenSpan],
    number_token_index: usize,
    word_index: usize,
    expected_numbering: Option<MnemonicNumbering>,
) -> Option<(usize, MnemonicNumbering)> {
    let number_token = tokens.get(number_token_index)?.text(text);
    let numbering = MnemonicNumbering::from_marker(number_token.parse().ok()?, word_index)?;
    if expected_numbering.is_some_and(|expected| expected != numbering) {
        return None;
    }

    let word_token_index = number_token_index + 1;
    let word_token = *tokens.get(word_token_index)?;
    let separator = &text[tokens[number_token_index].end..word_token.start];
    if !is_mnemonic_separator(separator) || !is_mnemonic_word(word_token.text(text)) {
        return None;
    }

    Some((word_token_index, numbering))
}

fn mnemonic_numbering_before_word(
    text: &str,
    tokens: &[TokenSpan],
    word_token_index: usize,
) -> Option<MnemonicNumbering> {
    let number_token_index = word_token_index.checked_sub(1)?;
    let (numbered_word_token_index, numbering) =
        numbered_mnemonic_word_index(text, tokens, number_token_index, 0, None)?;

    (numbered_word_token_index == word_token_index).then_some(numbering)
}

impl MnemonicNumbering {
    fn from_marker(marker: usize, word_index: usize) -> Option<Self> {
        match marker.checked_sub(word_index)? {
            0 => Some(Self::ZeroBased),
            1 => Some(Self::OneBased),
            _ => None,
        }
    }
}

fn is_mnemonic_word(word: &str) -> bool {
    word.chars().all(|character| character.is_ascii_alphabetic())
}

fn is_mnemonic_separator(separator: &str) -> bool {
    !separator.is_empty()
        && mnemonic_separator_characters_are_valid(separator, is_mnemonic_format_character)
}

fn mnemonic_separator_characters_are_valid(
    separator: &str,
    mut is_format_character: impl FnMut(char) -> bool,
) -> bool {
    let mut cursor = 0;

    while cursor < separator.len() {
        if let Some(escape_len) = escaped_whitespace_len(&separator[cursor..]) {
            cursor += escape_len;
            continue;
        }

        let character = separator[cursor..].chars().next().expect("cursor is in bounds");
        if !character.is_whitespace() && !is_format_character(character) {
            return false;
        }

        cursor += character.len_utf8();
    }

    true
}

fn escaped_whitespace_len(text: &str) -> Option<usize> {
    if text.starts_with("\\n") || text.starts_with("\\r") || text.starts_with("\\t") {
        return Some(2);
    }

    let unicode_escape = text.strip_prefix("\\u{")?;
    let closing_brace = unicode_escape.find('}')?;
    let hexadecimal = &unicode_escape[..closing_brace];
    if hexadecimal.is_empty()
        || hexadecimal.len() > 6
        || !hexadecimal.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }

    let value = u32::from_str_radix(hexadecimal, 16).ok()?;
    let character = char::from_u32(value)?;
    character.is_whitespace().then_some(closing_brace + 4)
}

fn is_mnemonic_format_character(character: char) -> bool {
    matches!(
        character,
        ',' | ';'
            | '.'
            | ':'
            | '\''
            | '"'
            | '`'
            | '\\'
            | '['
            | ']'
            | '('
            | ')'
            | '-'
            | '–'
            | '—'
            | '*'
            | '+'
            | '|'
            | '/'
            | '•'
            | '◦'
            | '‘'
            | '’'
            | '“'
            | '”'
    )
}

fn normalized_mnemonic_phrase(
    text: &str,
    tokens: &[TokenSpan],
    word_token_indexes: &[usize],
) -> String {
    let mut words = Vec::with_capacity(word_token_indexes.len());

    for token_index in word_token_indexes {
        words.push(tokens[*token_index].text(text).to_ascii_lowercase());
    }

    words.join(" ")
}

fn is_txid(token: &str) -> bool {
    token.len() == TXID_HEX_LEN && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_extended_key(token: &str) -> bool {
    const PREFIXES: [&str; 12] = [
        "xpub", "ypub", "zpub", "tpub", "upub", "vpub", "xprv", "yprv", "zprv", "tprv", "uprv",
        "vprv",
    ];

    token.len() >= 50
        && PREFIXES.iter().any(|prefix| token.starts_with(prefix))
        && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_wif_private_key(token: &str) -> bool {
    let has_known_prefix = token.starts_with('5')
        || token.starts_with('K')
        || token.starts_with('L')
        || token.starts_with('9')
        || token.starts_with('c');

    has_known_prefix && PrivateKey::from_wif(token).is_ok()
}

fn is_bitcoin_address(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    let has_known_prefix = lower.starts_with("bc1")
        || lower.starts_with("tb1")
        || lower.starts_with("bcrt1")
        || token.starts_with('1')
        || token.starts_with('3')
        || token.starts_with('m')
        || token.starts_with('n')
        || token.starts_with('2');

    has_known_prefix && Address::from_str(token).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MNEMONIC_WORDS: [&str; 12] = [
        "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
        "abandon", "abandon", "abandon", "about",
    ];

    fn redactor_without_paths() -> Redactor {
        Redactor::new(vec![])
    }

    #[test]
    fn redacts_bitcoin_addresses_across_networks_and_scripts() {
        let mut redactor = redactor_without_paths();
        let input = concat!(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 ",
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0 ",
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7 ",
            "bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5 ",
            "1BoatSLRHtKNngkdXEeobR76b53LETtpyT"
        );

        let output = redactor.redact(input);

        assert_eq!(output.matches("<redacted-bitcoin-address-").count(), 5);
        assert!(!output.contains("1BoatSLRHtKNngkdXEeobR76b53LETtpyT"));
    }

    #[test]
    fn redacts_extended_keys_and_txids_with_stable_placeholders() {
        let mut redactor = redactor_without_paths();
        let xpub = format!("xpub{}", "A".repeat(106));
        let txid = "4d3c2b1a".repeat(8);
        let input = format!("{xpub} {txid} {xpub} {txid}");

        let output = redactor.redact(&input);

        assert_eq!(output.matches("<redacted-extended-key-1>").count(), 2);
        assert_eq!(output.matches("<redacted-transaction-id-1>").count(), 2);
        assert!(!output.contains(&xpub));
        assert!(!output.contains(&txid));
    }

    #[test]
    fn redacts_txids_embedded_inside_larger_tokens() {
        let mut redactor = redactor_without_paths();
        let txid = "4d3c2b1a".repeat(8);
        let input = format!("payment{txid}suffix again {txid}");

        let output = redactor.redact(&input);

        assert_eq!(output.matches("<redacted-transaction-id-1>").count(), 2);
        assert!(output.contains("payment<redacted-transaction-id-1>suffix"));
        assert!(!output.contains(&txid));
    }

    #[test]
    fn debug_output_does_not_include_seen_plaintext_secrets() {
        let mut redactor = redactor_without_paths();
        let address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        let output = redactor.redact(&format!("{address} {mnemonic}"));
        let debug = format!("{redactor:?}");

        assert!(output.contains("<redacted-bitcoin-address-1>"));
        assert!(output.contains("<redacted-seed-phrase-1>"));
        assert!(!debug.contains(address));
        assert!(!debug.contains(mnemonic));
        assert!(debug.contains("seen_count"));
    }

    #[test]
    fn redacts_bip39_seed_phrases_with_stable_placeholders() {
        let mut redactor = redactor_without_paths();
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let input = format!("seed: {mnemonic}\nagain: {mnemonic}");

        let output = redactor.redact(&input);

        assert_eq!(output.matches("<redacted-seed-phrase-1>").count(), 2);
        assert!(!output.contains(mnemonic));
    }

    #[test]
    fn redacts_bip39_seed_phrases_across_newlines() {
        let mut redactor = redactor_without_paths();
        let mnemonic = "abandon abandon abandon abandon abandon abandon\nabandon abandon abandon abandon abandon about";

        let output = redactor.redact(mnemonic);

        assert_eq!(output, "<redacted-seed-phrase-1>");
    }

    #[test]
    fn redacts_formatted_bip39_seed_phrases() {
        let mut redactor = redactor_without_paths();
        let words = MNEMONIC_WORDS;
        let comma_separated = words.join(",");
        let quoted = format!("{words:?}");
        let numbered = words
            .iter()
            .enumerate()
            .map(|(index, word)| format!("{}. {word}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let bulleted = words.iter().map(|word| format!("- {word}")).collect::<Vec<_>>().join("\n");

        for formatted in [comma_separated, quoted, numbered, bulleted] {
            let output = redactor.redact(&formatted);

            assert!(output.contains("<redacted-seed-phrase-1>"), "output: {output}");
            assert!(!output.contains("abandon"), "output: {output}");
            assert!(!output.contains("about"), "output: {output}");
        }
    }

    #[test]
    fn redacts_zero_and_one_based_numbered_seed_phrases() {
        let mut redactor = redactor_without_paths();

        for numbering_base in [0, 1] {
            let input = MNEMONIC_WORDS
                .iter()
                .enumerate()
                .map(|(index, word)| format!("{}. {word}", index + numbering_base))
                .collect::<Vec<_>>()
                .join("\n");

            let output = redactor.redact(&input);

            assert_eq!(output, format!("{numbering_base}. <redacted-seed-phrase-1>"));
        }
    }

    #[test]
    fn redacts_whitespace_only_numbered_seed_phrases() {
        let mut redactor = redactor_without_paths();

        for (numbering_base, column_separator) in [(1, " "), (0, "\t")] {
            let input = MNEMONIC_WORDS
                .iter()
                .enumerate()
                .map(|(index, word)| format!("{}{column_separator}{word}", index + numbering_base))
                .collect::<Vec<_>>()
                .join("\n");

            let output = redactor.redact(&input);

            assert_eq!(
                output,
                format!("{numbering_base}{column_separator}<redacted-seed-phrase-1>")
            );
        }
    }

    #[test]
    fn numbered_seed_phrases_require_consistent_sequential_markers() {
        let mut redactor = redactor_without_paths();
        let input = MNEMONIC_WORDS
            .iter()
            .enumerate()
            .map(|(index, word)| {
                let number = if index == 6 { index + 2 } else { index + 1 };
                format!("{number} {word}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let output = redactor.redact(&input);

        assert_eq!(output, input);
    }

    #[test]
    fn redacts_debug_formatted_multiline_seed_phrases() {
        let mut redactor = redactor_without_paths();
        let mnemonic =
            format!("{}\n{}", MNEMONIC_WORDS[..6].join(" "), MNEMONIC_WORDS[6..].join(" "));
        let input = format!("{mnemonic:?}");

        let output = redactor.redact(&input);

        assert!(input.contains("\\n"));
        assert_eq!(output, "\"<redacted-seed-phrase-1>\"");
    }

    #[test]
    fn redacts_debug_formatted_unicode_whitespace_seed_phrases() {
        let mut redactor = redactor_without_paths();
        let mnemonic = MNEMONIC_WORDS.join("\u{b}");
        let input = format!("{mnemonic:?}");

        let output = redactor.redact(&input);

        assert!(input.contains("\\u{b}"));
        assert_eq!(output, "\"<redacted-seed-phrase-1>\"");

        let non_whitespace_escape = MNEMONIC_WORDS.join("\\u{61}");
        assert_eq!(redactor.redact(&non_whitespace_escape), non_whitespace_escape);
    }

    #[test]
    fn redacts_slash_separated_seed_phrases() {
        let mut redactor = redactor_without_paths();
        let input = MNEMONIC_WORDS.join("/");

        let output = redactor.redact(&input);

        assert_eq!(output, "<redacted-seed-phrase-1>");
    }

    #[test]
    fn redacts_period_separated_seed_phrases() {
        let mut redactor = redactor_without_paths();
        let input = MNEMONIC_WORDS.join(". ");

        let output = redactor.redact(&input);

        assert_eq!(output, "<redacted-seed-phrase-1>");
    }

    #[test]
    fn numbered_quoted_seed_phrase_boundaries_remain_balanced() {
        let mut redactor = redactor_without_paths();
        let input = MNEMONIC_WORDS
            .iter()
            .enumerate()
            .map(|(index, word)| format!("{}. \"{word}\"", index + 1))
            .collect::<Vec<_>>()
            .join("\n");

        let output = redactor.redact(&input);

        assert_eq!(output, "1. \"<redacted-seed-phrase-1>\"");
    }

    #[test]
    fn bracketed_numbered_seed_phrase_boundaries_preserve_the_first_label() {
        let mut redactor = redactor_without_paths();
        let compact = MNEMONIC_WORDS
            .iter()
            .enumerate()
            .map(|(index, word)| format!("[{}] {word}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let spaced = MNEMONIC_WORDS
            .iter()
            .enumerate()
            .map(|(index, word)| format!("[ {} ] {word}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");

        let compact_output = redactor.redact(&compact);
        let spaced_output = redactor.redact(&spaced);

        assert_eq!(compact_output, "[1] <redacted-seed-phrase-1>");
        assert_eq!(spaced_output, "[ 1 ] <redacted-seed-phrase-1>");
    }

    #[test]
    fn formatted_words_still_require_a_valid_bip39_checksum() {
        let mut redactor = redactor_without_paths();
        let invalid_words = ["abandon"; 12];
        let input = format!("{invalid_words:?}");

        let output = redactor.redact(&input);

        assert_eq!(output, input);
    }

    #[test]
    fn redacts_wif_private_keys() {
        let mut redactor = redactor_without_paths();
        let mainnet_wif = "5JYkZjmN7PVMjJUfJWfRFwtuXTGB439XV6faajeHPAM9Z2PT2R3";
        let testnet_wif = "cVt4o7BGAig1UXywgGSmARhxMdzP5qvQsxKkSsc1XEkw3tDTQFpy";
        let input = format!("{mainnet_wif} {testnet_wif} {mainnet_wif}");

        let output = redactor.redact(&input);

        assert_eq!(output.matches("<redacted-wif-private-key-1>").count(), 2);
        assert_eq!(output.matches("<redacted-wif-private-key-2>").count(), 1);
        assert!(!output.contains(mainnet_wif));
        assert!(!output.contains(testnet_wif));
    }

    #[test]
    fn does_not_redact_amounts_or_near_misses() {
        let mut redactor = redactor_without_paths();
        let output = redactor.redact("amount=12345 sats fee=1.23 BTC abc123 not_a_txid");

        assert!(output.contains("12345 sats"));
        assert!(output.contains("1.23 BTC"));
        assert!(output.contains("abc123"));
    }

    #[test]
    fn redacts_known_paths_before_tokens() {
        let path = PathPlaceholder {
            path: "/var/mobile/Containers/Data/Application/abc/.data/wallets".to_string(),
            placeholder: "<COVE_WALLET_DATA_DIR>",
        };
        let mut redactor = Redactor::new(vec![path]);
        let output = redactor
            .redact("Wallet dir: /var/mobile/Containers/Data/Application/abc/.data/wallets");

        assert_eq!(output, "Wallet dir: <COVE_WALLET_DATA_DIR>");
    }
}

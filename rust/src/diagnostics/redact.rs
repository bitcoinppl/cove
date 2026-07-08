use std::{cmp::Reverse, collections::BTreeMap, path::PathBuf, str::FromStr as _};

use bip39::{Language, Mnemonic};
use bitcoin::{Address, PrivateKey};

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

#[derive(Debug, Clone)]
pub(crate) struct Redactor {
    paths: Vec<PathPlaceholder>,
    seen: BTreeMap<String, String>,
    counts: BTreeMap<SecretKind, u32>,
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

        Self { paths, seen: BTreeMap::new(), counts: BTreeMap::new() }
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
        if let Some(placeholder) = self.seen.get(secret) {
            return placeholder.clone();
        }

        let next = self.counts.entry(kind).and_modify(|count| *count += 1).or_insert(1);
        let placeholder = format!("<redacted-{}-{next}>", kind.placeholder_name());
        self.seen.insert(secret.to_string(), placeholder.clone());

        placeholder
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

    for (index, character) in text.char_indices() {
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
    for word_count in BIP39_WORD_COUNTS {
        let end_index = token_index + word_count;
        if end_index > tokens.len() {
            continue;
        }

        let candidate_tokens = &tokens[token_index..end_index];
        if !is_whitespace_separated(text, candidate_tokens) {
            continue;
        }

        let Some(normalized_phrase) = normalized_mnemonic_phrase(text, candidate_tokens) else {
            continue;
        };

        if Mnemonic::parse_in_normalized(Language::English, &normalized_phrase).is_err() {
            continue;
        }

        return Some(MnemonicMatch {
            start: candidate_tokens[0].start,
            end: candidate_tokens[word_count - 1].end,
            next_token_index: end_index,
            normalized_phrase,
        });
    }

    None
}

fn is_whitespace_separated(text: &str, tokens: &[TokenSpan]) -> bool {
    tokens
        .windows(2)
        .all(|window| text[window[0].end..window[1].start].chars().all(char::is_whitespace))
}

fn normalized_mnemonic_phrase(text: &str, tokens: &[TokenSpan]) -> Option<String> {
    let mut words = Vec::with_capacity(tokens.len());

    for token in tokens {
        let word = token.text(text);
        if !word.chars().all(|character| character.is_ascii_alphabetic()) {
            return None;
        }

        words.push(word.to_ascii_lowercase());
    }

    Some(words.join(" "))
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

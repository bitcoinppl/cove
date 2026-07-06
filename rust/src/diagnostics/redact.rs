use std::{cmp::Reverse, collections::BTreeMap, path::PathBuf, str::FromStr as _};

use bitcoin::Address;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SecretKind {
    BitcoinAddress,
    ExtendedKey,
    TransactionId,
}

#[derive(Debug, Clone)]
struct PathPlaceholder {
    path: String,
    placeholder: &'static str,
}

#[derive(Debug)]
pub(crate) struct Redactor {
    paths: Vec<PathPlaceholder>,
    seen: BTreeMap<String, String>,
    counts: BTreeMap<SecretKind, u32>,
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
        self.redact_tokens(&path_redacted)
    }

    fn redact_paths(&self, text: &str) -> String {
        self.paths
            .iter()
            .fold(text.to_string(), |redacted, path| redacted.replace(&path.path, path.placeholder))
    }

    fn redact_tokens(&mut self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut token_start = None;

        for (index, character) in text.char_indices() {
            if is_token_character(character) {
                token_start.get_or_insert(index);
                continue;
            }

            if let Some(start) = token_start.take() {
                output.push_str(&self.redact_token(&text[start..index]));
            }
            output.push(character);
        }

        if let Some(start) = token_start {
            output.push_str(&self.redact_token(&text[start..]));
        }

        output
    }

    fn redact_token(&mut self, token: &str) -> String {
        let Some(kind) = classify_secret(token) else {
            return token.to_string();
        };

        if let Some(placeholder) = self.seen.get(token) {
            return placeholder.clone();
        }

        let next = self.counts.entry(kind).and_modify(|count| *count += 1).or_insert(1);
        let placeholder = format!("<redacted-{}-{next}>", kind.placeholder_name());
        self.seen.insert(token.to_string(), placeholder.clone());

        placeholder
    }
}

impl SecretKind {
    fn placeholder_name(self) -> &'static str {
        match self {
            Self::BitcoinAddress => "bitcoin-address",
            Self::ExtendedKey => "extended-key",
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

    if is_bitcoin_address(token) {
        return Some(SecretKind::BitcoinAddress);
    }

    None
}

fn is_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
}

fn is_txid(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
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

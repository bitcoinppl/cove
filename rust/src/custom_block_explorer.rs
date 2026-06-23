use std::fmt;

use cove_types::Network;
use url::Url;

const PLACEHOLDER: &str = "{txid}";
const PLACEHOLDER_MARKER_SEED: &str = "coveplaceholdertxid";
const SUPPORTED_PLACEHOLDERS: [&str; 3] = ["{0}", "{txid}", "{tx_id}"];

pub const PREVIEW_TXID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BlockExplorerOption {
    MempoolSpace,
    MempoolGuide,
    BullBitcoin,
    Blockstream,
    Custom,
}

#[uniffi::export]
impl BlockExplorerOption {
    pub fn display_name(&self) -> String {
        self.as_display_name().to_string()
    }
}

impl BlockExplorerOption {
    const PRESETS: [Self; 5] = [
        Self::MempoolSpace,
        Self::MempoolGuide,
        Self::BullBitcoin,
        Self::Blockstream,
        Self::Custom,
    ];

    pub(crate) const fn all() -> [Self; 5] {
        Self::PRESETS
    }

    pub(crate) const fn base_url(&self) -> Option<&'static str> {
        match self {
            Self::MempoolSpace | Self::Custom => None,
            Self::MempoolGuide => Some("https://mempool.guide/"),
            Self::BullBitcoin => Some("https://mempool.bullbitcoin.com/"),
            Self::Blockstream => Some("https://blockstream.info/"),
        }
    }

    pub(crate) fn matching_stored_template(stored_template: Option<&str>) -> Self {
        let Some(stored_template) = stored_template else {
            return Self::MempoolSpace;
        };

        let Ok(template) = CustomBlockExplorerTemplate::parse_stored(stored_template) else {
            return Self::MempoolSpace;
        };

        Self::all()
            .into_iter()
            .find(|option| option.matches_template(&template))
            .unwrap_or(Self::Custom)
    }

    fn as_display_name(&self) -> &'static str {
        match self {
            Self::MempoolSpace => "Default (mempool.space)",
            Self::MempoolGuide => "mempool.guide",
            Self::BullBitcoin => "mempool.bullbitcoin.com",
            Self::Blockstream => "blockstream.info",
            Self::Custom => "Custom",
        }
    }

    fn matches_template(&self, template: &CustomBlockExplorerTemplate) -> bool {
        let preset_template = match self {
            Self::MempoolSpace => Some(CustomBlockExplorerTemplate::default_for(Network::Bitcoin)),
            Self::Custom => None,
            _ => self.base_url().and_then(|base_url| {
                CustomBlockExplorerTemplate::parse(Network::Bitcoin, base_url).ok()
            }),
        };

        preset_template.is_some_and(|preset_template| preset_template.as_str() == template.as_str())
    }
}

#[uniffi::export]
pub fn all_block_explorer_options() -> Vec<BlockExplorerOption> {
    BlockExplorerOption::all().to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustomBlockExplorerError {
    #[error("Block explorer URL cannot be empty")]
    Empty,

    #[error("Block explorer URL must use http or https")]
    InvalidScheme,

    #[error("Block explorer URL must include a host")]
    MissingHost,

    #[error("Block explorer URL cannot include a fragment")]
    Fragment,

    #[error("Unsupported block explorer template placeholder")]
    UnsupportedPlaceholder,

    #[error("Invalid block explorer URL")]
    InvalidUrl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomBlockExplorerTemplate(String);

impl CustomBlockExplorerTemplate {
    pub fn parse(network: Network, input: &str) -> Result<Self, CustomBlockExplorerError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(CustomBlockExplorerError::Empty);
        }

        validate_placeholders(trimmed)?;

        if contains_supported_placeholder(trimmed) {
            return Self::parse_template(trimmed);
        }

        Self::parse_base_url(network, trimmed)
    }

    pub fn default_for(network: Network) -> Self {
        let template = match network {
            Network::Bitcoin => "https://mempool.space/tx/{txid}",
            Network::Testnet => "https://mempool.space/testnet/tx/{txid}",
            Network::Testnet4 => "https://mempool.space/testnet4/tx/{txid}",
            Network::Signet => "https://mutinynet.com/tx/{txid}",
        };

        Self(template.to_string())
    }

    pub fn parse_stored(input: &str) -> Result<Self, CustomBlockExplorerError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(CustomBlockExplorerError::Empty);
        }

        validate_placeholders(trimmed)?;

        if !contains_supported_placeholder(trimmed) {
            return Err(CustomBlockExplorerError::UnsupportedPlaceholder);
        }

        Self::parse_template(trimmed)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn render(&self, txid: impl fmt::Display) -> String {
        let txid = txid.to_string();
        SUPPORTED_PLACEHOLDERS
            .iter()
            .fold(self.0.clone(), |rendered, placeholder| rendered.replace(placeholder, &txid))
    }

    fn parse_template(input: &str) -> Result<Self, CustomBlockExplorerError> {
        let placeholder_marker = placeholder_marker_absent_from(input);
        let probe = replace_supported_placeholders(input, &placeholder_marker);
        let url = parse_http_url_with_optional_scheme(&probe)?;
        if url.fragment().is_some() {
            return Err(CustomBlockExplorerError::Fragment);
        }

        validate_placeholder_location(&url, &placeholder_marker)?;

        let canonical = url.to_string().replace(&placeholder_marker, PLACEHOLDER);
        Ok(Self(canonical))
    }

    fn parse_base_url(network: Network, input: &str) -> Result<Self, CustomBlockExplorerError> {
        let mut url = parse_http_url_with_optional_scheme(input)?;
        if url.fragment().is_some() {
            return Err(CustomBlockExplorerError::Fragment);
        }

        let placeholder_marker = placeholder_marker_absent_from(url.as_str());
        let path = canonical_base_path(&url, network, &placeholder_marker);
        url.set_path(&path);

        let canonical = url.to_string().replace(&placeholder_marker, PLACEHOLDER);
        Ok(Self(canonical))
    }
}

pub fn effective_transaction_url(
    network: Network,
    stored_template: Option<&str>,
    txid: impl fmt::Display,
) -> String {
    if let Some(template) = stored_template
        .and_then(|stored_template| CustomBlockExplorerTemplate::parse_stored(stored_template).ok())
    {
        return template.render(txid);
    }

    CustomBlockExplorerTemplate::default_for(network).render(txid)
}

fn parse_http_url(input: &str) -> Result<Url, CustomBlockExplorerError> {
    let url = Url::parse(input).map_err(|_| CustomBlockExplorerError::InvalidUrl)?;

    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(CustomBlockExplorerError::InvalidScheme),
    }

    if url.host_str().is_none() {
        return Err(CustomBlockExplorerError::MissingHost);
    }

    Ok(url)
}

fn parse_http_url_with_optional_scheme(input: &str) -> Result<Url, CustomBlockExplorerError> {
    match parse_http_url(input) {
        Ok(url) => Ok(url),
        Err(error) if !input.contains("://") => {
            let input_with_scheme = format!("https://{input}");
            parse_http_url(&input_with_scheme).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

fn canonical_base_path(url: &Url, network: Network, placeholder_marker: &str) -> String {
    let path = url.path().trim_end_matches('/').trim_start_matches('/');
    let path = canonicalize_known_host_path(url.host_str(), network, path);

    if path.is_empty() {
        format!("/tx/{placeholder_marker}")
    } else {
        format!("/{path}/tx/{placeholder_marker}")
    }
}

fn canonicalize_known_host_path(host: Option<&str>, network: Network, path: &str) -> String {
    let Some(host) = host else {
        return path.to_string();
    };

    let Some(prefix) = known_host_network_prefix(host, network) else {
        return path.to_string();
    };

    if prefix.is_empty() || path == prefix || path.starts_with(&format!("{prefix}/")) {
        return path.to_string();
    }

    if path.is_empty() { prefix.to_string() } else { format!("{prefix}/{path}") }
}

fn known_host_network_prefix(host: &str, network: Network) -> Option<&'static str> {
    match (host, network) {
        ("mempool.space", Network::Bitcoin) => Some(""),
        ("mempool.space", Network::Testnet) => Some("testnet"),
        ("mempool.space", Network::Testnet4) => Some("testnet4"),
        ("mutinynet.com", Network::Signet) => Some(""),
        _ => None,
    }
}

fn validate_placeholder_location(
    url: &Url,
    placeholder_marker: &str,
) -> Result<(), CustomBlockExplorerError> {
    let placeholder_in_path = url.path().contains(placeholder_marker);
    let placeholder_in_query = url.query().is_some_and(|query| query.contains(placeholder_marker));
    let placeholder_in_user_info = url.username().contains(placeholder_marker)
        || url.password().is_some_and(|password| password.contains(placeholder_marker));
    let placeholder_in_host = url.host_str().is_some_and(|host| host.contains(placeholder_marker));

    if placeholder_in_user_info || placeholder_in_host {
        return Err(CustomBlockExplorerError::UnsupportedPlaceholder);
    }

    if placeholder_in_path || placeholder_in_query {
        return Ok(());
    }

    Err(CustomBlockExplorerError::UnsupportedPlaceholder)
}

fn placeholder_marker_absent_from(input: &str) -> String {
    let mut marker = PLACEHOLDER_MARKER_SEED.to_string();
    while input.contains(&marker) {
        marker.push('_');
    }

    marker
}

fn validate_placeholders(input: &str) -> Result<(), CustomBlockExplorerError> {
    let mut chars = input.char_indices().peekable();

    while let Some((index, character)) = chars.next() {
        match character {
            '{' => {
                let Some((end_index, _)) = chars.find(|(_, character)| *character == '}') else {
                    return Err(CustomBlockExplorerError::UnsupportedPlaceholder);
                };

                let token = &input[index..=end_index];
                if !SUPPORTED_PLACEHOLDERS.contains(&token) {
                    return Err(CustomBlockExplorerError::UnsupportedPlaceholder);
                }
            }
            '}' => return Err(CustomBlockExplorerError::UnsupportedPlaceholder),
            _ => {}
        }
    }

    Ok(())
}

fn contains_supported_placeholder(input: &str) -> bool {
    SUPPORTED_PLACEHOLDERS.iter().any(|placeholder| input.contains(placeholder))
}

fn replace_supported_placeholders(input: &str, replacement: &str) -> String {
    SUPPORTED_PLACEHOLDERS
        .iter()
        .fold(input.to_string(), |value, placeholder| value.replace(placeholder, replacement))
}

#[cfg(test)]
mod tests {
    use super::{BlockExplorerOption, CustomBlockExplorerError, CustomBlockExplorerTemplate};
    use crate::network::Network;

    const TXID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn block_explorer_options_expose_expected_order_and_labels() {
        let options = super::all_block_explorer_options();

        assert_eq!(
            options.as_slice(),
            &[
                BlockExplorerOption::MempoolSpace,
                BlockExplorerOption::MempoolGuide,
                BlockExplorerOption::BullBitcoin,
                BlockExplorerOption::Blockstream,
                BlockExplorerOption::Custom,
            ]
        );
        assert_eq!(BlockExplorerOption::MempoolSpace.display_name(), "Default (mempool.space)");
        assert_eq!(BlockExplorerOption::Blockstream.display_name(), "blockstream.info");
    }

    #[test]
    fn defaults_match_existing_transaction_urls() {
        assert_eq!(
            CustomBlockExplorerTemplate::default_for(Network::Bitcoin).render(TXID),
            "https://mempool.space/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            CustomBlockExplorerTemplate::default_for(Network::Testnet).render(TXID),
            "https://mempool.space/testnet/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            CustomBlockExplorerTemplate::default_for(Network::Testnet4).render(TXID),
            "https://mempool.space/testnet4/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            CustomBlockExplorerTemplate::default_for(Network::Signet).render(TXID),
            "https://mutinynet.com/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn placeholders_are_normalized_and_all_occurrences_render() {
        let template = CustomBlockExplorerTemplate::parse(
            Network::Bitcoin,
            "https://example.com/tx/{0}/again/{txid}?id={tx_id}",
        )
        .unwrap();

        assert_eq!(template.as_str(), "https://example.com/tx/{txid}/again/{txid}?id={txid}");
        assert_eq!(
            template.render(TXID),
            "https://example.com/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/again/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn query_placeholder_renders() {
        let template = CustomBlockExplorerTemplate::parse(
            Network::Bitcoin,
            "https://example.com/search?q={txid}",
        )
        .unwrap();

        assert_eq!(
            template.render(TXID),
            "https://example.com/search?q=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        let template =
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "example.com/search?q={txid}")
                .unwrap();

        assert_eq!(template.as_str(), "https://example.com/search?q={txid}");
    }

    #[test]
    fn plain_base_url_normalizes_to_transaction_template() {
        let cases = [
            (" https://example.com ", "https://example.com/tx/{txid}"),
            ("example.com", "https://example.com/tx/{txid}"),
            ("mempool.space", "https://mempool.space/tx/{txid}"),
            ("https://mempool.guide/", "https://mempool.guide/tx/{txid}"),
            ("mempool.guide", "https://mempool.guide/tx/{txid}"),
            ("https://mempool.bullbitcoin.com/", "https://mempool.bullbitcoin.com/tx/{txid}"),
            ("https://blockstream.info/", "https://blockstream.info/tx/{txid}"),
            ("blockstream.info", "https://blockstream.info/tx/{txid}"),
        ];

        for (input, expected) in cases {
            let template = CustomBlockExplorerTemplate::parse(Network::Bitcoin, input).unwrap();

            assert_eq!(template.as_str(), expected);
        }
    }

    #[test]
    fn plain_base_path_preserves_path_port_and_query() {
        let template = CustomBlockExplorerTemplate::parse(
            Network::Bitcoin,
            "http://192.168.1.10:3000/explorer/?source=cove",
        )
        .unwrap();

        assert_eq!(template.as_str(), "http://192.168.1.10:3000/explorer/tx/{txid}?source=cove");

        let template =
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "example.com/explorer").unwrap();

        assert_eq!(template.as_str(), "https://example.com/explorer/tx/{txid}");
    }

    #[test]
    fn literal_marker_text_is_not_rewritten_to_placeholder() {
        let template = CustomBlockExplorerTemplate::parse(
            Network::Bitcoin,
            "https://example.com/coveplaceholdertxid?q=coveplaceholdertxid",
        )
        .unwrap();

        assert_eq!(
            template.as_str(),
            "https://example.com/coveplaceholdertxid/tx/{txid}?q=coveplaceholdertxid"
        );

        let template = CustomBlockExplorerTemplate::parse(
            Network::Bitcoin,
            "https://example.com/coveplaceholdertxid/{txid}?q=coveplaceholdertxid",
        )
        .unwrap();

        assert_eq!(
            template.as_str(),
            "https://example.com/coveplaceholdertxid/{txid}?q=coveplaceholdertxid"
        );
    }

    #[test]
    fn known_hosts_canonicalize_network_path_before_transaction_path() {
        let testnet =
            CustomBlockExplorerTemplate::parse(Network::Testnet, "https://mempool.space").unwrap();
        let testnet4 =
            CustomBlockExplorerTemplate::parse(Network::Testnet4, "https://mempool.space/")
                .unwrap();
        let signet =
            CustomBlockExplorerTemplate::parse(Network::Signet, "https://mutinynet.com").unwrap();

        assert_eq!(testnet.as_str(), "https://mempool.space/testnet/tx/{txid}");
        assert_eq!(testnet4.as_str(), "https://mempool.space/testnet4/tx/{txid}");
        assert_eq!(signet.as_str(), "https://mutinynet.com/tx/{txid}");
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "").unwrap_err(),
            CustomBlockExplorerError::Empty
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "ftp://example.com").unwrap_err(),
            CustomBlockExplorerError::InvalidScheme
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "https://").unwrap_err(),
            CustomBlockExplorerError::InvalidUrl
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "https://example.com/#frag")
                .unwrap_err(),
            CustomBlockExplorerError::Fragment
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "https://example.com/{hash}")
                .unwrap_err(),
            CustomBlockExplorerError::UnsupportedPlaceholder
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "https://{txid}.example.com")
                .unwrap_err(),
            CustomBlockExplorerError::UnsupportedPlaceholder
        );
        assert_eq!(
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "https://user{txid}@example.com")
                .unwrap_err(),
            CustomBlockExplorerError::UnsupportedPlaceholder
        );
    }

    #[test]
    fn stored_values_must_be_valid_templates() {
        assert_eq!(
            CustomBlockExplorerTemplate::parse_stored("https://example.com").unwrap_err(),
            CustomBlockExplorerError::UnsupportedPlaceholder
        );

        let template =
            CustomBlockExplorerTemplate::parse_stored("https://example.com/tx/{0}").unwrap();
        assert_eq!(template.as_str(), "https://example.com/tx/{txid}");
    }
}

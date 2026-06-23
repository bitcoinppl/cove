use std::fmt;

use cove_types::Network;
use url::{Host, Url};

use super::BlockExplorerOption;

const PLACEHOLDER: &str = "{txid}";
const PLACEHOLDER_MARKER_SEED: &str = "coveplaceholdertxid";
const SUPPORTED_PLACEHOLDERS: [&str; 3] = ["{0}", "{txid}", "{tx_id}"];

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
    /// Parses user input into a canonical transaction URL template.
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

    /// Returns the built-in transaction URL template for a network.
    pub fn default_for(network: Network) -> Self {
        let template = match network {
            Network::Bitcoin => "https://mempool.space/tx/{txid}",
            Network::Testnet => "https://mempool.space/testnet/tx/{txid}",
            Network::Testnet4 => "https://mempool.space/testnet4/tx/{txid}",
            Network::Signet => "https://mutinynet.com/tx/{txid}",
        };

        Self(template.to_string())
    }

    /// Parses an already stored template and rejects values that are not complete templates.
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

    /// Returns the canonical template string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Renders this template with a transaction id.
    pub fn render(&self, txid: impl fmt::Display) -> String {
        let txid = txid.to_string();
        SUPPORTED_PLACEHOLDERS
            .iter()
            .fold(self.0.clone(), |rendered, placeholder| rendered.replace(placeholder, &txid))
    }

    pub(crate) fn from_preset_base_url(network: Network, input: &str) -> Option<Self> {
        parse_http_url(input).ok().map(|url| Self::from_base_url(network, url))
    }

    /// Parses input that already includes a supported transaction placeholder.
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

    /// Parses a bare explorer URL and expands it into a transaction template.
    fn parse_base_url(network: Network, input: &str) -> Result<Self, CustomBlockExplorerError> {
        let url = parse_http_url_with_optional_scheme(input)?;
        if url.fragment().is_some() {
            return Err(CustomBlockExplorerError::Fragment);
        }

        if let Some(template) = Self::known_for_input_url(network, &url) {
            return Ok(template);
        }

        Ok(Self::from_base_url(network, url))
    }

    /// Builds a transaction template from a validated base URL.
    fn from_base_url(network: Network, mut url: Url) -> Self {
        let placeholder_marker = placeholder_marker_absent_from(url.as_str());
        let path = canonical_base_path(&url, network, &placeholder_marker);
        url.set_path(&path);

        let canonical = url.to_string().replace(&placeholder_marker, PLACEHOLDER);
        Self(canonical)
    }

    fn known_for_input_url(network: Network, url: &Url) -> Option<Self> {
        BlockExplorerOption::all()
            .into_iter()
            .filter_map(|option| option.template_for_network(network))
            .find(|template| template.matches_input_url(url))
    }

    fn matches_input_url(&self, input_url: &Url) -> bool {
        if input_url.query().is_some() {
            return false;
        }

        let placeholder_marker = placeholder_marker_absent_from(self.as_str());
        let probe = self.as_str().replace(PLACEHOLDER, &placeholder_marker);
        let Ok(template_url) = parse_http_url(&probe) else {
            return false;
        };

        if input_url.scheme() != template_url.scheme()
            || input_url.host_str() != template_url.host_str()
            || input_url.port_or_known_default() != template_url.port_or_known_default()
        {
            return false;
        }

        let Some(transaction_path) =
            Self::normalized_transaction_path(&template_url, &placeholder_marker)
        else {
            return false;
        };
        let base_path = transaction_path
            .strip_suffix("/tx")
            .unwrap_or_else(|| if transaction_path == "tx" { "" } else { transaction_path });
        let input_path = normalized_path(input_url.path());

        input_path.is_empty() || input_path == base_path || input_path == transaction_path
    }

    fn normalized_transaction_path<'a>(url: &'a Url, marker: &str) -> Option<&'a str> {
        let path = normalized_path(url.path());
        let path = path.strip_suffix(marker)?;

        Some(path.trim_end_matches('/'))
    }
}

/// Parses a URL and accepts only HTTP(S) URLs with a host.
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

/// Parses a URL, inferring a scheme when the user omitted one.
fn parse_http_url_with_optional_scheme(input: &str) -> Result<Url, CustomBlockExplorerError> {
    match parse_http_url(input) {
        Ok(url) => Ok(url),
        Err(error) if !input.contains("://") => {
            let scheme = default_scheme_for_scheme_less_input(input);
            let input_with_scheme = format!("{scheme}://{input}");
            parse_http_url(&input_with_scheme).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

/// Returns the scheme to use for input that did not include one.
fn default_scheme_for_scheme_less_input(input: &str) -> &'static str {
    if scheme_less_input_prefers_http(input) { "http" } else { "https" }
}

/// Returns whether scheme-less input should default to HTTP.
fn scheme_less_input_prefers_http(input: &str) -> bool {
    let probe = format!("https://{input}");

    let Ok(url) = Url::parse(&probe) else {
        return false;
    };

    match url.host() {
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => true,
        Some(Host::Domain(host)) => {
            host.trim_end_matches('.').to_ascii_lowercase().ends_with(".local")
        }
        None => false,
    }
}

/// Normalizes a URL path for path comparison and composition.
fn normalized_path(path: &str) -> &str {
    path.trim_end_matches('/').trim_start_matches('/')
}

/// Returns the canonical path for a base URL plus transaction placeholder.
fn canonical_base_path(url: &Url, network: Network, placeholder_marker: &str) -> String {
    let path = normalized_path(url.path());
    let path = canonicalize_known_host_path(url.host_str(), network, path);

    if path.is_empty() {
        format!("/tx/{placeholder_marker}")
    } else if path.rsplit('/').next() == Some("tx") {
        format!("/{path}/{placeholder_marker}")
    } else {
        format!("/{path}/tx/{placeholder_marker}")
    }
}

/// Inserts the network path prefix required by known multi-network hosts.
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

/// Returns the network path prefix for hosts whose URL paths encode network selection.
fn known_host_network_prefix(host: &str, network: Network) -> Option<&'static str> {
    match (host, network) {
        ("mempool.space", Network::Bitcoin) => Some(""),
        ("mempool.space", Network::Testnet) => Some("testnet"),
        ("mempool.space", Network::Testnet4) => Some("testnet4"),
        ("mutinynet.com", Network::Signet) => Some(""),
        _ => None,
    }
}

/// Validates that the transaction placeholder appears only where URL rendering can replace it.
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

/// Creates a temporary marker that is not already present in the input.
fn placeholder_marker_absent_from(input: &str) -> String {
    let mut marker = PLACEHOLDER_MARKER_SEED.to_string();
    while input.contains(&marker) {
        marker.push('_');
    }

    marker
}

/// Validates that every brace-delimited placeholder is supported.
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

/// Returns whether the input includes a supported transaction placeholder.
fn contains_supported_placeholder(input: &str) -> bool {
    SUPPORTED_PLACEHOLDERS.iter().any(|placeholder| input.contains(placeholder))
}

/// Replaces every supported transaction placeholder with a temporary marker.
fn replace_supported_placeholders(input: &str, replacement: &str) -> String {
    SUPPORTED_PLACEHOLDERS
        .iter()
        .fold(input.to_string(), |value, placeholder| value.replace(placeholder, replacement))
}

#[cfg(test)]
mod tests {
    use cove_types::Network;

    use super::{CustomBlockExplorerError, CustomBlockExplorerTemplate};
    use crate::custom_block_explorer::effective_transaction_url;

    const TXID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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

        let template =
            CustomBlockExplorerTemplate::parse(Network::Bitcoin, "node.local/search?q={txid}")
                .unwrap();

        assert_eq!(template.as_str(), "http://node.local/search?q={txid}");
    }

    #[test]
    fn plain_base_url_normalizes_to_transaction_template() {
        let cases = [
            (" https://example.com ", "https://example.com/tx/{txid}"),
            ("example.com", "https://example.com/tx/{txid}"),
            ("example.com/tx", "https://example.com/tx/{txid}"),
            ("https://node.local", "https://node.local/tx/{txid}"),
            ("https://192.168.1.10:3000/explorer", "https://192.168.1.10:3000/explorer/tx/{txid}"),
            ("node.local", "http://node.local/tx/{txid}"),
            ("node.local:3000/explorer", "http://node.local:3000/explorer/tx/{txid}"),
            ("192.168.1.10:3000/explorer", "http://192.168.1.10:3000/explorer/tx/{txid}"),
            ("[::1]:3000/explorer", "http://[::1]:3000/explorer/tx/{txid}"),
            ("mempool.space", "https://mempool.space/tx/{txid}"),
            ("mempool.space/tx", "https://mempool.space/tx/{txid}"),
            ("https://mempool.guide/", "https://mempool.guide/tx/{txid}"),
            ("mempool.guide", "https://mempool.guide/tx/{txid}"),
            ("mempool.guide/tx", "https://mempool.guide/tx/{txid}"),
            ("https://mempool.bullbitcoin.com/", "https://mempool.bullbitcoin.com/tx/{txid}"),
            ("https://blockstream.info/", "https://blockstream.info/tx/{txid}"),
            ("blockstream.info", "https://blockstream.info/tx/{txid}"),
            ("blockstream.info/tx", "https://blockstream.info/tx/{txid}"),
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
        let testnet_tx =
            CustomBlockExplorerTemplate::parse(Network::Testnet, "mempool.space/testnet/tx")
                .unwrap();
        let signet_tx =
            CustomBlockExplorerTemplate::parse(Network::Signet, "mutinynet.com/tx").unwrap();

        assert_eq!(testnet.as_str(), "https://mempool.space/testnet/tx/{txid}");
        assert_eq!(testnet4.as_str(), "https://mempool.space/testnet4/tx/{txid}");
        assert_eq!(signet.as_str(), "https://mutinynet.com/tx/{txid}");
        assert_eq!(testnet_tx.as_str(), "https://mempool.space/testnet/tx/{txid}");
        assert_eq!(signet_tx.as_str(), "https://mutinynet.com/tx/{txid}");
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

        let template =
            CustomBlockExplorerTemplate::parse_stored("https://node.local/tx/{txid}").unwrap();
        assert_eq!(template.as_str(), "https://node.local/tx/{txid}");
    }

    #[test]
    fn effective_transaction_url_preserves_explicit_local_and_ip_schemes() {
        assert_eq!(
            effective_transaction_url(Network::Bitcoin, Some("https://node.local/tx/{txid}"), TXID),
            "https://node.local/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        assert_eq!(
            effective_transaction_url(
                Network::Bitcoin,
                Some("https://192.168.1.10:3000/explorer/tx/{txid}"),
                TXID
            ),
            "https://192.168.1.10:3000/explorer/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }
}

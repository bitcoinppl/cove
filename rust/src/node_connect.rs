use tracing::error;
use url::Url;

use crate::node::client::electrum::transport;
use crate::node::tls::{self, TlsTrust};
use crate::{database::Database, network::Network, node::Node};
use cove_macros::impl_default_for;
use eyre::{Context, eyre};

pub const BITCOIN_ESPLORA: [(&str, &str); 1] =
    [("blockstream.info", "https://blockstream.info/api/")];

// self-signed SSL presets: electrum.emzy.de, electrum.bitaroo.net,
// fulcrum.sethforprivacy.com, electrum1.bluewallet.io
// enable these if Cove supports self-signed SSL certs later
pub const BITCOIN_ELECTRUM: [(&str, &str); 3] = [
    ("fulcrum.bullbitcoin.com", "ssl://fulcrum.bullbitcoin.com:50002"),
    ("electrum.blockstream.info", "ssl://electrum.blockstream.info:50002"),
    ("electrum.diynodes.com", "ssl://electrum.diynodes.com:50022"),
];

pub const TESTNET_ESPLORA: [(&str, &str); 2] = [
    ("mempool.space", "https://mempool.space/testnet/api/"),
    ("blockstream.info", "https://blockstream.info/testnet/api/"),
];

pub const TESTNET_ELECTRUM: [(&str, &str); 1] =
    [("testnet.hsmiths.com", "ssl://testnet.hsmiths.com:53012")];

pub const TESTNET4_ESPLORA: [(&str, &str); 1] =
    [("mempool.space", "https://mempool.space/testnet4/api/")];

pub const TESTNET4_ELECTRUM: [(&str, &str); 1] =
    [("mempool.space electrum", "ssl://mempool.space:40002")];

pub const SIGNET_ESPLORA: [(&str, &str); 1] = [("mutinynet", "https://mutinynet.com/api")];

#[derive(Debug, Clone, uniffi::Object)]
pub struct NodeSelector {
    network: Network,
    node_list: Vec<NodeSelection>,
}

#[derive(Debug, Clone, uniffi::Enum, PartialEq, Eq, Hash)]
pub enum NodeSelection {
    Preset(Node),
    Custom(Node),
}

type Error = NodeSelectorError;

#[derive(Debug, Clone, uniffi::Enum, PartialEq, Eq, Hash, thiserror::Error)]
pub enum NodeSelectorError {
    #[error("node with name {0} not found")]
    NodeNotFound(String),

    #[error("unable to set selected node: {0}")]
    SetSelectedNodeError(String),

    #[error("unable to access node: {0}")]
    NodeAccessError(String),

    #[error("unable to parse node url: {0}")]
    ParseNodeUrlError(String),

    #[error("unable to read the server's certificate: {0}")]
    ReadCertificateError(String),

    #[error("the server's certificate is not trusted")]
    CertificateNotTrusted,

    #[error("saving this node would forget the certificate it trusts")]
    CertificateWouldBeForgotten,
}

/// What to do about a node whose certificate was rejected.
#[derive(Debug, Clone, uniffi::Enum, PartialEq, Eq, Hash)]
pub enum CertificateDecision {
    /// Nothing is trusted for this url yet, so the certificate can be offered
    /// for the user to accept.
    Unrecognized { certificate: NodeCertificate },

    /// This url already trusts a different certificate. Offering to accept the
    /// new one would undo the decision the user already made, so it is reported
    /// rather than asked about.
    Changed,
}

/// A certificate a server presented, offered to the user for confirmation.
#[derive(Debug, Clone, uniffi::Record, PartialEq, Eq, Hash)]
pub struct NodeCertificate {
    /// SHA-256 of the certificate, ready to store as [`TlsTrust`].
    pub sha256: Vec<u8>,

    /// The same value as colon separated hex, so it can be compared against
    /// what the server operator sees.
    pub display: String,
}

impl_default_for!(NodeSelector);
#[uniffi::export(async_runtime = "tokio")]
impl NodeSelector {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let network = Database::global().global_config.selected_network();
        let selected_node = Database::global().global_config.selected_node();

        let node_list = node_list(network);

        let node_selection_list = if node_list.contains(&selected_node) {
            node_list.into_iter().map(NodeSelection::Preset).collect()
        } else {
            let mut node_selection_list =
                node_list.into_iter().map(NodeSelection::Preset).collect::<Vec<NodeSelection>>();

            node_selection_list.push(NodeSelection::Custom(selected_node.clone()));
            node_selection_list
        };

        Self { network, node_list: node_selection_list }
    }

    #[uniffi::method]
    pub fn node_list(&self) -> Vec<NodeSelection> {
        self.node_list.clone()
    }

    #[uniffi::method]
    pub fn selected_node(&self) -> NodeSelection {
        let selected_node = Database::global().global_config.selected_node();

        if node_list(self.network).contains(&selected_node) {
            NodeSelection::Preset(selected_node)
        } else {
            NodeSelection::Custom(selected_node)
        }
    }

    #[uniffi::method]
    pub fn select_preset_node(&self, name: String) -> Result<Node, Error> {
        let node = node_list(self.network)
            .into_iter()
            .find(|node| node.name == name)
            .or_else(|| {
                let selected_node = Database::global().global_config.selected_node();
                if selected_node.name == name { Some(selected_node) } else { None }
            })
            .ok_or_else(|| {
                error!("node with name {name} not found");
                NodeSelectorError::NodeNotFound(name)
            })?;

        Database::global()
            .global_config
            .set_selected_node(&node)
            .map_err(|error| NodeSelectorError::SetSelectedNodeError(error.to_string()))?;

        Ok(node)
    }

    #[uniffi::method]
    pub async fn check_selected_node(&self, node: Node) -> Result<(), Error> {
        node.check_url().await.map_err(|error| Error::NodeAccessError(format!("{error:?}")))?;

        Ok(())
    }

    #[uniffi::method(default(tls = None))]
    /// Use the url and name of the custom node to set it as the selected node
    pub fn parse_custom_node(
        &self,
        url: String,
        name: String,
        entered_name: String,
        tls: Option<TlsTrust>,
    ) -> Result<Node, Error> {
        let node_type = name.to_ascii_lowercase();

        let url =
            parse_node_url(&url).map_err(|error| Error::ParseNodeUrlError(error.to_string()))?;

        if !has_usable_host(&url) {
            return Err(Error::ParseNodeUrlError("invalid url, no domain".to_string()));
        }

        let url_string = url.to_string();

        let name = if entered_name.is_empty() {
            url.domain().unwrap_or(url_string.as_str()).to_string()
        } else {
            entered_name
        };

        let node = if node_type.contains("electrum") {
            // Only an ssl:// node can present a certificate, and failing here
            // says so rather than leaving it to a generic connection error.
            if tls.is_some() && !url_string.starts_with("ssl://") {
                return Err(Error::ParseNodeUrlError(
                    "custom certificates require an ssl:// url".to_string(),
                ));
            }

            Node { tls, ..Node::new_electrum(name, url_string, self.network) }
        } else if node_type.contains("esplora") {
            // Silently dropping the setting here would contradict the Esplora
            // client, which refuses a node it cannot honor.
            if tls.is_some() {
                return Err(Error::ParseNodeUrlError(
                    "esplora nodes do not support custom certificate settings".to_string(),
                ));
            }

            Node::new_esplora(name, url_string, self.network)
        } else {
            error!("invalid node type: {node_type}");
            Node::default(self.network)
        };

        Ok(node)
    }

    #[uniffi::method]
    /// Decide what a rejected certificate means for this url.
    ///
    /// Deciding here rather than in each app keeps one rule: a url that already
    /// trusts a certificate is never offered a different one.
    pub async fn certificate_decision(&self, url: String) -> Result<CertificateDecision, Error> {
        let url = normalized_url(&url)?;

        if self.trusted_certificate(&url).is_some() {
            return Ok(CertificateDecision::Changed);
        }

        let certificate = self.fetch_node_certificate(url).await?;
        Ok(CertificateDecision::Unrecognized { certificate })
    }

    #[uniffi::method]
    /// Read the certificate a server presents, so it can be shown to the user.
    ///
    /// The certificate is not verified. It is only trusted once the user has
    /// compared the fingerprint against their server and accepted it.
    pub async fn fetch_node_certificate(&self, url: String) -> Result<NodeCertificate, Error> {
        let url = normalized_url(&url)?;

        let certificate =
            cove_tokio::unblock::run_blocking(move || transport::peer_certificate(&url))
                .await
                .map_err(|error| Error::ReadCertificateError(error.to_string()))?;

        let sha256 = tls::fingerprint(&certificate);

        Ok(NodeCertificate { sha256: sha256.to_vec(), display: tls::display_fingerprint(&sha256) })
    }

    #[uniffi::method]
    /// Check the node url and set it as selected node if it is valid
    pub async fn check_and_save_node(&self, node: Node) -> Result<(), Error> {
        // A caller that forgets to carry the settings forward would otherwise
        // quietly drop the certificate the user chose to trust.
        let saved = Database::global().global_config.selected_node();
        if would_forget_certificate(&saved, &node) {
            return Err(Error::CertificateWouldBeForgotten);
        }

        node.check_url().await.map_err(|error| {
            tracing::warn!("error checking node: {error:?}");

            // Distinguished so the caller can offer to trust the certificate
            // instead of showing a generic failure.
            if error.is_certificate_error() {
                return Error::CertificateNotTrusted;
            }

            Error::NodeAccessError(error.to_string())
        })?;

        Database::global()
            .global_config
            .set_selected_node(&node)
            .map_err(|error| Error::SetSelectedNodeError(error.to_string()))?;

        Ok(())
    }
}

fn node_list(network: Network) -> Vec<Node> {
    match network {
        Network::Bitcoin => {
            let mut nodes = BITCOIN_ELECTRUM
                .iter()
                .map(|(name, url)| Node::new_electrum(name.to_string(), url.to_string(), network))
                .collect::<Vec<Node>>();

            nodes.extend(
                BITCOIN_ESPLORA.iter().map(|(name, url)| {
                    Node::new_esplora(name.to_string(), url.to_string(), network)
                }),
            );

            nodes
        }

        Network::Testnet => {
            let mut nodes = TESTNET_ELECTRUM
                .iter()
                .map(|(name, url)| Node::new_electrum(name.to_string(), url.to_string(), network))
                .collect::<Vec<Node>>();

            nodes.extend(
                TESTNET_ESPLORA.iter().map(|(name, url)| {
                    Node::new_esplora(name.to_string(), url.to_string(), network)
                }),
            );

            nodes
        }

        Network::Signet => SIGNET_ESPLORA
            .iter()
            .map(|(name, url)| Node::new_esplora(name.to_string(), url.to_string(), network))
            .collect::<Vec<Node>>(),

        Network::Testnet4 => {
            let mut nodes = TESTNET4_ESPLORA
                .iter()
                .map(|(name, url)| Node::new_esplora(name.to_string(), url.to_string(), network))
                .collect::<Vec<Node>>();

            nodes.extend(
                TESTNET4_ELECTRUM.iter().map(|(name, url)| {
                    Node::new_electrum(name.to_string(), url.to_string(), network)
                }),
            );

            nodes
        }
    }
}

impl NodeSelector {
    fn trusted_certificate(&self, url: &str) -> Option<TlsTrust> {
        trusted_certificate(&Database::global().global_config.selected_node(), url)
    }
}

/// The certificate `saved` trusts for `url`, which is only its own certificate
/// settings and only when it is the same url.
fn trusted_certificate(saved: &Node, url: &str) -> Option<TlsTrust> {
    (saved.url == url).then(|| saved.tls.clone()).flatten()
}

/// Whether saving `node` would drop a certificate `saved` already trusts.
fn would_forget_certificate(saved: &Node, node: &Node) -> bool {
    node.tls.is_none() && trusted_certificate(saved, &node.url).is_some()
}

fn normalized_url(url: &str) -> Result<String, Error> {
    let url = parse_node_url(url)
        .map_err(|error| Error::ParseNodeUrlError(error.to_string()))?
        .to_string();

    Ok(url.strip_suffix('/').unwrap_or(&url).to_string())
}

/// A url is usable when it names a host we can actually reach: a dotted domain
/// or a literal IP address, which is how self hosted servers are often reached.
fn has_usable_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.contains('.'),
        Some(_) => true,
        None => false,
    }
}

fn parse_node_url(url: &str) -> eyre::Result<Url> {
    let url = url.replace("http://", "tcp://");
    let url = url.replace("https://", "ssl://");

    let mut url = if url.contains("://") {
        Url::parse(&url)?
    } else {
        let url_str = format!("none://{url}/");
        Url::parse(&url_str)?
    };

    // set the scheme properly, use the port as a hint
    match (url.scheme(), url.port()) {
        ("none", Some(50002)) => url
            .set_scheme("ssl")
            .map_err(|()| eyre!("can't set scheme to ssl"))
            .context("original: none, port is 50002")?,
        ("none", Some(50001)) => url
            .set_scheme("tcp")
            .map_err(|()| eyre!("can't set scheme to tcp"))
            .context("original: none, port is 50001")?,
        ("none", port) => {
            url.set_scheme("tcp")
                .map_err(|()| eyre!("can't set scheme to tcp"))
                .wrap_err_with(|| format!("original: none, port is {port:?}"))?;
        }
        _ => {}
    }

    // set the port to if not set, default to 50002 for ssl and 50001 for tcp
    match (url.port(), url.scheme()) {
        (Some(_), _) => {}
        (None, "ssl") => url.set_port(Some(50002)).map_err(|()| eyre!("can't set port"))?,
        (None, "tcp") => url.set_port(Some(50001)).map_err(|()| eyre!("can't set port"))?,
        (None, _) => {
            url.set_port(Some(50002)).map_err(|()| eyre!("can't set port"))?;
        }
    }

    Ok(url)
}

#[uniffi::export]
impl NodeSelection {
    fn to_node(&self) -> Node {
        self.clone().into()
    }
}

#[uniffi::export]
fn default_node_selection() -> NodeSelection {
    let network = Database::global().global_config.selected_network();

    match network {
        Network::Bitcoin => {
            let (name, url) = BITCOIN_ELECTRUM[0];
            NodeSelection::Preset(Node::new_electrum(name.to_string(), url.to_string(), network))
        }
        Network::Testnet => {
            let (name, url) = TESTNET_ESPLORA[0];
            NodeSelection::Preset(Node::new_esplora(name.to_string(), url.to_string(), network))
        }
        Network::Signet => {
            let (name, url) = SIGNET_ESPLORA[0];
            NodeSelection::Preset(Node::new_esplora(name.to_string(), url.to_string(), network))
        }
        Network::Testnet4 => {
            let (name, url) = TESTNET4_ESPLORA[0];
            NodeSelection::Preset(Node::new_esplora(name.to_string(), url.to_string(), network))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector() -> NodeSelector {
        NodeSelector { network: Network::Bitcoin, node_list: Vec::new() }
    }

    fn parse(url: &str) -> Result<Node, Error> {
        selector().parse_custom_node(
            url.to_string(),
            "Custom Electrum".to_string(),
            String::new(),
            None,
        )
    }

    #[test]
    fn custom_nodes_keep_their_certificate_settings() {
        let trust = TlsTrust::PinnedFingerprint { sha256: vec![3; 32] };

        let node = selector()
            .parse_custom_node(
                "ssl://node.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(trust.clone()),
            )
            .unwrap();

        assert_eq!(node.tls, Some(trust));
    }

    /// Self hosted servers are commonly reached by address rather than by name.
    #[test]
    fn nodes_can_be_reached_by_ip_address() {
        assert_eq!(parse("ssl://192.168.1.50:50002").unwrap().url, "ssl://192.168.1.50:50002");
        assert_eq!(parse("ssl://[fd00::1]:50002").unwrap().url, "ssl://[fd00::1]:50002");
    }

    #[test]
    fn esplora_nodes_reject_certificate_settings() {
        let error = selector()
            .parse_custom_node(
                "https://esplora.example.com".to_string(),
                "Custom Esplora".to_string(),
                String::new(),
                Some(TlsTrust::PinnedFingerprint { sha256: vec![1; 32] }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::ParseNodeUrlError(_)), "{error}");
    }

    #[test]
    fn certificate_settings_require_an_ssl_url() {
        let error = selector()
            .parse_custom_node(
                "tcp://node.example.com:50001".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(TlsTrust::PinnedFingerprint { sha256: vec![1; 32] }),
            )
            .unwrap_err();

        assert!(matches!(error, Error::ParseNodeUrlError(_)), "{error}");
    }

    fn pinned(url: &str) -> Node {
        Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![4; 32] }),
            ..Node::new_electrum("saved".to_string(), url.to_string(), Network::Bitcoin)
        }
    }

    #[test]
    fn a_certificate_is_only_trusted_for_the_url_it_was_accepted_for() {
        let saved = pinned("ssl://node.example.com:50002");

        assert!(trusted_certificate(&saved, "ssl://node.example.com:50002").is_some());
        assert!(trusted_certificate(&saved, "ssl://other.example.com:50002").is_none());
    }

    #[test]
    fn a_node_without_a_certificate_trusts_nothing() {
        let saved = Node::default(Network::Bitcoin);

        assert!(trusted_certificate(&saved, &saved.url).is_none());
    }

    /// Dropping the settings on the way in is how the trust prompt turned into a
    /// question asked on every save.
    #[test]
    fn saving_a_node_may_not_forget_the_certificate_it_trusts() {
        let saved = pinned("ssl://node.example.com:50002");

        let forgetful = Node { tls: None, ..saved.clone() };
        assert!(would_forget_certificate(&saved, &forgetful));

        assert!(!would_forget_certificate(&saved, &saved));
        assert!(!would_forget_certificate(&Node::default(Network::Bitcoin), &forgetful));

        // A different url has its own trust, so it is not being forgotten.
        let elsewhere =
            Node { tls: None, url: "ssl://other.example.com:50002".to_string(), ..saved.clone() };
        assert!(!would_forget_certificate(&saved, &elsewhere));
    }

    #[test]
    fn a_url_without_a_host_is_rejected() {
        assert!(parse("ssl://nodomain:50002").is_err());
    }
}

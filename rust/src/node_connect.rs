use tracing::error;
use url::{Host, Url};

use crate::{database::Database, network::Network, node::Node};
use cove_macros::impl_default_for;
use cove_util::result_ext::ResultExt as _;
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

            node_selection_list.push(NodeSelection::Custom(selected_node));
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
            .map_err_str(NodeSelectorError::SetSelectedNodeError)?;

        Ok(node)
    }

    #[uniffi::method]
    pub async fn check_selected_node(&self, node: Node) -> Result<(), Error> {
        node.check_url().await.map_err_debug(Error::NodeAccessError)?;

        Ok(())
    }

    #[uniffi::method]
    /// Use the url and name of the custom node to set it as the selected node
    pub fn parse_custom_node(
        &self,
        url: String,
        name: String,
        entered_name: String,
    ) -> Result<Node, Error> {
        let node_type = name.to_ascii_lowercase();

        // the url rules depend on the api: Electrum uses ssl/tcp with default ports, while an
        // Esplora url is an http(s) address that must keep its scheme and port as entered
        let url = if node_type.contains("esplora") {
            parse_esplora_url(&url)
        } else {
            parse_node_url(&url)
        }
        .map_err_str(Error::ParseNodeUrlError)?;

        // http(s) urls parse an ip address as an ip host with no domain, so a local Esplora
        // server is valid here, while a named host still needs a dot
        let has_valid_host = match url.host() {
            Some(Host::Domain(domain)) => domain.contains('.'),
            Some(Host::Ipv4(_) | Host::Ipv6(_)) => true,
            None => false,
        };

        if !has_valid_host {
            return Err(Error::ParseNodeUrlError("invalid url, no domain".to_string()));
        }

        let url_string = url.to_string();

        let name = if entered_name.is_empty() {
            url.host_str().unwrap_or(url_string.as_str()).to_string()
        } else {
            entered_name
        };

        let node = if node_type.contains("electrum") {
            Node::new_electrum(name, url_string, self.network)
        } else if node_type.contains("esplora") {
            Node::new_esplora(name, url_string, self.network)
        } else {
            error!("invalid node type: {node_type}");
            Node::default(self.network)
        };

        Ok(node)
    }

    #[uniffi::method]
    /// Check the node url and set it as selected node if it is valid
    pub async fn check_and_save_node(&self, node: Node) -> Result<(), Error> {
        node.check_url().await.map_err(|error| {
            tracing::warn!("error checking node: {error:?}");
            Error::NodeAccessError(error.to_string())
        })?;

        Database::global()
            .global_config
            .set_selected_node(&node)
            .map_err_str(Error::SetSelectedNodeError)?;

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

/// Parses an Esplora url, defaulting to https when no scheme is given
fn parse_esplora_url(url: &str) -> eyre::Result<Url> {
    let url =
        if url.contains("://") { Url::parse(url)? } else { Url::parse(&format!("https://{url}"))? };

    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(eyre!("unsupported scheme {scheme} for an Esplora node, use http or https")),
    }
}

/// Parses an Electrum url into an ssl or tcp url with a port, using the port as a scheme hint
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

#[cfg(test)]
mod tests {
    use bitcoin::Network as BitcoinNetwork;

    use super::*;
    use crate::{
        node::ApiType,
        test_support::{ensure_tokio_runtime, fake_node::FakeNode, global_state_test_lock},
    };

    fn selector(network: Network) -> NodeSelector {
        NodeSelector { network, node_list: Vec::new() }
    }

    // regression for #928: the Electrum-only url rules turned an Esplora url into ssl://host:50002
    #[test]
    fn parse_custom_node_keeps_an_esplora_url_as_entered() {
        let node = selector(Network::Signet)
            .parse_custom_node(
                "https://mutinynet.com/api".to_string(),
                "Custom Esplora".to_string(),
                String::new(),
            )
            .unwrap();

        assert_eq!(node.api_type, ApiType::Esplora);
        assert_eq!(node.url, "https://mutinynet.com/api");
        assert_eq!(node.name, "mutinynet.com");
    }

    #[test]
    fn parse_custom_node_accepts_a_local_esplora_ip_address() {
        let node = selector(Network::Signet)
            .parse_custom_node(
                "http://192.168.1.10:3002".to_string(),
                "Custom Esplora".to_string(),
                String::new(),
            )
            .unwrap();

        assert_eq!(node.url, "http://192.168.1.10:3002/");
        assert_eq!(node.name, "192.168.1.10");
    }

    #[test]
    fn parse_custom_node_maps_electrum_urls_to_electrum_schemes_and_ports() {
        let selector = selector(Network::Bitcoin);
        let parse = |url: &str| {
            selector
                .parse_custom_node(url.to_string(), "Custom Electrum".to_string(), String::new())
                .unwrap()
        };

        let node = parse("https://electrum.example.com");
        assert_eq!(node.api_type, ApiType::Electrum);
        assert_eq!(node.url, "ssl://electrum.example.com:50002");

        assert_eq!(parse("http://electrum.example.com").url, "tcp://electrum.example.com:50001");
        assert_eq!(parse("electrum.example.com:50002").url, "ssl://electrum.example.com:50002/");
        assert_eq!(parse("electrum.example.com").url, "tcp://electrum.example.com:50001/");
    }

    fn selected_node() -> Node {
        Database::global().global_config.selected_node_for_network(Network::Bitcoin)
    }

    fn reset_selected_node() {
        Database::global()
            .global_config
            .set_selected_node(&Node::default(Network::Bitcoin))
            .expect("default node is saved");
    }

    #[tokio::test]
    async fn reachable_custom_nodes_are_saved_as_the_selected_node() {
        let _guard = global_state_test_lock().lock().await;
        ensure_tokio_runtime();
        crate::database::test_support::init_test_database();
        reset_selected_node();

        let node = FakeNode::start(BitcoinNetwork::Bitcoin);
        let selector = NodeSelector::new();

        for custom in [node.esplora_node("custom esplora"), node.electrum_node("custom electrum")] {
            selector.check_and_save_node(custom.clone()).await.expect("a reachable node is saved");
            assert_eq!(selected_node(), custom);
        }

        reset_selected_node();
    }

    #[tokio::test]
    async fn unreachable_custom_nodes_are_rejected_and_leave_the_selection_alone() {
        let _guard = global_state_test_lock().lock().await;
        ensure_tokio_runtime();
        crate::database::test_support::init_test_database();
        reset_selected_node();

        let selector = NodeSelector::new();
        let mut offline = FakeNode::start(BitcoinNetwork::Bitcoin);
        let refused_electrum = offline.electrum_node("dead electrum");
        let refused_esplora = offline.esplora_node("dead esplora");
        offline.go_offline();

        // a server that answers, but with an error, is as unusable as a missing one
        let broken = FakeNode::start(BitcoinNetwork::Bitcoin);
        broken.on("/blocks/tip/height").fail(400, "not an esplora server");
        let erroring_esplora = broken.esplora_node("erroring esplora");

        let before = selected_node();

        for rejected in [refused_electrum, refused_esplora, erroring_esplora] {
            let name = rejected.name.clone();
            let error = selector
                .check_and_save_node(rejected)
                .await
                .expect_err(&format!("{name} must be rejected"));

            assert!(matches!(error, Error::NodeAccessError(_)), "{name}: {error:?}");
            assert_eq!(selected_node(), before, "{name} must not replace the selected node");
        }

        reset_selected_node();
    }
}

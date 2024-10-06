use tracing::error;
use url::Url;

use crate::{database::Database, network::Network, node::Node};
use macros::impl_default_for;

pub const BITCOIN_ESPLORA: [(&str, &str); 2] = [
    ("blockstream.info", "https://blockstream.info/api/"),
    ("mempool.space", "https://mempool.space/api/"),
];

pub const BITCOIN_ELECTRUM: [(&str, &str); 3] = [
    (
        "electrum.blockstream.info",
        "ssl://electrum.blockstream.info:50002",
    ),
    ("mempool.space electrum", "ssl://mempool.space:50002"),
    ("electrum.diynodes.com", "ssl://electrum.diynodes.com:50022"),
];

pub const TESTNET_ESPLORA: [(&str, &str); 2] = [
    ("mempool.space", "https://mempool.space/testnet/api/"),
    ("blockstream.info", "https://blockstream.info/testnet/api/"),
];

pub const TESTNET_ELECTRUM: [(&str, &str); 1] =
    [("testnet.hsmiths.com", "ssl://testnet.hsmiths.com:53012")];

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
            let mut node_selection_list = node_list
                .into_iter()
                .map(NodeSelection::Preset)
                .collect::<Vec<NodeSelection>>();

            node_selection_list.push(NodeSelection::Custom(selected_node.clone()));
            node_selection_list
        };

        Self {
            network,
            node_list: node_selection_list,
        }
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
                if selected_node.name == name {
                    Some(selected_node)
                } else {
                    None
                }
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
        node.check_url()
            .await
            .map_err(|error| Error::NodeAccessError(format!("{error:?}")))?;

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

        let url =
            parse_node_url(&url).map_err(|error| Error::ParseNodeUrlError(error.to_string()))?;

        if !url.domain().unwrap_or_default().contains('.') {
            return Err(Error::ParseNodeUrlError(
                "invalid url, no domain".to_string(),
            ));
        }

        let url_string = url.to_string();

        let name = if entered_name.is_empty() {
            url.domain().unwrap_or(url_string.as_str()).to_string()
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
    }
}

fn parse_node_url(url: &str) -> Result<Url, url::ParseError> {
    let mut url = if url.contains("://") {
        Url::parse(url)?
    } else {
        let url_str = format!("none://{url}/");
        Url::parse(&url_str)?
    };

    // set the scheme properly, use the port as a hint
    match (url.scheme(), url.port()) {
        ("https", _) => url.set_scheme("ssl").expect("set scheme"),
        ("http", _) => url.set_scheme("tcp").expect("set scheme"),
        ("none", Some(50002)) => url.set_scheme("ssl").expect("set scheme"),
        ("none", Some(50001)) => url.set_scheme("tcp").expect("set scheme"),
        ("none", _) => url.set_scheme("tcp").expect("set scheme"),
        _ => {}
    };

    // set the port to if not set, default to 50002 for ssl and 50001 for tcp
    match (url.port(), url.scheme()) {
        (Some(_), _) => {}
        (None, "ssl") => url.set_port(Some(50002)).expect("set port"),
        (None, "tcp") => url.set_port(Some(50001)).expect("set port"),
        (None, _) => {
            error!("invalid node url: {url}, should already be set");
            url.set_port(Some(50002)).expect("set port")
        }
    };

    Ok(url)
}

mod ffi {
    use super::NodeSelection;
    use crate::node::Node;

    #[uniffi::export]
    pub fn node_selection_to_node(node: NodeSelection) -> Node {
        node.into()
    }
}

#[uniffi::export]
pub fn default_node_selection() -> NodeSelection {
    let network = Database::global().global_config.selected_network();

    let (name, url) = match network {
        Network::Bitcoin => BITCOIN_ESPLORA[0],
        Network::Testnet => TESTNET_ESPLORA[0],
    };

    NodeSelection::Preset(Node::new_esplora(
        name.to_string(),
        url.to_string(),
        network,
    ))
}

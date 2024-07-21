use tracing::error;

use crate::{database::Database, impl_default_for, network::Network, node::Node};

pub const BITCOIN_ESPLORA: (&str, &str) = ("blockstream.info", "https://blockstream.info/api/");

const BITCOIN_ELECTRUM: [(&str, &str); 4] = [
    ("bitcoin.lu.ke", "bitcoin.lu.ke"),
    ("electrum.emzy.de", "electrum.emzy.de"),
    ("electrum.bitaroo.net", "electrum.bitaroo.net"),
    ("electrum.diynodes.com", "electrum.diynodes.com"),
];

const TESTNET_ESPLORA: (&str, &str) = ("blockstream.info", "https://blockstream.info/testnet/api/");

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
        let Some(node) = node_list(self.network)
            .into_iter()
            .find(|node| node.name == name)
        else {
            error!("node with name {name} not found");
            return Err(NodeSelectorError::NodeNotFound(name));
        };

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
            .map_err(|error| Error::NodeAccessError(error.to_string()))?;

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

            let (name, url) = BITCOIN_ESPLORA;
            nodes.push(Node::new_esplora(
                name.to_string(),
                url.to_string(),
                network,
            ));

            nodes
        }

        Network::Testnet => {
            let (name, url) = TESTNET_ESPLORA;
            vec![Node::new_esplora(
                name.to_string(),
                url.to_string(),
                network,
            )]
        }
    }
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
        Network::Bitcoin => BITCOIN_ESPLORA,
        Network::Testnet => TESTNET_ESPLORA,
    };

    NodeSelection::Preset(Node::new_esplora(
        name.to_string(),
        url.to_string(),
        network,
    ))
}

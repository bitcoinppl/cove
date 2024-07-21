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
    selected_node: NodeSelection,
    node_list: Vec<NodeSelection>,
}

#[derive(Debug, Clone, uniffi::Enum, PartialEq, Eq, Hash)]
pub enum NodeSelection {
    Preset(Node),
    Custom(Node),
}

impl_default_for!(NodeSelector);
#[uniffi::export]
impl NodeSelector {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let network = Database::global().global_config.selected_network();
        let selected_node = Database::global().global_config.selected_node();

        let node_list = node_list(network);
        let (node_selection_list, selected_node) = if node_list.contains(&selected_node) {
            let mut node_selection_list = node_list
                .into_iter()
                .map(NodeSelection::Preset)
                .collect::<Vec<NodeSelection>>();

            node_selection_list.push(NodeSelection::Custom(selected_node.clone()));
            (node_selection_list, NodeSelection::Custom(selected_node))
        } else {
            let node_selection_list = node_list.into_iter().map(NodeSelection::Preset).collect();
            (node_selection_list, NodeSelection::Preset(selected_node))
        };

        Self {
            selected_node,
            node_list: node_selection_list,
        }
    }

    #[uniffi::method]
    pub fn node_list(&self) -> Vec<NodeSelection> {
        self.node_list.clone()
    }

    #[uniffi::method]
    pub fn selected_node(&self) -> NodeSelection {
        self.selected_node.clone()
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

#[uniffi::export]
pub fn node_selection_to_node(node: NodeSelection) -> Node {
    node.into()
}

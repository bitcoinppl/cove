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
}

impl_default_for!(NodeSelector);

#[uniffi::export]
impl NodeSelector {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let network = Database::global().global_config.selected_network();

        Self { network }
    }

    #[uniffi::method]
    pub fn node_list(&self) -> Vec<Node> {
        match self.network {
            Network::Bitcoin => {
                let mut nodes = BITCOIN_ELECTRUM
                    .iter()
                    .map(|(name, url)| {
                        Node::new_electrum(name.to_string(), url.to_string(), self.network)
                    })
                    .collect::<Vec<Node>>();

                let (name, url) = BITCOIN_ESPLORA;
                nodes.push(Node::new_esplora(
                    name.to_string(),
                    url.to_string(),
                    self.network,
                ));

                nodes
            }

            Network::Testnet => {
                let (name, url) = TESTNET_ESPLORA;
                vec![Node::new_esplora(
                    name.to_string(),
                    url.to_string(),
                    self.network,
                )]
            }
        }
    }
}

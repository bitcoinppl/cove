use tracing::{error, info, warn};
use url::Url;

use cove_util::ResultExt as _;

use crate::{
    database::{Database, global_flag::GlobalFlagKey},
    network::Network,
    node::{
        ApiType, Node,
        client::{NodeClient, NodeClientOptions},
    },
};
use cove_macros::impl_default_for;
use eyre::{Context, eyre};

pub const BITCOIN_ESPLORA: [(&str, &str); 2] = [
    ("blockstream.info", "https://blockstream.info/api/"),
    ("mempool.space", "https://mempool.space/api/"),
];

pub const BITCOIN_ELECTRUM: [(&str, &str); 3] = [
    ("electrum.blockstream.info", "ssl://electrum.blockstream.info:50002"),
    ("mempool.space electrum", "ssl://mempool.space:50002"),
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
        check_node_with_tor_inference(&node)
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
        let hinted_api_type = if node_type.contains("electrum") {
            ApiType::Electrum
        } else if node_type.contains("esplora") {
            ApiType::Esplora
        } else {
            error!("invalid node type: {node_type}");
            return Err(Error::ParseNodeUrlError("invalid node type".to_string()));
        };
        let inferred_api_type = infer_api_type_from_url_hint(&url);
        let api_type = match inferred_api_type {
            Some(inferred) if inferred != hinted_api_type => {
                warn!(
                    requested_type = ?hinted_api_type,
                    inferred_type = ?inferred,
                    url = %url,
                    "custom node type mismatched url; using type inferred from url",
                );
                inferred
            }
            Some(inferred) => inferred,
            None => hinted_api_type,
        };

        let url = parse_node_url(&url, api_type).map_err_str(Error::ParseNodeUrlError)?;

        if !url.domain().unwrap_or_default().contains('.') {
            return Err(Error::ParseNodeUrlError("invalid url, no domain".to_string()));
        }

        let url_string = url.to_string();

        let name = if entered_name.is_empty() {
            url.domain().unwrap_or(url_string.as_str()).to_string()
        } else {
            entered_name
        };

        let node = match api_type {
            ApiType::Electrum => Node::new_electrum(name, url_string, self.network),
            ApiType::Esplora => Node::new_esplora(name, url_string, self.network),
            ApiType::Rpc => Node::default(self.network),
        };

        Ok(node)
    }

    #[uniffi::method]
    /// Check the node url and set it as selected node if it is valid
    pub async fn check_and_save_node(&self, node: Node) -> Result<(), Error> {
        check_node_with_tor_inference(&node).await.map_err(|error| {
            tracing::warn!("error checking node: {error:?}");
            Error::NodeAccessError(error.to_string())
        })?;

        let database = Database::global();

        if node_implies_tor(&node) {
            database
                .global_flag
                .set_bool_config(GlobalFlagKey::TorSettingsDiscovered, true)
                .map_err(|error| Error::SetSelectedNodeError(error.to_string()))?;
            database
                .global_config
                .set_use_tor(true)
                .map_err(|error| Error::SetSelectedNodeError(error.to_string()))?;
        }

        database
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

fn node_implies_tor(node: &Node) -> bool {
    Url::parse(&node.url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .is_some_and(|host| host.ends_with(".onion"))
}

async fn check_node_with_tor_inference(node: &Node) -> Result<(), crate::node::Error> {
    let inferred_tor = node_implies_tor(node);
    info!(node = %node.url, api_type = ?node.api_type, inferred_tor, "checking node with tor inference");

    let db = Database::global();
    let config = db.global_config();

    if !inferred_tor {
        if config.use_tor() {
            info!(node = %node.url, "node does not imply tor, but global tor is enabled; checking through configured tor client");
            let client = NodeClient::new(node).await?;
            client.check_url().await?;
            return Ok(());
        }

        info!(node = %node.url, "node does not imply tor and global tor is disabled; checking directly");
        return node.check_url().await;
    }

    let tor_external_host = config
        .tor_external_host()
        .ok()
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let batch_size = match node.api_type {
        ApiType::Electrum => 10,
        ApiType::Esplora | ApiType::Rpc => 1,
    };

    let options = NodeClientOptions {
        batch_size,
        use_tor: true,
        tor_mode: config.tor_mode().unwrap_or_default(),
        tor_external_host,
        tor_external_port: config.tor_external_port(),
    };

    info!(node = %node.url, options = ?options, "node implies tor; building tor-enabled node client");

    let client = NodeClient::new_with_options(node, options).await?;
    info!(node = %node.url, "running node check through tor-capable client");
    client.check_url().await?;

    Ok(())
}

fn parse_node_url(url: &str, api_type: ApiType) -> eyre::Result<Url> {
    match api_type {
        ApiType::Electrum => parse_electrum_url(url),
        ApiType::Esplora | ApiType::Rpc => parse_http_url(url),
    }
}

fn infer_api_type_from_url_hint(url: &str) -> Option<ApiType> {
    let lowered = url.trim().to_ascii_lowercase();

    if lowered.is_empty() {
        return None;
    }

    if lowered.starts_with("tcp://") || lowered.starts_with("ssl://") {
        return Some(ApiType::Electrum);
    }

    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        return Some(ApiType::Esplora);
    }

    if lowered.contains("://") {
        return None;
    }

    if lowered.contains('/') {
        return Some(ApiType::Esplora);
    }

    match lowered.rsplit_once(':') {
        Some((_, "50001" | "50002")) => Some(ApiType::Electrum),
        _ => None,
    }
}

fn parse_electrum_url(url: &str) -> eyre::Result<Url> {
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

    if !matches!(url.scheme(), "ssl" | "tcp") {
        return Err(eyre!(
            "invalid electrum url scheme `{}`; expected tcp:// or ssl://",
            url.scheme(),
        ));
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

fn parse_http_url(url: &str) -> eyre::Result<Url> {
    let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };

    let mut url = Url::parse(&url)?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(eyre!(
            "invalid esplora url scheme `{}`; expected http:// or https://",
            url.scheme(),
        ));
    }

    if url.port().is_none() {
        let default_port = if url.scheme() == "http" { 80 } else { 443 };
        url.set_port(Some(default_port)).map_err(|()| eyre!("can't set port"))?;
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

    let (name, url) = match network {
        Network::Bitcoin => BITCOIN_ESPLORA[0],
        Network::Testnet => TESTNET_ESPLORA[0],
        Network::Signet => SIGNET_ESPLORA[0],
        Network::Testnet4 => TESTNET4_ESPLORA[0],
    };

    NodeSelection::Preset(Node::new_esplora(name.to_string(), url.to_string(), network))
}

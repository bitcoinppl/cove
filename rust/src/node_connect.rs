use tracing::{debug, error};
use url::Url;

use crate::{
    database::{Database, global_flag::GlobalFlagKey},
    manager::tor_manager::{TOR_MANAGER, TorManagerAction},
    network::Network,
    node::{
        ApiType, Node,
        client::{NodeClient, NodeClientOptions},
    },
};
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
    /// No preset or selected custom node has the requested name
    #[error("node with name {0} not found")]
    NodeNotFound(String),

    /// The selected node could not be persisted
    #[error("unable to set selected node: {0}")]
    SetSelectedNodeError(String),

    /// The node could not be reached
    #[error("unable to access node: {0}")]
    NodeAccessError(String),

    /// The node URL is invalid
    #[error("unable to parse node url: {0}")]
    ParseNodeUrlError(String),

    /// The node is an onion service but Tor is disabled
    #[error("the selected node requires Tor")]
    OnionNodeRequiresTor,

    /// The selected API type contradicts the URL's transport scheme
    #[error("the selected node type does not match the URL scheme")]
    ApiTypeMismatch { selected: ApiType, inferred: ApiType },

    /// Tor could not be enabled before saving an onion node
    #[error("unable to enable Tor for the selected node")]
    TorEnableError,

    /// The progressive-discovery flag could not be persisted
    #[error("unable to save that Tor settings were discovered")]
    TorSettingsDiscoveryError,
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
        check_node_with_tor_inference(&node).await?;

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
        let selected_api_type = if node_type.contains("electrum") {
            ApiType::Electrum
        } else if node_type.contains("esplora") {
            ApiType::Esplora
        } else {
            error!(node_type, "invalid node type");
            return Err(Error::ParseNodeUrlError("invalid node type".to_string()));
        };

        if let Some(inferred_api_type) = infer_api_type_from_url_hint(&url)
            && inferred_api_type != selected_api_type
        {
            debug!(
                selected_api_type = ?selected_api_type,
                inferred_api_type = ?inferred_api_type,
                host = "<redacted>",
                "custom node type does not match URL scheme",
            );

            return Err(Error::ApiTypeMismatch {
                selected: selected_api_type,
                inferred: inferred_api_type,
            });
        }

        let url = parse_node_url(&url, selected_api_type)
            .map_err(|error| Error::ParseNodeUrlError(error.to_string()))?;

        if !url.domain().unwrap_or_default().contains('.') {
            return Err(Error::ParseNodeUrlError("invalid url, no domain".to_string()));
        }

        let url_string = url.to_string();

        let name = if entered_name.is_empty() {
            url.domain().unwrap_or(url_string.as_str()).to_string()
        } else {
            entered_name
        };

        let node = match selected_api_type {
            ApiType::Electrum => Node::new_electrum(name, url_string, self.network),
            ApiType::Esplora => Node::new_esplora(name, url_string, self.network),
            ApiType::Rpc => Node::default(self.network),
        };

        Ok(node)
    }

    #[uniffi::method]
    /// Check the node url and set it as selected node if it is valid
    pub async fn check_and_save_node(&self, node: Node) -> Result<(), Error> {
        enable_tor_for_onion_save(&node).await?;
        check_node_with_tor_inference(&node).await?;

        let database = Database::global();
        if node_implies_tor(&node) {
            database
                .global_flag
                .set_bool_config(GlobalFlagKey::TorSettingsDiscovered, true)
                .map_err(|error| {
                    debug!(error = ?error, "failed to persist Tor discovery flag");
                    Error::TorSettingsDiscoveryError
                })?;
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

/// Returns whether a node address requires onion routing
pub(crate) fn node_implies_tor(node: &Node) -> bool {
    Url::parse(&node.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .or_else(|| {
            Url::parse(&format!("tcp://{}", node.url.trim()))
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
        })
        .is_some_and(|host| host.to_ascii_lowercase().ends_with(".onion"))
}

/// Switches the selected node to the first clearnet preset for the active network
pub(crate) fn switch_to_first_clearnet_preset() -> Result<Node, ClearnetPresetError> {
    let database = Database::global();
    let network = database.global_config.selected_network();
    let node = node_list(network)
        .into_iter()
        .find(|node| !node_implies_tor(node))
        .ok_or(ClearnetPresetError::NotFound(network))?;

    database.global_config.set_selected_node(&node)?;

    Ok(node)
}

/// Error produced while selecting a clearnet fallback node
#[derive(Debug, thiserror::Error)]
pub(crate) enum ClearnetPresetError {
    /// No clearnet preset exists for the active network
    #[error("no clearnet preset exists for {0}")]
    NotFound(Network),

    /// The selected preset could not be persisted
    #[error("failed to save the clearnet preset: {0}")]
    Database(#[from] crate::database::Error),
}

async fn enable_tor_for_onion_save(node: &Node) -> Result<(), Error> {
    if !node_implies_tor(node) {
        return Ok(());
    }

    TOR_MANAGER.dispatch(TorManagerAction::Enable).await.map_err(|error| {
        debug!(error = ?error, host = "<redacted-onion-host>", "failed to enable Tor for onion node");
        Error::TorEnableError
    })?;

    Ok(())
}

async fn check_node_with_tor_inference(node: &Node) -> Result<(), Error> {
    let redacted_host =
        if node_implies_tor(node) { "<redacted-onion-host>" } else { "<redacted-host>" };
    let options = NodeClientOptions::from_db_for_node(node).map_err(|error| {
        debug!(?error, host = redacted_host, "failed to read Tor settings");
        Error::NodeAccessError("connection failed".to_string())
    })?;
    let options = check_options_for_node(node, options)?;

    debug!(
        api_type = ?node.api_type,
        host = redacted_host,
        through_tor = options.tor.is_some(),
        "checking node",
    );

    let client = NodeClient::new_with_options(node, options).await.map_err(|_| {
        debug!(api_type = ?node.api_type, host = redacted_host, "failed to create node client");
        Error::NodeAccessError("connection failed".to_string())
    })?;
    client.check_url().await.map_err(|_| {
        debug!(api_type = ?node.api_type, host = redacted_host, "node check failed");
        Error::NodeAccessError("connection failed".to_string())
    })?;

    Ok(())
}

fn check_options_for_node(
    node: &Node,
    options: NodeClientOptions,
) -> Result<NodeClientOptions, Error> {
    if node_implies_tor(node) && options.tor.is_none() {
        return Err(Error::OnionNodeRequiresTor);
    }

    Ok(options)
}

fn parse_node_url(url: &str, api_type: ApiType) -> eyre::Result<Url> {
    match api_type {
        ApiType::Electrum => parse_electrum_url(url),
        ApiType::Esplora | ApiType::Rpc => parse_esplora_url(url),
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

fn parse_esplora_url(url: &str) -> eyre::Result<Url> {
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
    use crate::node::client::TorProxy;

    fn node(url: &str) -> Node {
        Node::new_electrum("test".to_string(), url.to_string(), Network::Bitcoin)
    }

    #[test]
    fn onion_detection_accepts_schemed_and_bare_hosts() {
        assert!(node_implies_tor(&node("ssl://example.onion:50002")));
        assert!(node_implies_tor(&node("https://example.onion/api")));
        assert!(node_implies_tor(&node("custom://example.onion/path")));
        assert!(node_implies_tor(&node("EXAMPLE.ONION:50001")));
        assert!(!node_implies_tor(&node("ssl://example.com:50002")));
    }

    #[test]
    fn electrum_parser_accepts_onion_hosts_and_normalizes_transport() {
        let parsed = parse_electrum_url("example.onion:50002").unwrap();

        assert_eq!(parsed.scheme(), "ssl");
        assert_eq!(parsed.host_str(), Some("example.onion"));
        assert_eq!(parsed.port(), Some(50002));

        let parsed = parse_electrum_url("http://example.onion:50001").unwrap();
        assert_eq!(parsed.scheme(), "tcp");
    }

    #[test]
    fn electrum_parser_rejects_unsupported_schemes() {
        let error = parse_electrum_url("ftp://example.onion:50002").unwrap_err();

        assert!(error.to_string().contains("invalid electrum url scheme"));
    }

    #[test]
    fn esplora_parser_preserves_http_schemes_and_onion_hosts() {
        let https = parse_esplora_url("https://example.onion/api").unwrap();
        assert_eq!(https.scheme(), "https");
        assert_eq!(https.host_str(), Some("example.onion"));
        assert_eq!(https.path(), "/api");

        let http = parse_esplora_url("http://example.com/api").unwrap();
        assert_eq!(http.scheme(), "http");
    }

    #[test]
    fn esplora_parser_rejects_electrum_schemes() {
        let error = parse_esplora_url("ssl://example.onion:50002").unwrap_err();

        assert!(error.to_string().contains("invalid esplora url scheme"));
    }

    #[test]
    fn parse_custom_node_returns_typed_api_mismatch() {
        let selector = NodeSelector { network: Network::Bitcoin, node_list: Vec::new() };

        assert_eq!(
            selector.parse_custom_node(
                "https://example.com/api".to_string(),
                "Electrum".to_string(),
                String::new(),
            ),
            Err(Error::ApiTypeMismatch { selected: ApiType::Electrum, inferred: ApiType::Esplora })
        );
        assert_eq!(
            selector.parse_custom_node(
                "ssl://example.com:50002".to_string(),
                "Esplora".to_string(),
                String::new(),
            ),
            Err(Error::ApiTypeMismatch { selected: ApiType::Esplora, inferred: ApiType::Electrum })
        );
    }

    #[test]
    fn parse_custom_esplora_node_preserves_https() {
        let selector = NodeSelector { network: Network::Bitcoin, node_list: Vec::new() };
        let node = selector
            .parse_custom_node(
                "https://example.onion/api".to_string(),
                "Esplora".to_string(),
                String::new(),
            )
            .unwrap();

        assert_eq!(node.api_type, ApiType::Esplora);
        assert!(node.url.starts_with("https://"));
    }

    #[test]
    fn onion_check_requires_a_tor_route() {
        let onion = node("ssl://example.onion:50002");
        let direct = NodeClientOptions { batch_size: 10, tor: None };

        assert_eq!(check_options_for_node(&onion, direct), Err(Error::OnionNodeRequiresTor));

        let routed = NodeClientOptions { batch_size: 10, tor: Some(TorProxy::BuiltIn) };
        assert_eq!(check_options_for_node(&onion, routed.clone()), Ok(routed));
    }
}

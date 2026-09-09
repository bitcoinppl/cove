use std::sync::Arc;

use parking_lot::Mutex;
use tracing::error;
use url::Url;

use crate::node::client::electrum::transport;
use crate::node::tls::{self, TlsTrust};
use crate::{
    database::{
        Database,
        global_config::{
            CertificateTrustCache, GlobalConfigTable, GlobalConfigTableError, NodeRuntimeState,
        },
    },
    network::Network,
    node::Node,
};
use cove_macros::impl_default_for;
use cove_util::ResultExt as _;
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
    certificate_trust: CertificateTrustCache,
    selection_order: NodeSelectionOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionRequest(u64);

/// One commit order shared by all selectors for a database
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeSelectionOrder(Arc<Mutex<u64>>);

#[derive(Debug, Clone, uniffi::Enum, PartialEq, Eq, Hash)]
pub enum NodeSelection {
    Preset(Node),
    Custom(Node),
}

type Error = NodeSelectorError;

#[derive(Debug, Clone)]
enum CertificateTrustHydrationError {
    Store(GlobalConfigTableError),
}

impl From<CertificateTrustHydrationError> for Error {
    fn from(error: CertificateTrustHydrationError) -> Self {
        match error {
            CertificateTrustHydrationError::Store(error) => {
                Self::CertificateTrustStoreError(error.to_string())
            }
        }
    }
}

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

    #[error("a newer node selection replaced this request")]
    SelectionSuperseded,

    /// Reports an invalid persisted certificate trust store
    #[error("unable to read the certificate trust store: {0}")]
    CertificateTrustStoreError(String),
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

/// Certificate trust accepted for one canonical endpoint
#[derive(Debug, Clone, uniffi::Record, PartialEq, Eq, Hash)]
pub struct EndpointCertificateTrust {
    /// The endpoint URL that the user trusted
    pub endpoint: String,

    /// How the endpoint's certificate is trusted
    pub tls: TlsTrust,
}

impl_default_for!(NodeSelector);
#[uniffi::export(async_runtime = "tokio")]
impl NodeSelector {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let database = Database::global();
        let network = database.global_config.selected_network();
        let selected_node = database.global_config.stored_selected_node();
        let certificate_trust = database.global_config.certificate_trust_cache();

        let node_selection_list = node_selection_list(network, selected_node);

        Self {
            network,
            node_list: node_selection_list,
            certificate_trust,
            selection_order: database.global_config.node_selection_order(),
        }
    }

    #[uniffi::method]
    pub fn node_list(&self) -> Vec<NodeSelection> {
        self.node_list.clone()
    }

    #[uniffi::method]
    pub fn selected_node(&self) -> NodeRuntimeState {
        Database::global().global_config.selected_node_state_for_network(self.network)
    }

    #[uniffi::method]
    /// Check and select a preset node without changing durable state on failure
    pub async fn select_preset_node(&self, name: String) -> Result<Node, Error> {
        let request = self.begin_selection_request();
        let network = self.network;
        let requested_name = name.clone();
        let (requested_node, global_config) = cove_tokio::unblock::run_blocking(move || {
            let database = Database::global();
            let requested_node =
                requested_preset_node(network, &requested_name, &database.global_config)?;

            Ok::<_, Error>((requested_node, database.global_config.clone()))
        })
        .await?;

        self.select_preset_node_with_config(requested_node, &global_config, request).await
    }

    #[uniffi::method]
    /// Prevent an in-flight node selection from changing durable state
    pub fn cancel_pending_selection(&self) {
        self.begin_selection_request();
    }

    #[uniffi::method]
    /// Check a node's network connection, including its certificate settings
    pub async fn check_node(&self, node: Node) -> Result<(), Error> {
        self.check_node_connection(node).await
    }

    #[uniffi::method(default(certificate_trust = None))]
    /// Use the url and name of the custom node to set it as the selected node
    pub fn parse_custom_node(
        &self,
        url: String,
        name: String,
        entered_name: String,
        certificate_trust: Option<EndpointCertificateTrust>,
    ) -> Result<Node, Error> {
        let node_type = name.to_ascii_lowercase();

        let url = parse_node_url(&url).map_err_str(Error::ParseNodeUrlError)?;

        if !has_usable_host(&url) {
            return Err(Error::ParseNodeUrlError("invalid url, no domain".to_string()));
        }

        let parsed_url_string = strip_trailing_slash(url.to_string());

        let name = if entered_name.is_empty() {
            url.domain().unwrap_or(parsed_url_string.as_str()).to_string()
        } else {
            entered_name
        };

        let trusted_endpoint = certificate_trust
            .as_ref()
            .map(|trust| {
                normalize_certificate_endpoint(&trust.endpoint).map_err(|error| {
                    Error::ParseNodeUrlError(format!("invalid certificate trust endpoint: {error}"))
                })
            })
            .transpose()?;

        let node = if node_type.contains("electrum") {
            let (url_string, session_tls) = if url.scheme() == "ssl" {
                let endpoint = normalize_certificate_endpoint(&parsed_url_string)
                    .map_err_str(Error::ParseNodeUrlError)?;

                let session_tls = certificate_trust
                    .filter(|_| trusted_endpoint.as_deref() == Some(endpoint.as_str()))
                    .map(|trust| trust.tls);

                (endpoint, session_tls)
            } else {
                (parsed_url_string, None)
            };

            let node =
                Node { tls: session_tls, ..Node::new_electrum(name, url_string, self.network) };

            self.hydrate_certificate_trust(node)?
        } else if node_type.contains("esplora") {
            Node::new_esplora(name, parsed_url_string, self.network)
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
        let endpoint =
            normalize_certificate_endpoint(&url).map_err_str(Error::ParseNodeUrlError)?;

        if self.trusted_certificate(&endpoint)?.is_some() {
            return Ok(CertificateDecision::Changed);
        }

        let certificate = self.fetch_node_certificate(endpoint.clone()).await?;

        if self.trusted_certificate(&endpoint)?.is_some() {
            return Ok(CertificateDecision::Changed);
        }

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
                .map_err_str(Error::ReadCertificateError)?;

        let sha256 = tls::fingerprint(&certificate);

        Ok(NodeCertificate { sha256: sha256.to_vec(), display: tls::display_fingerprint(&sha256) })
    }

    #[uniffi::method]
    /// Save a node after its network connection has been checked
    pub async fn save_node(&self, node: Node) -> Result<(), Error> {
        let request = self.begin_selection_request();
        let global_config = Database::global().global_config.clone();

        self.save_node_with_config(node, &global_config, request).await
    }
}

impl NodeSelector {
    fn begin_selection_request(&self) -> SelectionRequest {
        let mut generation = self.selection_order.0.lock();
        *generation = generation.wrapping_add(1);

        SelectionRequest(*generation)
    }

    async fn save_node_with_config(
        &self,
        node: Node,
        global_config: &GlobalConfigTable,
        request: SelectionRequest,
    ) -> Result<(), Error> {
        let selector = self.clone();
        let config = global_config.clone();

        cove_tokio::unblock::run_blocking(move || {
            let requested_node = selector.requested_custom_node(&node);

            selector.commit_checked_node(&requested_node, &node, &config, request).map(|_| ())
        })
        .await
    }

    async fn select_preset_node_with_config(
        &self,
        requested_node: Node,
        global_config: &GlobalConfigTable,
        request: SelectionRequest,
    ) -> Result<Node, Error> {
        let selector = self.clone();
        let requested_node_for_prepare = requested_node.clone();
        let node = cove_tokio::unblock::run_blocking(move || {
            selector.prepare_preset_node(requested_node_for_prepare)
        })
        .await?;

        node.check_url().await.map_err_debug(Error::NodeAccessError)?;

        let selector = self.clone();
        let config = global_config.clone();
        cove_tokio::unblock::run_blocking(move || {
            selector.commit_checked_node(&requested_node, &node, &config, request)
        })
        .await
    }

    fn prepare_preset_node(&self, requested_node: Node) -> Result<Node, Error> {
        match self.hydrate_certificate_trust(requested_node.clone()) {
            Ok(node) => Ok(node),
            Err(CertificateTrustHydrationError::Store(
                GlobalConfigTableError::CertificateTrustConflict(_),
            )) if requested_node.tls.is_none() => Ok(requested_node),
            Err(error) => Err(error.into()),
        }
    }

    fn requested_custom_node(&self, checked_node: &Node) -> Node {
        let unpinned_node = Node { tls: None, ..checked_node.clone() };

        // parsing can hydrate durable trust, but recovery still needs the original unpinned intent
        match self.hydrate_certificate_trust(unpinned_node.clone()) {
            Ok(hydrated_node) if hydrated_node == *checked_node => unpinned_node,
            _ => checked_node.clone(),
        }
    }

    fn commit_checked_node(
        &self,
        requested_node: &Node,
        checked_node: &Node,
        global_config: &GlobalConfigTable,
        request: SelectionRequest,
    ) -> Result<Node, Error> {
        // keep request creation and the durable commit in one serial order so
        // a newer request cannot appear between the identity check and write
        let generation = self.selection_order.0.lock();
        if *generation != request.0 {
            return Err(Error::SelectionSuperseded);
        }

        if requested_node.tls.is_none()
            && let Some(endpoint) = global_config.conflicted_selected_node_endpoint(self.network)
        {
            return global_config
                .recover_selected_node_from_certificate_trust_conflict(
                    requested_node,
                    checked_node,
                    &endpoint,
                )
                .map_err_str(NodeSelectorError::SetSelectedNodeError);
        }

        global_config
            .set_selected_node(checked_node)
            .map_err_str(NodeSelectorError::SetSelectedNodeError)?;

        Ok(checked_node.clone())
    }

    #[cfg(test)]
    pub(crate) fn with_global_config(network: Network, global_config: &GlobalConfigTable) -> Self {
        Self {
            network,
            node_list: node_selection_list(network, Node::default(network)),
            certificate_trust: global_config.certificate_trust_cache(),
            selection_order: global_config.node_selection_order(),
        }
    }

    async fn check_node_connection(&self, node: Node) -> Result<(), Error> {
        node.check_url().await.map_err(|error| {
            tracing::warn!("error checking node: {error:?}");

            // Distinguished so the caller can offer to trust the certificate
            // instead of showing a generic failure.
            if error.is_certificate_error() {
                return Error::CertificateNotTrusted;
            }

            Error::NodeAccessError(error.to_string())
        })?;

        Ok(())
    }
}

fn requested_preset_node(
    network: Network,
    name: &str,
    global_config: &GlobalConfigTable,
) -> Result<Node, Error> {
    node_list(network)
        .into_iter()
        .find(|node| node.name == name)
        .or_else(|| {
            let selected_node = global_config.stored_selected_node_for_network(network);
            (selected_node.name == name).then_some(selected_node)
        })
        .ok_or_else(|| {
            error!("node with name {name} not found");
            NodeSelectorError::NodeNotFound(name.to_string())
        })
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

fn node_selection_list(network: Network, selected_node: Node) -> Vec<NodeSelection> {
    let presets = node_list(network);

    if presets.iter().any(|preset| same_preset_identity(preset, &selected_node)) {
        return presets.into_iter().map(NodeSelection::Preset).collect();
    }

    let mut selections = presets.into_iter().map(NodeSelection::Preset).collect::<Vec<_>>();
    selections.push(NodeSelection::Custom(selected_node));
    selections
}

pub(crate) fn selected_node_selection(network: Network, selected_node: Node) -> NodeSelection {
    if node_list(network).iter().any(|preset| same_preset_identity(preset, &selected_node)) {
        NodeSelection::Preset(selected_node)
    } else {
        NodeSelection::Custom(selected_node)
    }
}

fn same_preset_identity(preset: &Node, candidate: &Node) -> bool {
    preset.name == candidate.name
        && preset.network == candidate.network
        && preset.api_type == candidate.api_type
        && normalized_preset_url(&preset.url) == normalized_preset_url(&candidate.url)
}

fn normalized_preset_url(url: &str) -> Option<String> {
    normalize_node_url(url).ok()
}

impl NodeSelector {
    fn hydrate_certificate_trust(
        &self,
        node: Node,
    ) -> std::result::Result<Node, CertificateTrustHydrationError> {
        if node.tls.is_some()
            || node.api_type != crate::node::ApiType::Electrum
            || !is_ssl_electrum_endpoint(&node.url)
        {
            return Ok(node);
        }

        let endpoint = normalize_certificate_endpoint(&node.url).map_err(|error| {
            CertificateTrustHydrationError::Store(
                GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string()),
            )
        })?;
        let trust = {
            let certificate_trust = self.certificate_trust.read();

            match certificate_trust.as_ref() {
                Ok(snapshot) => snapshot
                    .trust_for_endpoint(&endpoint)
                    .map_err(CertificateTrustHydrationError::Store)?,
                Err(error) => return Err(CertificateTrustHydrationError::Store(error.clone())),
            }
        };

        Ok(match trust {
            Some(tls) => Node { tls: Some(tls), ..node },
            None => node,
        })
    }

    fn trusted_certificate(&self, url: &str) -> Result<Option<TlsTrust>, Error> {
        let certificate_trust = self.certificate_trust.read();

        match certificate_trust.as_ref() {
            Ok(snapshot) => snapshot
                .trust_for_endpoint(url)
                .map_err(|error| Error::CertificateTrustStoreError(error.to_string())),
            Err(error) => Err(Error::CertificateTrustStoreError(error.to_string())),
        }
    }
}

fn normalized_url(url: &str) -> Result<String, Error> {
    normalize_node_url(url).map_err_str(Error::ParseNodeUrlError)
}

/// Returns the normalized URL used in the node configuration
pub(crate) fn normalize_node_url(url: &str) -> eyre::Result<String> {
    let url = parse_node_url(url)?;

    Ok(strip_trailing_slash(url.to_string()))
}

/// Returns the canonical SSL Electrum endpoint used as a certificate identity
pub(crate) fn normalize_certificate_endpoint(url: &str) -> eyre::Result<String> {
    let url = parse_node_url(url)?;

    if url.scheme() != "ssl" {
        return Err(eyre!("certificate trust requires an ssl:// url"));
    }

    if !has_usable_host(&url) {
        return Err(eyre!("certificate trust requires a usable host"));
    }

    let host = match url.host().ok_or_else(|| eyre!("certificate trust requires a host"))? {
        url::Host::Domain(domain) => domain.to_ascii_lowercase(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => format!("[{address}]"),
    };
    let port = url.port().unwrap_or(50002);

    Ok(format!("ssl://{host}:{port}"))
}

pub(crate) fn is_ssl_electrum_endpoint(url: &str) -> bool {
    normalize_certificate_endpoint(url).is_ok()
}

fn strip_trailing_slash(url: String) -> String {
    url.strip_suffix('/').map(str::to_string).unwrap_or(url)
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
    use std::sync::Arc;

    use parking_lot::RwLock;

    use crate::database::global_config::{
        CertificateTrustSnapshot, CertificateTrustStore, GlobalConfigTable,
    };
    use crate::node::client::electrum::test_server::{TestServer, setup};

    use super::*;

    fn selector() -> NodeSelector {
        selector_with_trust(Ok(CertificateTrustSnapshot::default()))
    }

    fn selector_with_trust(
        certificate_trust: std::result::Result<CertificateTrustSnapshot, GlobalConfigTableError>,
    ) -> NodeSelector {
        NodeSelector {
            network: Network::Bitcoin,
            node_list: Vec::new(),
            certificate_trust: Arc::new(RwLock::new(certificate_trust)),
            selection_order: NodeSelectionOrder::default(),
        }
    }

    fn pinned_store(endpoint: &str) -> CertificateTrustSnapshot {
        let mut store = crate::database::global_config::CertificateTrustStore::default();
        store
            .insert_or_match(
                endpoint.to_string(),
                TlsTrust::PinnedFingerprint { sha256: vec![7; 32] },
            )
            .unwrap();
        CertificateTrustSnapshot::from_store(store)
    }

    fn pinned_store_with_conflict(
        retained_endpoint: &str,
        conflicted_endpoint: &str,
    ) -> CertificateTrustSnapshot {
        let mut store = crate::database::global_config::CertificateTrustStore::default();
        store
            .insert_or_match(
                retained_endpoint.to_string(),
                TlsTrust::PinnedFingerprint { sha256: vec![8; 32] },
            )
            .unwrap();

        CertificateTrustSnapshot::with_store_and_conflicted_endpoint(
            store,
            conflicted_endpoint.to_string(),
        )
    }

    fn endpoint_trust(endpoint: &str) -> EndpointCertificateTrust {
        EndpointCertificateTrust {
            endpoint: endpoint.to_string(),
            tls: TlsTrust::PinnedFingerprint { sha256: vec![3; 32] },
        }
    }

    /// A trailing slash is the same server, so it must use the same canonical
    /// endpoint in the trust store
    #[test]
    fn a_trailing_slash_is_normalized_away() {
        assert_eq!(
            normalize_node_url("ssl://node.example.com:50002/").unwrap(),
            "ssl://node.example.com:50002"
        );
    }

    #[test]
    fn transport_equivalent_urls_share_one_certificate_identity() {
        let canonical = normalize_certificate_endpoint("ssl://node.example.com:50002").unwrap();

        for equivalent in [
            "ssl://node.example.com:50002/ignored/path?query=value#fragment",
            "ssl://user:password@node.example.com:50002",
            "ssl://NODE.EXAMPLE.COM:50002",
            "ssl://node.example.com",
            "https://node.example.com:50002/ignored",
        ] {
            assert_eq!(normalize_certificate_endpoint(equivalent).unwrap(), canonical);
        }
    }

    #[test]
    fn certificate_identity_preserves_transport_host_and_effective_port() {
        assert_eq!(
            normalize_certificate_endpoint("ssl://[fd00::1]/path").unwrap(),
            "ssl://[fd00::1]:50002"
        );
        assert_ne!(
            normalize_certificate_endpoint("ssl://node.example.com:50001").unwrap(),
            normalize_certificate_endpoint("ssl://node.example.com:50002").unwrap()
        );
    }

    #[test]
    fn custom_nodes_keep_their_certificate_settings() {
        let trust = endpoint_trust("ssl://node.example.com:50002");

        let node = selector()
            .parse_custom_node(
                "ssl://node.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(trust.clone()),
            )
            .unwrap();

        assert_eq!(node.tls, Some(trust.tls));
    }

    #[test]
    fn session_trust_matches_transport_equivalent_endpoints() {
        let node = selector()
            .parse_custom_node(
                "https://NODE.example.com:50002/path?query=value".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://node.example.com:50002/")),
            )
            .unwrap();

        assert_eq!(node.url, "ssl://node.example.com:50002");
        assert_eq!(node.tls, Some(TlsTrust::PinnedFingerprint { sha256: vec![3; 32] }));
    }

    #[test]
    fn untrusted_ssl_electrum_nodes_use_the_canonical_transport_url() {
        let node = selector()
            .parse_custom_node(
                "ssl://user:password@NODE.example.com/ignored/path?query=value#fragment"
                    .to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(node.url, "ssl://node.example.com:50002");
        assert_eq!(node.tls, None);
    }

    #[test]
    fn session_trust_does_not_apply_to_another_endpoint() {
        let node = selector()
            .parse_custom_node(
                "ssl://other.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://node.example.com:50002")),
            )
            .unwrap();

        assert_eq!(node.tls, None);
    }

    #[test]
    fn session_trust_does_not_apply_to_tcp_or_esplora() {
        let tcp = selector()
            .parse_custom_node(
                "tcp://node.example.com:50001".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://node.example.com:50002")),
            )
            .unwrap();
        let esplora = selector()
            .parse_custom_node(
                "https://node.example.com".to_string(),
                "Custom Esplora".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://node.example.com:50002")),
            )
            .unwrap();

        assert_eq!(tcp.tls, None);
        assert_eq!(esplora.tls, None);
    }

    #[test]
    fn invalid_session_trust_endpoint_is_a_parse_error() {
        let error = selector()
            .parse_custom_node(
                "ssl://node.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(endpoint_trust("tcp://node.example.com:50001")),
            )
            .unwrap_err();

        assert!(
            matches!(error, Error::ParseNodeUrlError(message) if message.contains("certificate trust endpoint"))
        );
    }

    #[test]
    fn custom_nodes_hydrate_trust_from_the_selector_snapshot() {
        let node = selector_with_trust(Ok(pinned_store("ssl://node.example.com:50002")))
            .parse_custom_node(
                "ssl://node.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(node.tls, Some(TlsTrust::PinnedFingerprint { sha256: vec![7; 32] }));
    }

    #[test]
    fn custom_nodes_do_not_hydrate_trust_for_another_endpoint() {
        let node = selector_with_trust(Ok(pinned_store("ssl://node.example.com:50002")))
            .parse_custom_node(
                "ssl://other.example.com:50002".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(node.tls, None);
    }

    #[test]
    fn custom_nodes_scope_conflicts_to_their_endpoint() {
        let retained_endpoint = "ssl://retained.example.com:50002";
        let conflicted_endpoint = "ssl://conflicted.example.com:50002";
        let selector = selector_with_trust(Ok(pinned_store_with_conflict(
            retained_endpoint,
            conflicted_endpoint,
        )));

        let retained = selector
            .parse_custom_node(
                retained_endpoint.to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                None,
            )
            .unwrap();
        assert_eq!(retained.tls, Some(TlsTrust::PinnedFingerprint { sha256: vec![8; 32] }));

        let error = selector
            .parse_custom_node(
                conflicted_endpoint.to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            Error::CertificateTrustStoreError(message)
                if message.contains("conflicted.example.com:50002")
        ));
    }

    #[test]
    fn corrupt_trust_snapshot_is_reported_when_hydrating_a_custom_node() {
        let error = selector_with_trust(Err(GlobalConfigTableError::InvalidCertificateTrustStore(
            "corrupt trust store".to_string(),
        )))
        .parse_custom_node(
            "ssl://node.example.com:50002".to_string(),
            "Custom Electrum".to_string(),
            String::new(),
            None,
        )
        .unwrap_err();

        assert!(
            matches!(error, Error::CertificateTrustStoreError(message) if message.contains("corrupt trust store"))
        );
    }

    #[test]
    fn only_a_typed_trust_conflict_is_a_recovery_candidate() {
        let endpoint = "ssl://node.example.com:50002";
        let selector = selector_with_trust(Ok(CertificateTrustSnapshot::with_conflicted_endpoint(
            endpoint.to_string(),
        )));
        let node = Node::new_electrum("Custom Electrum".into(), endpoint.into(), Network::Bitcoin);

        assert!(matches!(
            selector.hydrate_certificate_trust(node),
            Err(CertificateTrustHydrationError::Store(
                GlobalConfigTableError::CertificateTrustConflict(candidate)
            )) if candidate == endpoint
        ));
        assert_eq!(
            selector.certificate_trust.read().as_ref().unwrap().first_conflict().as_deref(),
            Some(endpoint)
        );

        let corrupt = selector_with_trust(Err(
            GlobalConfigTableError::InvalidCertificateTrustStore("corrupt trust store".to_string()),
        ));
        assert_eq!(corrupt.certificate_trust.read().as_ref().ok(), None);
    }

    #[test]
    fn pinned_preset_is_classified_as_a_preset_without_tls_equality() {
        let preset = node_list(Network::Bitcoin)
            .into_iter()
            .find(|node| node.api_type == crate::node::ApiType::Electrum)
            .unwrap();
        let selected = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![7; 32] }),
            ..preset.clone()
        };

        assert!(matches!(
            selected_node_selection(Network::Bitcoin, selected),
            NodeSelection::Preset(_)
        ));

        let renamed = Node { name: "Custom name".to_string(), ..preset };
        assert!(matches!(
            selected_node_selection(Network::Bitcoin, renamed),
            NodeSelection::Custom(_)
        ));
    }

    #[test]
    fn pinned_preset_hydrates_before_selection() {
        let preset = node_list(Network::Bitcoin)
            .into_iter()
            .find(|node| node.api_type == crate::node::ApiType::Electrum)
            .unwrap();
        let expected = TlsTrust::PinnedFingerprint { sha256: vec![7; 32] };
        let selector = selector_with_trust(Ok(pinned_store(&preset.url)));

        let hydrated = selector.hydrate_certificate_trust(preset).unwrap();

        assert_eq!(hydrated.tls, Some(expected));
    }

    #[test]
    fn non_tls_presets_do_not_require_a_trust_snapshot() {
        let selector = selector_with_trust(Err(
            GlobalConfigTableError::InvalidCertificateTrustStore("corrupt trust store".to_string()),
        ));
        let esplora = node_list(Network::Bitcoin)
            .into_iter()
            .find(|node| node.api_type == crate::node::ApiType::Esplora)
            .unwrap();

        assert_eq!(selector.hydrate_certificate_trust(esplora.clone()).unwrap(), esplora);
    }

    #[test]
    fn newer_selector_instance_supersedes_an_older_instances_request() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_global_config();
        let previous = Node::new_esplora(
            "Previous".into(),
            "https://previous.example/api".into(),
            Network::Bitcoin,
        );
        table.set_selected_node(&previous).unwrap();

        let stale_selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let current_selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let stale_request = stale_selector.begin_selection_request();
        let current_request = current_selector.begin_selection_request();
        let stale_node =
            Node::new_esplora("Stale".into(), "https://stale.example/api".into(), Network::Bitcoin);

        let error = stale_selector
            .commit_checked_node(&stale_node, &stale_node, &table, stale_request)
            .unwrap_err();

        assert_eq!(error, Error::SelectionSuperseded);
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), previous);

        let current_node = Node::new_esplora(
            "Current".into(),
            "https://current.example/api".into(),
            Network::Bitcoin,
        );
        current_selector
            .commit_checked_node(&current_node, &current_node, &table, current_request)
            .unwrap();

        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), current_node);
    }

    #[tokio::test]
    async fn failed_preset_validation_leaves_stored_node_and_trust_unchanged() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let previous_trust = TlsTrust::PinnedFingerprint { sha256: vec![9; 32] };
        let previous = Node {
            tls: Some(previous_trust.clone()),
            ..Node::new_electrum(
                "Custom".into(),
                "ssl://previous.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&previous).unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let server = TestServer::self_signed("localhost");
        let candidate = Node::new_electrum(
            "Unavailable preset".into(),
            format!("ssl://127.0.0.1:{}", server.port),
            Network::Bitcoin,
        );

        let request = selector.begin_selection_request();
        let error = selector
            .select_preset_node_with_config(candidate.clone(), &table, request)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::NodeAccessError(_)));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), previous);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &previous.url).unwrap(),
            Some(previous_trust)
        );
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &candidate.url).unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn successful_preset_validation_commits_the_preset() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let previous = Node::new_esplora(
            "Previous".into(),
            "https://previous.example/api".into(),
            Network::Bitcoin,
        );
        table.set_selected_node(&previous).unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let server = TestServer::self_signed("localhost");
        let trust = server.fingerprint_trust();
        let candidate = Node {
            tls: Some(trust.clone()),
            ..Node::new_electrum(
                "Working preset".into(),
                format!("ssl://127.0.0.1:{}", server.port),
                Network::Bitcoin,
            )
        };

        let request = selector.begin_selection_request();
        let selected = selector
            .select_preset_node_with_config(candidate.clone(), &table, request)
            .await
            .unwrap();

        assert_eq!(selected, candidate);
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), candidate);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &selected.url).unwrap(),
            Some(trust)
        );
    }

    #[tokio::test]
    async fn preset_validation_hydrates_durable_trust_before_check_and_commit() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let previous = Node::new_esplora(
            "Previous".into(),
            "https://previous.example/api".into(),
            Network::Bitcoin,
        );
        table.set_selected_node(&previous).unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let server = TestServer::self_signed("localhost");
        let endpoint = format!("ssl://127.0.0.1:{}", server.port);
        let trust = server.fingerprint_trust();
        let mut trust_store = CertificateTrustStore::default();
        trust_store.insert_or_match(endpoint.clone(), trust.clone()).unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&trust_store).unwrap(),
            )
            .unwrap();
        let candidate = Node::new_electrum("Hydrated preset".into(), endpoint, Network::Bitcoin);

        let request = selector.begin_selection_request();
        let selected =
            selector.select_preset_node_with_config(candidate, &table, request).await.unwrap();

        assert_eq!(selected.tls, Some(trust.clone()));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), selected);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &selected.url).unwrap(),
            Some(trust)
        );
    }

    #[tokio::test]
    async fn failed_selector_recovery_keeps_conflict_and_selection_unchanged() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let endpoint = "ssl://shared.example.com:50002";
        let previous = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![2; 32] }),
            ..Node::new_electrum("Bitcoin legacy".into(), endpoint.into(), Network::Bitcoin)
        };
        let other = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![3; 32] }),
            ..Node::new_electrum("Testnet legacy".into(), endpoint.into(), Network::Testnet)
        };
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&previous).unwrap(),
            )
            .unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&other).unwrap(),
            )
            .unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let server = TestServer::self_signed("localhost");
        let replacement = Node::new_electrum(
            "Replacement".into(),
            format!("ssl://127.0.0.1:{}", server.port),
            Network::Bitcoin,
        );

        let request = selector.begin_selection_request();
        let error = selector
            .select_preset_node_with_config(replacement, &table, request)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::NodeAccessError(_)));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), previous);
        assert!(matches!(
            table.certificate_trust_for_url(Network::Bitcoin, endpoint),
            Err(crate::database::Error::GlobalConfig(
                GlobalConfigTableError::CertificateTrustConflict(_)
            ))
        ));
    }

    #[tokio::test]
    async fn successful_selector_recovery_commits_after_validation() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let conflict_endpoint = "ssl://shared.example.com:50002";
        let previous = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![4; 32] }),
            ..Node::new_electrum(
                "Bitcoin legacy".into(),
                conflict_endpoint.into(),
                Network::Bitcoin,
            )
        };
        let other = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![5; 32] }),
            ..Node::new_electrum(
                "Testnet legacy".into(),
                conflict_endpoint.into(),
                Network::Testnet,
            )
        };
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&previous).unwrap(),
            )
            .unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&other).unwrap(),
            )
            .unwrap();

        let server = TestServer::self_signed("localhost");
        let replacement_endpoint = format!("ssl://127.0.0.1:{}", server.port);
        let replacement_trust = server.fingerprint_trust();
        let mut trust_store = CertificateTrustStore::default();
        trust_store
            .insert_or_match(replacement_endpoint.clone(), replacement_trust.clone())
            .unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&trust_store).unwrap(),
            )
            .unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let replacement =
            Node::new_electrum("Replacement".into(), replacement_endpoint, Network::Bitcoin);

        let request = selector.begin_selection_request();
        let selected =
            selector.select_preset_node_with_config(replacement, &table, request).await.unwrap();

        assert_eq!(selected.tls, Some(replacement_trust));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), selected);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, conflict_endpoint).unwrap(),
            Some(TlsTrust::PinnedFingerprint { sha256: vec![5; 32] })
        );
    }

    #[tokio::test]
    async fn checked_custom_replacement_uses_current_conflict_recovery() {
        crate::app::reconcile::test_support::init_noop_updater();
        setup();
        let (_tmp, table) = test_global_config();
        let conflict_endpoint = "ssl://shared.example.com:50002";
        let previous = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![6; 32] }),
            ..Node::new_electrum(
                "Bitcoin legacy".into(),
                conflict_endpoint.into(),
                Network::Bitcoin,
            )
        };
        let other = Node {
            tls: Some(TlsTrust::PinnedFingerprint { sha256: vec![7; 32] }),
            ..Node::new_electrum(
                "Testnet legacy".into(),
                conflict_endpoint.into(),
                Network::Testnet,
            )
        };
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&previous).unwrap(),
            )
            .unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&other).unwrap(),
            )
            .unwrap();

        let server = TestServer::self_signed("localhost");
        let replacement_endpoint = format!("ssl://127.0.0.1:{}", server.port);
        let replacement_trust = server.fingerprint_trust();
        let mut trust_store = CertificateTrustStore::default();
        trust_store
            .insert_or_match(replacement_endpoint.clone(), replacement_trust.clone())
            .unwrap();
        table
            .set(
                crate::database::global_config::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&trust_store).unwrap(),
            )
            .unwrap();

        let selector = NodeSelector::with_global_config(Network::Bitcoin, &table);
        let replacement = selector
            .parse_custom_node(
                replacement_endpoint,
                "Custom Electrum".into(),
                "Replacement".into(),
                None,
            )
            .unwrap();
        selector.check_node_connection(replacement.clone()).await.unwrap();

        let request = selector.begin_selection_request();
        selector.save_node_with_config(replacement.clone(), &table, request).await.unwrap();

        assert_eq!(replacement.tls, Some(replacement_trust));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), replacement);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, conflict_endpoint).unwrap(),
            Some(TlsTrust::PinnedFingerprint { sha256: vec![7; 32] })
        );
    }

    /// Self hosted servers are commonly reached by address rather than by name.
    #[test]
    fn nodes_can_be_reached_by_ip_address() {
        assert_eq!(
            selector()
                .parse_custom_node(
                    "ssl://192.168.1.50:50002".to_string(),
                    "Custom Electrum".to_string(),
                    String::new(),
                    Some(endpoint_trust("ssl://192.168.1.50:50002")),
                )
                .unwrap()
                .url,
            "ssl://192.168.1.50:50002"
        );
        assert_eq!(
            selector()
                .parse_custom_node(
                    "ssl://[fd00::1]:50002".to_string(),
                    "Custom Electrum".to_string(),
                    String::new(),
                    Some(endpoint_trust("ssl://[fd00::1]:50002")),
                )
                .unwrap()
                .url,
            "ssl://[fd00::1]:50002"
        );
    }

    #[test]
    fn esplora_nodes_ignore_certificate_settings() {
        let node = selector()
            .parse_custom_node(
                "https://esplora.example.com".to_string(),
                "Custom Esplora".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://esplora.example.com:50002")),
            )
            .unwrap();

        assert_eq!(node.tls, None);
    }

    #[test]
    fn tcp_nodes_ignore_certificate_settings() {
        let node = selector()
            .parse_custom_node(
                "tcp://node.example.com:50001".to_string(),
                "Custom Electrum".to_string(),
                String::new(),
                Some(endpoint_trust("ssl://node.example.com:50002")),
            )
            .unwrap();

        assert_eq!(node.tls, None);
    }

    #[test]
    fn a_url_without_a_host_is_rejected() {
        let url = parse_node_url("ssl://nodomain:50002").unwrap();
        assert!(!has_usable_host(&url));
    }

    fn test_global_config() -> (tempfile::TempDir, GlobalConfigTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = GlobalConfigTable::new(db, &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }
}

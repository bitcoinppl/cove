use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use parking_lot::{Mutex, RwLock};
use redb::{ReadableTable, TableDefinition};
use tap::TapFallible as _;
use tracing::{error, warn};

use cove_util::ResultExt as _;

use crate::{
    app::reconcile::{Update, Updater},
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
    custom_block_explorer::{
        BlockExplorerOption, CustomBlockExplorerError, CustomBlockExplorerTemplate, PREVIEW_TXID,
        effective_transaction_url,
    },
    fiat::FiatCurrency,
    network::Network,
    node::{ApiType, Node, tls::TlsTrust},
    string_config_accessor,
    wallet::metadata::{WalletId, WalletMode},
};

use super::{Error, error::SerdeError};

pub const TABLE: TableDefinition<&'static str, String> = TableDefinition::new("global_config");

type Result<T, E = Error> = std::result::Result<T, E>;
pub(crate) type CertificateTrustCache =
    Arc<RwLock<std::result::Result<CertificateTrustSnapshot, GlobalConfigTableError>>>;

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum GlobalConfigKey {
    SelectedWalletId,
    SelectedNetwork,
    SelectedFiatCurrency,
    SelectedNode(Network),
    /// Stores certificate trust by normalized Electrum endpoint
    CertificateTrustStore,
    ColorScheme,
    AuthType,
    HashedPinCode,
    WipeDataPin,
    DecoyPin,
    InDecoyMode,
    MainSelectedWalletId,
    DecoySelectedWalletId,
    LockedAt,
    OnboardingProgress,
    CustomBlockExplorer(Network),
}

impl From<GlobalConfigKey> for &'static str {
    fn from(key: GlobalConfigKey) -> Self {
        match key {
            GlobalConfigKey::SelectedWalletId => "selected_wallet_id",
            GlobalConfigKey::SelectedNetwork => "selected_network",
            GlobalConfigKey::SelectedFiatCurrency => "selected_fiat_currency",
            GlobalConfigKey::SelectedNode(Network::Bitcoin) => "selected_node_bitcoin",
            GlobalConfigKey::SelectedNode(Network::Testnet) => "selected_node_testnet",
            GlobalConfigKey::SelectedNode(Network::Testnet4) => "selected_node_testnet4",
            GlobalConfigKey::SelectedNode(Network::Signet) => "selected_node_signet",
            GlobalConfigKey::CertificateTrustStore => "certificate_trust_store",
            GlobalConfigKey::ColorScheme => "color_scheme",
            GlobalConfigKey::AuthType => "auth_type",
            GlobalConfigKey::HashedPinCode => "hashed_pin_code",
            GlobalConfigKey::WipeDataPin => "wipe_data_pin",
            GlobalConfigKey::DecoyPin => "decoy_pin",
            GlobalConfigKey::InDecoyMode => "in_decoy_mode",
            GlobalConfigKey::MainSelectedWalletId => "main_selected_wallet_id",
            GlobalConfigKey::DecoySelectedWalletId => "decoy_selected_wallet_id",
            GlobalConfigKey::LockedAt => "locked_at",
            GlobalConfigKey::OnboardingProgress => "onboarding_progress",
            GlobalConfigKey::CustomBlockExplorer(Network::Bitcoin) => {
                "custom_block_explorer_bitcoin"
            }
            GlobalConfigKey::CustomBlockExplorer(Network::Testnet) => {
                "custom_block_explorer_testnet"
            }
            GlobalConfigKey::CustomBlockExplorer(Network::Testnet4) => {
                "custom_block_explorer_testnet4"
            }
            GlobalConfigKey::CustomBlockExplorer(Network::Signet) => "custom_block_explorer_signet",
        }
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct GlobalConfigTable {
    db: Arc<redb::Database>,
    certificate_trust: CertificateTrustCache,
    certificate_trust_writer: Arc<Mutex<()>>,
}

impl GlobalConfigTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        let certificate_trust = {
            let table = write_txn.open_table(TABLE).expect("failed to create table");
            initialize_certificate_trust_cache(&table)
        };

        Self {
            db,
            certificate_trust: Arc::new(RwLock::new(certificate_trust)),
            certificate_trust_writer: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn certificate_trust_cache(&self) -> CertificateTrustCache {
        Arc::clone(&self.certificate_trust)
    }

    fn replace_certificate_trust_cache(
        &self,
        certificate_trust: std::result::Result<CertificateTrustSnapshot, GlobalConfigTableError>,
    ) {
        *self.certificate_trust.write() = certificate_trust;
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum GlobalConfigTableError {
    #[error("failed to save global config: {0}")]
    Save(String),

    #[error("failed to get global config: {0}")]
    Read(String),

    #[error("pin code must be hashed before saving")]
    PinCodeMustBeHashed,

    #[error("invalid custom block explorer: {0}")]
    InvalidCustomBlockExplorer(String),

    /// Reports malformed or invalid persisted certificate trust data
    #[error("invalid certificate trust store: {0}")]
    InvalidCertificateTrustStore(String),

    /// Reports an attempt to replace a remembered certificate with another one
    #[error("certificate trust conflicts for endpoint {0}")]
    CertificateTrustConflict(String),
}

/// Reports incoming certificate trust entries that matched an existing endpoint
/// with a different trust value
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CertificateTrustRestoreReport {
    pub(crate) conflicting_endpoints: Vec<String>,
}

/// Remembered certificate trust keyed by canonical SSL Electrum endpoint
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CertificateTrustStore(BTreeMap<String, TlsTrust>);

/// Effective trust loaded from durable entries and selected-node legacy pins
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CertificateTrustSnapshot {
    store: CertificateTrustStore,
    conflicted_endpoints: BTreeSet<String>,
}

impl CertificateTrustSnapshot {
    pub(crate) fn from_store(store: CertificateTrustStore) -> Self {
        Self { store, conflicted_endpoints: BTreeSet::new() }
    }

    pub(crate) fn trust_for_endpoint(
        &self,
        endpoint: &str,
    ) -> std::result::Result<Option<TlsTrust>, GlobalConfigTableError> {
        if self.conflicted_endpoints.contains(endpoint) {
            return Err(GlobalConfigTableError::CertificateTrustConflict(endpoint.to_string()));
        }

        Ok(self.store.get(endpoint))
    }

    pub(crate) fn ensure_no_conflicts(&self) -> std::result::Result<(), GlobalConfigTableError> {
        if let Some(endpoint) = self.conflicted_endpoints.first() {
            return Err(GlobalConfigTableError::CertificateTrustConflict(endpoint.clone()));
        }

        Ok(())
    }

    fn is_conflicted(&self, endpoint: &str) -> bool {
        self.conflicted_endpoints.contains(endpoint)
    }

    #[cfg(test)]
    pub(crate) fn first_conflict(&self) -> Option<String> {
        self.conflicted_endpoints.first().cloned()
    }

    #[cfg(test)]
    pub(crate) fn with_conflicted_endpoint(endpoint: String) -> Self {
        Self {
            store: CertificateTrustStore::default(),
            conflicted_endpoints: [endpoint].into_iter().collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_store_and_conflicted_endpoint(
        store: CertificateTrustStore,
        endpoint: String,
    ) -> Self {
        let mut snapshot = Self::from_store(store);
        snapshot.conflicted_endpoints.insert(endpoint);
        snapshot
    }

    fn remember_node_certificate_trust(
        &mut self,
        node: &Node,
    ) -> std::result::Result<(), GlobalConfigTableError> {
        let Some(trust) = node.tls.clone() else {
            return Ok(());
        };

        let Some(endpoint) = endpoint_for_node(node).map_err(|error| match error {
            Error::GlobalConfig(error) => error,
            error => GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string()),
        })?
        else {
            return Ok(());
        };

        match self.store.insert_or_match(endpoint, trust) {
            Ok(()) => {}
            Err(GlobalConfigTableError::CertificateTrustConflict(endpoint)) => {
                // retain the first valid value for unrelated reads, but fail closed for this endpoint
                self.conflicted_endpoints.insert(endpoint);
            }
            Err(error) => return Err(error),
        }

        Ok(())
    }

    fn merge(
        &mut self,
        other: &CertificateTrustStore,
    ) -> std::result::Result<CertificateTrustRestoreReport, GlobalConfigTableError> {
        self.store.merge(other)
    }
}

impl<'de> serde::Deserialize<'de> for CertificateTrustStore {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let entries = BTreeMap::<String, TlsTrust>::deserialize(deserializer)?;

        Self::try_from_entries(entries).map_err(serde::de::Error::custom)
    }
}

impl CertificateTrustStore {
    fn try_from_entries(entries: BTreeMap<String, TlsTrust>) -> std::result::Result<Self, String> {
        let mut store = Self::default();

        for (endpoint, trust) in entries {
            let canonical = crate::node_connect::normalize_certificate_endpoint(&endpoint)
                .map_err(|error| format!("invalid endpoint {endpoint:?}: {error}"))?;

            store.insert_or_match(canonical, trust).map_err(|error| error.to_string())?;
        }

        Ok(store)
    }

    pub(crate) fn get(&self, endpoint: &str) -> Option<TlsTrust> {
        self.0.get(endpoint).cloned()
    }

    pub(crate) fn insert_or_match(
        &mut self,
        endpoint: String,
        trust: TlsTrust,
    ) -> std::result::Result<(), GlobalConfigTableError> {
        crate::node::tls::client_config(&trust).map_err(|error| {
            GlobalConfigTableError::InvalidCertificateTrustStore(format!(
                "invalid TLS trust: {error}"
            ))
        })?;

        let endpoint =
            crate::node_connect::normalize_certificate_endpoint(&endpoint).map_err(|error| {
                GlobalConfigTableError::InvalidCertificateTrustStore(format!(
                    "invalid endpoint {endpoint:?}: {error}"
                ))
            })?;

        match self.0.get(&endpoint) {
            None => {
                self.0.insert(endpoint, trust);

                Ok(())
            }
            Some(existing) if existing == &trust => Ok(()),
            Some(_) => Err(GlobalConfigTableError::CertificateTrustConflict(endpoint)),
        }
    }

    fn remove_if_matches(&mut self, endpoint: &str, trust: &TlsTrust) -> bool {
        if self.0.get(endpoint) != Some(trust) {
            return false;
        }

        self.0.remove(endpoint).is_some()
    }

    pub(crate) fn merge(
        &mut self,
        other: &Self,
    ) -> std::result::Result<CertificateTrustRestoreReport, GlobalConfigTableError> {
        let mut report = CertificateTrustRestoreReport::default();

        for (endpoint, trust) in &other.0 {
            match self.insert_or_match(endpoint.clone(), trust.clone()) {
                Ok(()) => {}
                Err(GlobalConfigTableError::CertificateTrustConflict(endpoint)) => {
                    report.conflicting_endpoints.push(endpoint);
                }
                Err(error) => return Err(error),
            }
        }

        Ok(report)
    }
}

fn initialize_certificate_trust_cache<T>(
    table: &T,
) -> std::result::Result<CertificateTrustSnapshot, GlobalConfigTableError>
where
    T: ReadableTable<&'static str, String>,
{
    let trust_store_key: &'static str = GlobalConfigKey::CertificateTrustStore.into();
    let trust_store = table
        .get(trust_store_key)
        .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
        .map(|value| value.value())
        .map(|value| serde_json::from_str::<CertificateTrustStore>(&value))
        .transpose()
        .map_err(|error| GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string()))?
        .unwrap_or_default();
    let mut snapshot = CertificateTrustSnapshot::from_store(trust_store);

    for network in [Network::Bitcoin, Network::Testnet, Network::Testnet4, Network::Signet] {
        let selected_node_key: &'static str = GlobalConfigKey::SelectedNode(network).into();
        let Some(node_json) = table
            .get(selected_node_key)
            .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
            .map(|value| value.value())
        else {
            continue;
        };
        let Ok(node) = serde_json::from_str::<Node>(&node_json) else {
            continue;
        };

        snapshot.remember_node_certificate_trust(&node)?;
    }

    Ok(snapshot)
}

impl From<CustomBlockExplorerError> for GlobalConfigTableError {
    fn from(error: CustomBlockExplorerError) -> Self {
        Self::InvalidCustomBlockExplorer(error.to_string())
    }
}

impl GlobalConfigTable {
    string_config_accessor!(
        pub auth_type,
        GlobalConfigKey::AuthType,
        AuthType
    );

    string_config_accessor!(
        pub color_scheme,
        GlobalConfigKey::ColorScheme,
        ColorSchemeSelection,
        Update::ColorSchemeChanged
    );

    string_config_accessor!(
        pub fiat_currency,
        GlobalConfigKey::SelectedFiatCurrency,
        FiatCurrency,
        Update::FiatCurrencyChanged
    );

    string_config_accessor!(pub wipe_data_pin, GlobalConfigKey::WipeDataPin, String);
    string_config_accessor!(pub decoy_pin, GlobalConfigKey::DecoyPin, String);
    string_config_accessor!(priv_hashed_pin_code, GlobalConfigKey::HashedPinCode, String);

    string_config_accessor!(pub locked_at, GlobalConfigKey::LockedAt, u64);

    // string_config_accessor!(
    //     pub auth_type,
    //     GlobalConfigKey::AuthType,
    //     AuthType
    // );
}

impl GlobalConfigTable {
    pub fn set_decoy_mode(&self) -> Result<()> {
        // already in decoy mode, nothing to do
        if self.is_in_decoy_mode() {
            warn!("already in decoy mode");
            return Ok(());
        }

        // currently in main mode, save the selected wallet id as the decoy selected wallet id
        if let Some(id) = self.selected_wallet() {
            let _ = self
                .set(GlobalConfigKey::MainSelectedWalletId, id.to_string())
                .tap_err(|error| error!("unable to set main selected wallet id ({id}): {error}"));
        }

        // get the selected wallet id for decoy mode if it exists and select it
        if let Some(id) = self.get(GlobalConfigKey::DecoySelectedWalletId).ok().flatten() {
            let _ = self
                .select_wallet(id.clone().into())
                .tap_err(|error| error!("unable to select wallet for decoy {id}: {error}"));
        }

        self.set(GlobalConfigKey::InDecoyMode, "true".to_string())?;
        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub fn set_main_mode(&self) -> Result<()> {
        // already in main mode, nothing to do
        if self.is_in_main_mode() {
            warn!("already in main mode");
            return Ok(());
        }

        // currently in decoy mode, save the selected wallet id as the decoy selected wallet id
        if let Some(id) = self.selected_wallet() {
            let _ = self
                .set(GlobalConfigKey::DecoySelectedWalletId, id.to_string())
                .tap_err(|error| error!("unable to set decoy selected wallet id ({id}): {error}"));
        }

        // set the selected wallet id to the one saved if there is one
        if let Some(id) = self.get(GlobalConfigKey::MainSelectedWalletId).ok().flatten() {
            let _ = self
                .select_wallet(id.clone().into())
                .tap_err(|error| error!("unable to select wallet for main {id}: {error}"));
        }

        self.set(GlobalConfigKey::InDecoyMode, "false".to_string())?;
        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub(crate) fn custom_block_explorer_transaction_url(
        &self,
        network: Network,
        txid: String,
    ) -> String {
        let stored_template =
            self.get(GlobalConfigKey::CustomBlockExplorer(network)).ok().flatten();

        effective_transaction_url(network, stored_template.as_deref(), txid)
    }

    /// Returns the configured node for `network`, independent of the globally selected network
    pub(crate) fn selected_node_for_network(&self, network: Network) -> Node {
        let node = self.stored_selected_node_for_network(network);

        match self.node_with_certificate_trust(&node) {
            Ok(node) => node,
            Err(error) => {
                warn!("unable to restore certificate trust for selected node: {error}");

                if node.tls.is_none()
                    && node.api_type == ApiType::Electrum
                    && crate::node_connect::is_ssl_electrum_endpoint(&node.url)
                {
                    warn!("using the safe default node for {network} after trust hydration failed");

                    return Node::default(network);
                }

                node
            }
        }
    }

    fn stored_selected_node_for_network(&self, network: Network) -> Node {
        let selected_node_key = GlobalConfigKey::SelectedNode(network);
        let node_json = self.get(selected_node_key).unwrap_or(None).unwrap_or_default();
        let Ok(node) = serde_json::from_str::<Node>(&node_json) else {
            return Node::default(network);
        };

        if node.network != network {
            warn!("ignoring selected node with network {} stored for {}", node.network, network);

            return Node::default(network);
        }

        node
    }

    /// Returns `node` with trust restored from the endpoint trust store
    pub(crate) fn node_with_certificate_trust(&self, node: &Node) -> Result<Node> {
        if node.tls.is_some() {
            return Ok(node.clone());
        }

        let Some(endpoint) = endpoint_for_node(node)? else {
            return Ok(node.clone());
        };

        let Some(trust) = self.certificate_trust_for_url(node.network, &endpoint)? else {
            return Ok(node.clone());
        };

        Ok(Node { tls: Some(trust), ..node.clone() })
    }

    /// Returns the trust for `url`, including a legacy pin embedded in the
    /// selected node until the next selected-node write migrates it
    pub(crate) fn certificate_trust_for_url(
        &self,
        _network: Network,
        url: &str,
    ) -> Result<Option<TlsTrust>> {
        if !crate::node_connect::is_ssl_electrum_endpoint(url) {
            return Ok(None);
        }

        let endpoint = crate::node_connect::normalize_certificate_endpoint(url)
            .map_err_str(GlobalConfigTableError::InvalidCertificateTrustStore)?;

        let certificate_trust = self.certificate_trust.read();

        let trust = match certificate_trust.as_ref() {
            Ok(snapshot) => snapshot.trust_for_endpoint(&endpoint)?,
            Err(error) => return Err(error.clone().into()),
        };

        if trust.is_some() {
            return Ok(trust);
        }

        let selected_node = self.stored_selected_node_for_network(_network);
        let selected_endpoint = endpoint_for_node(&selected_node)?;
        if let (Some(trust), Some(selected_endpoint)) = (selected_node.tls, selected_endpoint)
            && selected_endpoint == endpoint
        {
            return Ok(Some(trust));
        }

        Ok(None)
    }

    pub(crate) fn conflicted_selected_node_endpoint(&self, network: Network) -> Option<String> {
        let node = self.stored_selected_node_for_network(network);
        let endpoint = endpoint_for_node(&node).ok().flatten()?;
        let certificate_trust = self.certificate_trust.read();

        certificate_trust.as_ref().ok()?.is_conflicted(&endpoint).then_some(endpoint)
    }

    /// Replaces a selected legacy node after the user explicitly recovers from
    /// a conflicting embedded certificate claim
    pub(crate) fn recover_selected_node_from_certificate_trust_conflict(
        &self,
        node: &Node,
        conflict_endpoint: &str,
    ) -> Result<Node> {
        if node.tls.is_some() {
            return Err(GlobalConfigTableError::CertificateTrustConflict(
                conflict_endpoint.to_string(),
            )
            .into());
        }

        let network = node.network;
        let selected_node_key: &'static str = GlobalConfigKey::SelectedNode(network).into();
        let trust_store_key: &'static str = GlobalConfigKey::CertificateTrustStore.into();

        let saved_node = {
            // serialize the recovery with all other trust-affecting writes
            let _trust_writer = self.certificate_trust_writer.lock();
            let write_txn =
                self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            let (saved_node, effective_trust) = {
                let mut table = write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccess(error.to_string()))?;

                let current_snapshot = initialize_certificate_trust_cache(&table)?;
                if !current_snapshot.is_conflicted(conflict_endpoint) {
                    return Err(GlobalConfigTableError::CertificateTrustConflict(
                        conflict_endpoint.to_string(),
                    )
                    .into());
                }

                let previous_node_json = table
                    .get(selected_node_key)
                    .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
                    .map(|value| value.value());
                let previous_node = previous_node_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Node>(value).ok())
                    .filter(|stored| stored.network == network)
                    .unwrap_or_else(|| Node::default(network));
                let Some(previous_trust) = previous_node.tls.clone() else {
                    return Err(GlobalConfigTableError::CertificateTrustConflict(
                        conflict_endpoint.to_string(),
                    )
                    .into());
                };
                let previous_endpoint = endpoint_for_node(&previous_node)?;
                if previous_endpoint.as_deref() != Some(conflict_endpoint) {
                    return Err(GlobalConfigTableError::CertificateTrustConflict(
                        conflict_endpoint.to_string(),
                    )
                    .into());
                }

                let trust_store_json = table
                    .get(trust_store_key)
                    .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
                    .map(|value| value.value());
                let mut trust_store = trust_store_json
                    .as_deref()
                    .map(serde_json::from_str::<CertificateTrustStore>)
                    .transpose()
                    .map_err(|error| {
                        GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string())
                    })?
                    .unwrap_or_default();

                // the replaced legacy claim is no longer durable trust
                // keep a durable entry only when it does not represent that exact claim
                trust_store.remove_if_matches(conflict_endpoint, &previous_trust);

                let mut node_to_store = node.clone();
                if let Some(endpoint) = endpoint_for_node(node)? {
                    node_to_store.tls = trust_store.get(&endpoint);
                }

                let node_json = serde_json::to_string(&node_to_store)
                    .map_err(|error| SerdeError::SerializationError(error.to_string()))?;
                let trust_store_json = serde_json::to_string(&trust_store).map_err(|error| {
                    GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string())
                })?;

                table
                    .insert(trust_store_key, trust_store_json)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
                table
                    .insert(selected_node_key, node_json)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;

                let effective_trust = initialize_certificate_trust_cache(&table)?;
                effective_trust.ensure_no_conflicts()?;

                (node_to_store, effective_trust)
            };

            write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            self.replace_certificate_trust_cache(Ok(effective_trust));

            saved_node
        };

        Updater::send_update(Update::DatabaseUpdated);
        Updater::send_update(Update::SelectedNodeChanged(saved_node.clone()));

        Ok(saved_node)
    }

    /// Merges remembered trust from a backup into the durable trust store
    pub(crate) fn restore_certificate_trust_store(
        &self,
        incoming: &CertificateTrustStore,
    ) -> Result<CertificateTrustRestoreReport> {
        let incoming = CertificateTrustStore::try_from_entries(incoming.0.clone())
            .map_err(GlobalConfigTableError::InvalidCertificateTrustStore)?;

        let report;
        {
            // serialize the commit and cache publication so a newer committed writer cannot be
            // overwritten by an older writer's delayed cache update
            let _trust_writer = self.certificate_trust_writer.lock();
            let write_txn =
                self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            let effective_trust = {
                let mut table = write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccess(error.to_string()))?;
                let trust_store_key: &'static str = GlobalConfigKey::CertificateTrustStore.into();
                let mut effective_trust = initialize_certificate_trust_cache(&table)?;

                report = effective_trust.merge(&incoming)?;
                effective_trust.ensure_no_conflicts()?;
                let trust_store_json =
                    serde_json::to_string(&effective_trust.store).map_err(|error| {
                        GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string())
                    })?;

                table
                    .insert(trust_store_key, trust_store_json)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;

                let effective_trust = initialize_certificate_trust_cache(&table)?;
                effective_trust.ensure_no_conflicts()?;

                effective_trust
            };

            write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            self.replace_certificate_trust_cache(Ok(effective_trust));
        }

        Updater::send_update(Update::DatabaseUpdated);

        Ok(report)
    }
}

fn endpoint_for_node(node: &Node) -> Result<Option<String>> {
    if node.api_type != ApiType::Electrum {
        return Ok(None);
    }

    if !crate::node_connect::is_ssl_electrum_endpoint(&node.url) {
        return Ok(None);
    }

    let endpoint = crate::node_connect::normalize_certificate_endpoint(&node.url)
        .map_err_str(GlobalConfigTableError::InvalidCertificateTrustStore)?;

    Ok(Some(endpoint))
}

impl GlobalConfigTable {
    fn set_values(
        &self,
        values: impl IntoIterator<Item = (GlobalConfigKey, String)>,
    ) -> Result<()> {
        let values = values.into_iter().collect::<Vec<_>>();
        let trust_affecting = values.iter().any(|(key, _)| {
            matches!(key, GlobalConfigKey::CertificateTrustStore | GlobalConfigKey::SelectedNode(_))
        });
        {
            // keep trust-store cache publication ordered after its committed transaction
            let _trust_writer = trust_affecting.then(|| self.certificate_trust_writer.lock());
            let write_txn =
                self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            let effective_trust = {
                let mut table = write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccess(error.to_string()))?;

                for (key, value) in values {
                    let key: &'static str = key.into();
                    table
                        .insert(key, value)
                        .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
                }

                trust_affecting.then(|| initialize_certificate_trust_cache(&table))
            };

            write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            if let Some(effective_trust) = effective_trust {
                self.replace_certificate_trust_cache(effective_trust);
            }
        }

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}

#[uniffi::export]
impl GlobalConfigTable {
    pub fn select_wallet(&self, id: WalletId) -> Result<()> {
        self.set(GlobalConfigKey::SelectedWalletId, id.to_string())?;

        Ok(())
    }

    pub fn selected_wallet(&self) -> Option<WalletId> {
        let id = self.get(GlobalConfigKey::SelectedWalletId).unwrap_or(None)?;

        let wallet_id = WalletId::from(id);

        Some(wallet_id)
    }

    pub fn clear_selected_wallet(&self) -> Result<()> {
        self.delete(GlobalConfigKey::SelectedWalletId)?;

        Ok(())
    }

    pub fn selected_network(&self) -> Network {
        let network = self
            .get(GlobalConfigKey::SelectedNetwork)
            .unwrap_or(None)
            .unwrap_or_else(|| "bitcoin".to_string());

        if let Ok(network) = Network::try_from(network.as_str()) {
            network
        } else {
            self.set_selected_network(Network::Bitcoin)
                .expect("failed to set network, please report this bug");

            Network::Bitcoin
        }
    }

    pub fn set_selected_network(&self, network: Network) -> Result<()> {
        self.set(GlobalConfigKey::SelectedNetwork, network.to_string())?;
        Updater::send_update(Update::SelectedNetworkChanged(network));

        Ok(())
    }

    pub fn is_in_main_mode(&self) -> bool {
        !self.is_in_decoy_mode()
    }

    pub fn wallet_mode(&self) -> WalletMode {
        if self.is_in_decoy_mode() { WalletMode::Decoy } else { WalletMode::Main }
    }

    pub fn is_in_decoy_mode(&self) -> bool {
        self.get(GlobalConfigKey::InDecoyMode)
            .unwrap_or(None)
            .unwrap_or_else(|| "false".to_string())
            == "true"
    }

    pub fn selected_node(&self) -> Node {
        let network = self.selected_network();
        self.selected_node_for_network(network)
    }

    pub fn set_selected_node(&self, node: &Node) -> Result<()> {
        let network = node.network;
        let selected_node_key: &'static str = GlobalConfigKey::SelectedNode(network).into();
        let trust_store_key: &'static str = GlobalConfigKey::CertificateTrustStore.into();

        let saved_node = {
            // serialize the commit and cache publication because this write also persists the
            // shared certificate trust store
            let _trust_writer = self.certificate_trust_writer.lock();
            let write_txn =
                self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            let (saved_node, effective_trust) = {
                let mut table = write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccess(error.to_string()))?;

                let previous_node_json = table
                    .get(selected_node_key)
                    .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
                    .map(|value| value.value());
                let trust_store_json = table
                    .get(trust_store_key)
                    .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
                    .map(|value| value.value());

                let previous_node = previous_node_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Node>(value).ok())
                    .filter(|stored| stored.network == network)
                    .unwrap_or_else(|| Node::default(network));
                let trust_store = trust_store_json
                    .as_deref()
                    .map(serde_json::from_str::<CertificateTrustStore>)
                    .transpose()
                    .map_err(|error| {
                        GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string())
                    })?
                    .unwrap_or_default();
                let mut candidate_trust = CertificateTrustSnapshot::from_store(trust_store);

                // migrate legacy pins before replacing the selected-node record
                candidate_trust.remember_node_certificate_trust(&previous_node)?;
                candidate_trust.remember_node_certificate_trust(node)?;

                let mut node_to_store = node.clone();
                if node_to_store.tls.is_none()
                    && let Some(endpoint) = endpoint_for_node(node)?
                {
                    node_to_store.tls = candidate_trust.trust_for_endpoint(&endpoint)?;
                }

                let node_json = serde_json::to_string(&node_to_store)
                    .map_err(|error| SerdeError::SerializationError(error.to_string()))?;

                let trust_store_json =
                    serde_json::to_string(&candidate_trust.store).map_err(|error| {
                        GlobalConfigTableError::InvalidCertificateTrustStore(error.to_string())
                    })?;

                table
                    .insert(trust_store_key, trust_store_json)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
                table
                    .insert(selected_node_key, node_json)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;

                let effective_trust = initialize_certificate_trust_cache(&table)?;
                effective_trust.ensure_no_conflicts()?;

                (node_to_store, effective_trust)
            };

            write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            self.replace_certificate_trust_cache(Ok(effective_trust));

            saved_node
        };

        Updater::send_update(Update::DatabaseUpdated);
        Updater::send_update(Update::SelectedNodeChanged(saved_node));

        Ok(())
    }

    pub fn custom_block_explorer(&self, network: Network) -> Option<String> {
        self.get(GlobalConfigKey::CustomBlockExplorer(network)).unwrap_or(None).and_then(
            |template| {
                CustomBlockExplorerTemplate::parse_stored(&template)
                    .ok()
                    .map(|template| template.as_str().to_string())
            },
        )
    }

    pub fn selected_block_explorer_option(&self, network: Network) -> BlockExplorerOption {
        let stored_template =
            self.get(GlobalConfigKey::CustomBlockExplorer(network)).ok().flatten();

        BlockExplorerOption::matching_stored_template(network, stored_template.as_deref())
    }

    pub fn effective_block_explorer_preview(&self, network: Network) -> String {
        self.custom_block_explorer_transaction_url(network, PREVIEW_TXID.to_string())
    }

    pub fn preview_custom_block_explorer(&self, network: Network, input: String) -> Result<String> {
        if input.trim().is_empty() {
            return Ok(CustomBlockExplorerTemplate::default_for(network).render(PREVIEW_TXID));
        }

        let template = CustomBlockExplorerTemplate::parse(network, &input)
            .map_err(GlobalConfigTableError::from)?;

        Ok(template.render(PREVIEW_TXID))
    }

    pub fn set_custom_block_explorer(
        &self,
        network: Network,
        input: String,
    ) -> Result<Option<String>> {
        if input.trim().is_empty() {
            self.clear_custom_block_explorer(network)?;
            return Ok(None);
        }

        let template = CustomBlockExplorerTemplate::parse(network, &input)
            .map_err(GlobalConfigTableError::from)?;
        let canonical = template.as_str().to_string();
        self.set(GlobalConfigKey::CustomBlockExplorer(network), canonical.clone())?;

        Ok(Some(canonical))
    }

    pub fn set_block_explorer_option(
        &self,
        network: Network,
        option: BlockExplorerOption,
    ) -> Result<Option<String>> {
        match option {
            BlockExplorerOption::MempoolSpace => {
                self.clear_custom_block_explorer(network)?;
                Ok(None)
            }
            BlockExplorerOption::Custom => Ok(self.custom_block_explorer(network)),
            BlockExplorerOption::MempoolGuide
            | BlockExplorerOption::BullBitcoin
            | BlockExplorerOption::Blockstream => {
                let template = option.template_for_network(network).ok_or_else(|| {
                    GlobalConfigTableError::InvalidCustomBlockExplorer(format!(
                        "{} is not supported on {}",
                        option.display_name(),
                        network.display_name()
                    ))
                })?;
                let canonical = template.as_str().to_string();
                self.set(GlobalConfigKey::CustomBlockExplorer(network), canonical.clone())?;

                Ok(Some(canonical))
            }
        }
    }

    pub fn clear_custom_block_explorer(&self, network: Network) -> Result<()> {
        self.delete(GlobalConfigKey::CustomBlockExplorer(network))
    }

    #[uniffi::method(name = "selectedFiatCurrency")]
    fn _selected_fiat_currency(&self) -> FiatCurrency {
        self.fiat_currency().unwrap_or_default()
    }

    #[uniffi::method(name = "authType")]
    pub fn _auth_type(&self) -> AuthType {
        self.auth_type().unwrap_or_default()
    }

    #[uniffi::method(name = "colorScheme")]
    pub fn _color_scheme(&self) -> ColorSchemeSelection {
        self.color_scheme().unwrap_or_default()
    }

    #[uniffi::method(name = "setColorScheme")]
    pub fn _set_color_scheme(&self, color_scheme: ColorSchemeSelection) -> Result<()> {
        self.set_color_scheme(color_scheme)
    }

    pub fn hashed_pin_code(&self) -> Result<String> {
        self.priv_hashed_pin_code()
    }

    pub fn delete_hashed_pin_code(&self) -> Result<()> {
        self.delete_priv_hashed_pin_code()
    }

    pub fn set_hashed_pin_code(&self, hashed_pin_code: String) -> Result<()> {
        if hashed_pin_code.is_empty() {
            return Err(GlobalConfigTableError::PinCodeMustBeHashed.into());
        }

        if hashed_pin_code.len() <= 6 {
            return Err(GlobalConfigTableError::PinCodeMustBeHashed.into());
        }

        self.set_priv_hashed_pin_code(hashed_pin_code)
    }

    pub(crate) fn get(&self, key: GlobalConfigKey) -> Result<Option<String>> {
        let read_txn =
            self.db.begin_read().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table =
            read_txn.open_table(TABLE).map_err(|error| Error::TableAccess(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    pub(crate) fn set(&self, key: GlobalConfigKey, value: String) -> Result<()> {
        self.set_values([(key, value)])
    }

    pub fn delete(&self, key: GlobalConfigKey) -> Result<()> {
        let trust_affecting = matches!(
            key,
            GlobalConfigKey::CertificateTrustStore | GlobalConfigKey::SelectedNode(_)
        );
        {
            // keep trust-store cache publication ordered after its committed transaction
            let _trust_writer = trust_affecting.then(|| self.certificate_trust_writer.lock());
            let write_txn =
                self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            let effective_trust = {
                let mut table = write_txn
                    .open_table(TABLE)
                    .map_err(|error| Error::TableAccess(error.to_string()))?;

                let key: &'static str = key.into();
                table
                    .remove(key)
                    .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;

                trust_affecting.then(|| initialize_certificate_trust_cache(&table))
            };

            write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

            if let Some(effective_trust) = effective_trust {
                self.replace_certificate_trust_cache(effective_trust);
            }
        }

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::custom_block_explorer::BlockExplorerOption;
    use cove_types::Network;

    #[test]
    fn test_selected_node_key() {
        use super::GlobalConfigKey;

        let key: &str = GlobalConfigKey::SelectedNode(Network::Bitcoin).into();
        assert_eq!(key, "selected_node_bitcoin");

        let key: &str = GlobalConfigKey::SelectedNode(Network::Testnet).into();
        assert_eq!(key, "selected_node_testnet");
    }

    #[test]
    fn network_scoped_node_rejects_stored_network_mismatch() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let bitcoin_node = crate::node::Node::new_esplora(
            "Wrong network".into(),
            "https://bitcoin.example/api".into(),
            Network::Bitcoin,
        );
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Signet),
                serde_json::to_string(&bitcoin_node).unwrap(),
            )
            .unwrap();

        let node = table.selected_node_for_network(Network::Signet);

        assert_eq!(node, crate::node::Node::default(Network::Signet));
        assert_eq!(node.network, Network::Signet);
    }

    #[test]
    fn legacy_conflict_is_scoped_to_endpoint_and_retains_unrelated_trust() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint_a = "ssl://shared.example.com:50002";
        let endpoint_b = "ssl://bitcoin.example.com:50002";
        let trust_a = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![1; 32] };
        let conflicting_trust =
            crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![2; 32] };
        let trust_b = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![3; 32] };

        let mut durable_trust = super::CertificateTrustStore::default();
        durable_trust.insert_or_match(endpoint_b.into(), trust_b.clone()).unwrap();
        table
            .set(
                super::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&durable_trust).unwrap(),
            )
            .unwrap();

        let testnet_node = crate::node::Node {
            tls: Some(trust_a),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint_a.into(),
                Network::Testnet,
            )
        };
        let testnet4_node = crate::node::Node {
            tls: Some(conflicting_trust),
            ..crate::node::Node::new_electrum(
                "Testnet4 legacy".into(),
                endpoint_a.into(),
                Network::Testnet4,
            )
        };
        let bitcoin_node = crate::node::Node::new_electrum(
            "Bitcoin custom".into(),
            endpoint_b.into(),
            Network::Bitcoin,
        );
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&testnet_node).unwrap(),
            )
            .unwrap();
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Testnet4),
                serde_json::to_string(&testnet4_node).unwrap(),
            )
            .unwrap();
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&bitcoin_node).unwrap(),
            )
            .unwrap();

        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, endpoint_b).unwrap(),
            Some(trust_b.clone())
        );
        assert!(matches!(
            table.certificate_trust_for_url(Network::Testnet, endpoint_a),
            Err(super::Error::GlobalConfig(
                super::GlobalConfigTableError::CertificateTrustConflict(endpoint)
            )) if endpoint == endpoint_a
        ));

        let selected_bitcoin = table.selected_node_for_network(Network::Bitcoin);
        assert_eq!(selected_bitcoin.url, endpoint_b);
        assert_eq!(selected_bitcoin.tls, Some(trust_b));
    }

    #[test]
    fn controlled_writes_roll_back_when_an_unrelated_endpoint_conflict_remains() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint_a = "ssl://shared.example.com:50002";
        let endpoint_b = "ssl://bitcoin.example.com:50002";
        let endpoint_c = "ssl://new.example.com:50002";
        let trust_a = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![4; 32] };
        let conflicting_trust =
            crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![5; 32] };
        let trust_b = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![6; 32] };
        let trust_c = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![7; 32] };

        let mut durable_trust = super::CertificateTrustStore::default();
        durable_trust.insert_or_match(endpoint_b.into(), trust_b.clone()).unwrap();
        table
            .set(
                super::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&durable_trust).unwrap(),
            )
            .unwrap();
        for (network, trust) in
            [(Network::Testnet, trust_a), (Network::Testnet4, conflicting_trust)]
        {
            let node = crate::node::Node {
                tls: Some(trust),
                ..crate::node::Node::new_electrum(
                    format!("{network} legacy"),
                    endpoint_a.into(),
                    network,
                )
            };
            table
                .set(
                    super::GlobalConfigKey::SelectedNode(network),
                    serde_json::to_string(&node).unwrap(),
                )
                .unwrap();
        }
        let bitcoin_node = crate::node::Node::new_electrum(
            "Bitcoin custom".into(),
            endpoint_b.into(),
            Network::Bitcoin,
        );
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&bitcoin_node).unwrap(),
            )
            .unwrap();

        let replacement = crate::node::Node::new_esplora(
            "Bitcoin replacement".into(),
            "https://replacement.example/api".into(),
            Network::Bitcoin,
        );
        let error = table.set_selected_node(&replacement).unwrap_err();
        assert!(matches!(
            error,
            super::Error::GlobalConfig(
                super::GlobalConfigTableError::CertificateTrustConflict(endpoint)
            ) if endpoint == endpoint_a
        ));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), bitcoin_node);

        let mut incoming = super::CertificateTrustStore::default();
        incoming.insert_or_match(endpoint_c.into(), trust_c).unwrap();
        let error = table.restore_certificate_trust_store(&incoming).unwrap_err();
        assert!(matches!(
            error,
            super::Error::GlobalConfig(
                super::GlobalConfigTableError::CertificateTrustConflict(endpoint)
            ) if endpoint == endpoint_a
        ));
        assert_eq!(certificate_trust_store(&table).get(endpoint_c), None);
        let selected_bitcoin = table.selected_node_for_network(Network::Bitcoin);
        assert_eq!(selected_bitcoin.url, bitcoin_node.url);
        assert_eq!(selected_bitcoin.tls, Some(trust_b));
    }

    #[test]
    fn cloned_tables_share_certificate_trust_writer() {
        let (_tmp, table) = test_table();
        let clone = table.clone();

        assert!(Arc::ptr_eq(&table.certificate_trust_writer, &clone.certificate_trust_writer));
    }

    #[test]
    fn selected_node_write_migrates_legacy_pin_before_overwrite() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![7; 32] };
        let legacy = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Custom".into(),
                "ssl://legacy.example.com:50002".into(),
                Network::Bitcoin,
            )
        };

        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&legacy).unwrap(),
            )
            .unwrap();

        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &legacy.url).unwrap(),
            Some(trust.clone())
        );

        let preset = crate::node::Node::new_electrum(
            "Preset".into(),
            "ssl://preset.example.com:50002".into(),
            Network::Bitcoin,
        );
        table.set_selected_node(&preset).unwrap();

        let returning =
            crate::node::Node::new_electrum("Custom".into(), legacy.url.clone(), Network::Bitcoin);
        table.set_selected_node(&returning).unwrap();

        assert_eq!(
            table.stored_selected_node_for_network(Network::Bitcoin).tls,
            Some(trust.clone())
        );
        assert_eq!(table.selected_node_for_network(Network::Bitcoin).tls, Some(trust));
    }

    #[test]
    fn deleting_certificate_trust_store_preserves_embedded_pin_for_hydration() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![13; 32] };
        let node = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Embedded".into(),
                "ssl://embedded.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&node).unwrap();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Bitcoin,
            table.certificate_trust_cache(),
        );

        table.delete(super::GlobalConfigKey::CertificateTrustStore).unwrap();

        let hydrated = selector
            .parse_custom_node(
                "ssl://embedded.example.com:50002".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(hydrated.tls, Some(trust));
    }

    #[test]
    fn selected_node_write_preserves_another_networks_embedded_pin() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![14; 32] };
        let legacy_node = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Legacy".into(),
                "ssl://legacy-testnet.example.com:53012".into(),
                Network::Testnet,
            )
        };
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&legacy_node).unwrap(),
            )
            .unwrap();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Testnet,
            table.certificate_trust_cache(),
        );

        let bitcoin_node = crate::node::Node::new_electrum(
            "Bitcoin".into(),
            "ssl://bitcoin.example.com:50002".into(),
            Network::Bitcoin,
        );
        table.set_selected_node(&bitcoin_node).unwrap();

        let hydrated = selector
            .parse_custom_node(
                "ssl://legacy-testnet.example.com:53012".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(hydrated.tls, Some(trust));
    }

    #[test]
    fn conflicting_selected_node_pin_rolls_back_node_and_store_writes() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let existing_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![2; 32] };
        let existing_node = crate::node::Node {
            tls: Some(existing_trust.clone()),
            ..crate::node::Node::new_electrum(
                "Existing".into(),
                "ssl://node.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&existing_node).unwrap();

        let conflicting_node = crate::node::Node {
            name: "Conflicting".into(),
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![3; 32] }),
            ..existing_node.clone()
        };

        let error = table.set_selected_node(&conflicting_node).unwrap_err();

        assert!(matches!(
            error,
            super::Error::GlobalConfig(super::GlobalConfigTableError::CertificateTrustConflict(
                endpoint
            )) if endpoint == "ssl://node.example.com:50002"
        ));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), existing_node);
        assert_eq!(
            table
                .certificate_trust_for_url(Network::Bitcoin, "ssl://node.example.com:50002")
                .unwrap(),
            Some(existing_trust)
        );
    }

    #[test]
    fn explicit_recovery_replaces_one_conflicting_legacy_claim() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://shared.example.com:50002";
        let first_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![2; 32] };
        let remaining_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![3; 32] };
        let first_node = crate::node::Node {
            tls: Some(first_trust),
            ..crate::node::Node::new_electrum(
                "Bitcoin legacy".into(),
                endpoint.into(),
                Network::Bitcoin,
            )
        };
        let remaining_node = crate::node::Node {
            tls: Some(remaining_trust.clone()),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint.into(),
                Network::Testnet,
            )
        };
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&first_node).unwrap(),
            )
            .unwrap();
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&remaining_node).unwrap(),
            )
            .unwrap();

        let replacement = crate::node::Node::new_esplora(
            "Preset".into(),
            "https://preset.example/api".into(),
            Network::Bitcoin,
        );
        let saved = table
            .recover_selected_node_from_certificate_trust_conflict(&replacement, endpoint)
            .unwrap();

        assert_eq!(saved, replacement);
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), replacement);
        assert_eq!(
            table
                .node_with_certificate_trust(&crate::node::Node::new_electrum(
                    "Remaining".into(),
                    endpoint.into(),
                    Network::Bitcoin,
                ))
                .unwrap()
                .tls,
            Some(remaining_trust)
        );
    }

    #[test]
    fn stale_recovery_snapshot_does_not_remove_a_pin_after_conflict_is_resolved() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://shared.example.com:50002";
        let retained_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![11; 32] };
        let stale_conflicting_trust =
            crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![12; 32] };
        let bitcoin_node = crate::node::Node {
            tls: Some(retained_trust.clone()),
            ..crate::node::Node::new_electrum(
                "Bitcoin legacy".into(),
                endpoint.into(),
                Network::Bitcoin,
            )
        };
        let testnet_node = crate::node::Node {
            tls: Some(stale_conflicting_trust),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint.into(),
                Network::Testnet,
            )
        };
        let replacement_testnet = crate::node::Node::new_esplora(
            "Testnet replacement".into(),
            "https://replacement.example/api".into(),
            Network::Testnet,
        );
        let mut durable_trust = super::CertificateTrustStore::default();
        durable_trust.insert_or_match(endpoint.into(), retained_trust.clone()).unwrap();

        {
            let write_txn = table.db.begin_write().unwrap();
            {
                let mut redb_table = write_txn.open_table(super::TABLE).unwrap();
                let bitcoin_key: &'static str =
                    super::GlobalConfigKey::SelectedNode(Network::Bitcoin).into();
                let testnet_key: &'static str =
                    super::GlobalConfigKey::SelectedNode(Network::Testnet).into();
                let trust_store_key: &'static str =
                    super::GlobalConfigKey::CertificateTrustStore.into();
                redb_table
                    .insert(bitcoin_key, serde_json::to_string(&bitcoin_node).unwrap())
                    .unwrap();
                redb_table
                    .insert(testnet_key, serde_json::to_string(&testnet_node).unwrap())
                    .unwrap();
                redb_table
                    .insert(trust_store_key, serde_json::to_string(&durable_trust).unwrap())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        *table.certificate_trust.write() =
            Err(super::GlobalConfigTableError::CertificateTrustConflict(endpoint.to_string()));

        // another writer resolved the conflict while the selector still held the old snapshot
        {
            let _trust_writer = table.certificate_trust_writer.lock();
            let write_txn = table.db.begin_write().unwrap();
            {
                let mut redb_table = write_txn.open_table(super::TABLE).unwrap();
                let testnet_key: &'static str =
                    super::GlobalConfigKey::SelectedNode(Network::Testnet).into();
                redb_table
                    .insert(testnet_key, serde_json::to_string(&replacement_testnet).unwrap())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let replacement = crate::node::Node::new_esplora(
            "Bitcoin replacement".into(),
            "https://bitcoin-replacement.example/api".into(),
            Network::Bitcoin,
        );
        let error = table
            .recover_selected_node_from_certificate_trust_conflict(&replacement, endpoint)
            .unwrap_err();

        assert!(matches!(
            error,
            super::Error::GlobalConfig(super::GlobalConfigTableError::CertificateTrustConflict(
                endpoint
            )) if endpoint == "ssl://shared.example.com:50002"
        ));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), bitcoin_node);
        assert_eq!(certificate_trust_store(&table).get(endpoint), Some(retained_trust));
    }

    #[test]
    fn explicit_recovery_removes_a_durable_pin_only_when_it_matches_the_replaced_claim() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://shared.example.com:50002";
        let replaced_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![4; 32] };
        let remaining_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![5; 32] };
        let replaced_node = crate::node::Node {
            tls: Some(replaced_trust.clone()),
            ..crate::node::Node::new_electrum(
                "Bitcoin legacy".into(),
                endpoint.into(),
                Network::Bitcoin,
            )
        };
        let remaining_node = crate::node::Node {
            tls: Some(remaining_trust.clone()),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint.into(),
                Network::Testnet,
            )
        };
        let mut durable_trust = super::CertificateTrustStore::default();
        durable_trust.insert_or_match(endpoint.into(), replaced_trust).unwrap();
        table
            .set(
                super::GlobalConfigKey::CertificateTrustStore,
                serde_json::to_string(&durable_trust).unwrap(),
            )
            .unwrap();
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&replaced_node).unwrap(),
            )
            .unwrap();
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Testnet),
                serde_json::to_string(&remaining_node).unwrap(),
            )
            .unwrap();

        let replacement = crate::node::Node::new_esplora(
            "Preset".into(),
            "https://preset.example/api".into(),
            Network::Bitcoin,
        );
        table
            .recover_selected_node_from_certificate_trust_conflict(&replacement, endpoint)
            .unwrap();

        assert_eq!(certificate_trust_store(&table).get(endpoint), None);
        assert_eq!(
            table
                .node_with_certificate_trust(&crate::node::Node::new_electrum(
                    "Remaining".into(),
                    endpoint.into(),
                    Network::Bitcoin,
                ))
                .unwrap()
                .tls,
            Some(remaining_trust)
        );
    }

    #[test]
    fn explicit_recovery_rolls_back_when_other_conflicting_claims_remain() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://shared.example.com:50002";
        let first_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![6; 32] }),
            ..crate::node::Node::new_electrum(
                "Bitcoin legacy".into(),
                endpoint.into(),
                Network::Bitcoin,
            )
        };
        let second_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![7; 32] }),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint.into(),
                Network::Testnet,
            )
        };
        let third_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![8; 32] }),
            ..crate::node::Node::new_electrum(
                "Testnet4 legacy".into(),
                endpoint.into(),
                Network::Testnet4,
            )
        };
        for (network, node) in [
            (Network::Bitcoin, first_node.clone()),
            (Network::Testnet, second_node),
            (Network::Testnet4, third_node),
        ] {
            table
                .set(
                    super::GlobalConfigKey::SelectedNode(network),
                    serde_json::to_string(&node).unwrap(),
                )
                .unwrap();
        }

        let replacement = crate::node::Node::new_esplora(
            "Preset".into(),
            "https://preset.example/api".into(),
            Network::Bitcoin,
        );
        let error = table
            .recover_selected_node_from_certificate_trust_conflict(&replacement, endpoint)
            .unwrap_err();

        assert!(matches!(
            error,
            super::Error::GlobalConfig(super::GlobalConfigTableError::CertificateTrustConflict(
                endpoint
            )) if endpoint == "ssl://shared.example.com:50002"
        ));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), first_node);
    }

    #[test]
    fn corrupt_durable_trust_blocks_explicit_recovery() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://shared.example.com:50002";
        let first_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![9; 32] }),
            ..crate::node::Node::new_electrum(
                "Bitcoin legacy".into(),
                endpoint.into(),
                Network::Bitcoin,
            )
        };
        let second_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![10; 32] }),
            ..crate::node::Node::new_electrum(
                "Testnet legacy".into(),
                endpoint.into(),
                Network::Testnet,
            )
        };
        for (network, node) in
            [(Network::Bitcoin, first_node.clone()), (Network::Testnet, second_node)]
        {
            table
                .set(
                    super::GlobalConfigKey::SelectedNode(network),
                    serde_json::to_string(&node).unwrap(),
                )
                .unwrap();
        }
        table.set(super::GlobalConfigKey::CertificateTrustStore, "not-json".into()).unwrap();

        let replacement = crate::node::Node::new_esplora(
            "Preset".into(),
            "https://preset.example/api".into(),
            Network::Bitcoin,
        );
        let error = table
            .recover_selected_node_from_certificate_trust_conflict(&replacement, endpoint)
            .unwrap_err();

        assert!(matches!(
            error,
            super::Error::GlobalConfig(
                super::GlobalConfigTableError::InvalidCertificateTrustStore(_)
            )
        ));
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), first_node);
    }

    #[test]
    fn trailing_slash_uses_the_same_trust_store_endpoint() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![8; 32] };
        let accepted = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Custom".into(),
                "ssl://node.example.com:50002/".into(),
                Network::Bitcoin,
            )
        };

        table.set_selected_node(&accepted).unwrap();
        let candidate = crate::node::Node::new_electrum(
            "Custom".into(),
            "ssl://node.example.com:50002".into(),
            Network::Bitcoin,
        );

        assert_eq!(table.node_with_certificate_trust(&candidate).unwrap().tls, Some(trust));
    }

    #[test]
    fn another_endpoint_does_not_inherit_a_pin() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let accepted = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![9; 32] }),
            ..crate::node::Node::new_electrum(
                "Custom".into(),
                "ssl://node.example.com:50002".into(),
                Network::Bitcoin,
            )
        };

        table.set_selected_node(&accepted).unwrap();
        let candidate = crate::node::Node::new_electrum(
            "Custom".into(),
            "ssl://other.example.com:50002".into(),
            Network::Bitcoin,
        );

        assert_eq!(table.node_with_certificate_trust(&candidate).unwrap().tls, None);
    }

    #[test]
    fn corrupt_certificate_trust_store_is_an_error() {
        let (_tmp, table) = test_table();
        table.set(super::GlobalConfigKey::CertificateTrustStore, "not-json".into()).unwrap();

        let node = crate::node::Node::new_electrum(
            "Custom".into(),
            "ssl://node.example.com:50002".into(),
            Network::Bitcoin,
        );

        assert!(matches!(
            table.node_with_certificate_trust(&node),
            Err(super::Error::GlobalConfig(
                super::GlobalConfigTableError::InvalidCertificateTrustStore(_)
            ))
        ));
    }

    #[test]
    fn embedded_pin_is_usable_when_the_separate_store_is_corrupt() {
        let (_tmp, table) = test_table();
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![4; 32] };
        let node = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Custom".into(),
                "ssl://node.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&node).unwrap(),
            )
            .unwrap();
        table.set(super::GlobalConfigKey::CertificateTrustStore, "not-json".into()).unwrap();

        assert_eq!(table.node_with_certificate_trust(&node).unwrap(), node);
        assert_eq!(table.selected_node_for_network(Network::Bitcoin), node);
    }

    #[test]
    fn corrupt_store_does_not_return_an_unpinned_custom_ssl_node() {
        let (_tmp, table) = test_table();
        let node = crate::node::Node::new_electrum(
            "Custom".into(),
            "ssl://custom.example.com:50002".into(),
            Network::Bitcoin,
        );
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&node).unwrap(),
            )
            .unwrap();
        table.set(super::GlobalConfigKey::CertificateTrustStore, "not-json".into()).unwrap();

        assert_eq!(
            table.selected_node_for_network(Network::Bitcoin),
            crate::node::Node::default(Network::Bitcoin)
        );
    }

    #[test]
    fn trust_store_canonicalizes_transport_equivalent_endpoints() {
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![1; 32] };

        let noncanonical = serde_json::json!({
            "ssl://user:password@NODE.example.com:50002/path?query=value#fragment": trust,
        });
        let store = serde_json::from_value::<super::CertificateTrustStore>(noncanonical).unwrap();
        assert_eq!(store.get("ssl://node.example.com:50002"), Some(trust));

        let non_ssl = serde_json::json!({
            "tcp://node.example.com:50001": crate::node::tls::TlsTrust::PinnedFingerprint {
                sha256: vec![1; 32]
            },
        });
        assert!(serde_json::from_value::<super::CertificateTrustStore>(non_ssl).is_err());
    }

    #[test]
    fn certificate_trust_store_rejects_invalid_fingerprint_lengths() {
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![1; 31] };
        let mut store = super::CertificateTrustStore::default();

        assert!(matches!(
            store.insert_or_match("ssl://node.example.com:50002".to_string(), trust.clone()),
            Err(super::GlobalConfigTableError::InvalidCertificateTrustStore(_))
        ));
        assert!(
            serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
                "ssl://node.example.com:50002": trust,
            }))
            .is_err()
        );
    }

    #[test]
    fn certificate_trust_store_rejects_unusable_custom_ca() {
        let trust =
            crate::node::tls::TlsTrust::CustomCa { cert: vec![0x30, 0x03, 0x02, 0x01, 0x00] };
        let mut store = super::CertificateTrustStore::default();

        assert!(matches!(
            store.insert_or_match("ssl://node.example.com:50002".to_string(), trust.clone()),
            Err(super::GlobalConfigTableError::InvalidCertificateTrustStore(_))
        ));
        assert!(
            serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
                "ssl://node.example.com:50002": trust,
            }))
            .is_err()
        );
    }

    #[test]
    fn certificate_trust_store_accepts_a_usable_custom_ca() {
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let trust = crate::node::tls::TlsTrust::CustomCa { cert: generated.cert.der().to_vec() };
        let mut store = super::CertificateTrustStore::default();

        assert!(store.insert_or_match("ssl://node.example.com:50002".to_string(), trust).is_ok());
    }

    #[test]
    fn restoring_certificate_trust_store_reports_conflicts_and_commits_other_entries() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let existing_node = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![2; 32] }),
            ..crate::node::Node::new_electrum(
                "Existing".into(),
                "ssl://existing.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&existing_node).unwrap();

        let incoming = serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
            "ssl://existing.example.com:50002/path": {
                "PinnedFingerprint": { "sha256": vec![6; 32] }
            },
            "ssl://incoming.example.com:50002": {
                "PinnedFingerprint": { "sha256": vec![3; 32] }
            }
        }))
        .unwrap();
        let report = table.restore_certificate_trust_store(&incoming).unwrap();

        assert_eq!(
            report.conflicting_endpoints,
            vec!["ssl://existing.example.com:50002".to_string()]
        );

        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &existing_node.url).unwrap(),
            existing_node.tls
        );
        assert_eq!(
            table
                .certificate_trust_for_url(Network::Bitcoin, "ssl://incoming.example.com:50002")
                .unwrap(),
            Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![3; 32] })
        );
    }

    #[test]
    fn restoring_a_conflicting_pin_preserves_the_existing_pin() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let existing = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![5; 32] }),
            ..crate::node::Node::new_electrum(
                "Existing".into(),
                "ssl://node.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&existing).unwrap();
        let incoming = serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
            "ssl://node.example.com:50002/path": {
                "PinnedFingerprint": { "sha256": vec![6; 32] }
            }
        }))
        .unwrap();

        let report = table.restore_certificate_trust_store(&incoming).unwrap();

        assert_eq!(report.conflicting_endpoints, vec!["ssl://node.example.com:50002".to_string()]);
        assert_eq!(
            table.certificate_trust_for_url(Network::Bitcoin, &existing.url).unwrap(),
            existing.tls
        );
    }

    #[test]
    fn restoring_certificate_trust_store_merges_legacy_selected_node_pins() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let endpoint = "ssl://legacy.example.com:50002";
        let legacy_trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![15; 32] };
        let legacy_node = crate::node::Node {
            tls: Some(legacy_trust.clone()),
            ..crate::node::Node::new_electrum("Legacy".into(), endpoint.into(), Network::Bitcoin)
        };
        table
            .set(
                super::GlobalConfigKey::SelectedNode(Network::Bitcoin),
                serde_json::to_string(&legacy_node).unwrap(),
            )
            .unwrap();

        let incoming = serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
            "ssl://legacy.example.com:50002": {
                "PinnedFingerprint": { "sha256": vec![16; 32] }
            },
            "ssl://new.example.com:50002": {
                "PinnedFingerprint": { "sha256": vec![17; 32] }
            }
        }))
        .unwrap();
        let report = table.restore_certificate_trust_store(&incoming).unwrap();

        assert_eq!(report.conflicting_endpoints, vec![endpoint.to_string()]);
        assert_eq!(table.stored_selected_node_for_network(Network::Bitcoin), legacy_node);

        let trust_store = certificate_trust_store(&table);
        assert_eq!(trust_store.get(endpoint), Some(legacy_trust));
        assert_eq!(
            trust_store.get("ssl://new.example.com:50002"),
            Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![17; 32] })
        );
    }

    #[test]
    fn selector_created_before_selected_node_write_hydrates_committed_trust() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Bitcoin,
            table.certificate_trust_cache(),
        );
        let trust = crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![7; 32] };
        let accepted = crate::node::Node {
            tls: Some(trust.clone()),
            ..crate::node::Node::new_electrum(
                "Custom".into(),
                "ssl://selected.example.com:50002".into(),
                Network::Bitcoin,
            )
        };

        table.set_selected_node(&accepted).unwrap();

        let hydrated = selector
            .parse_custom_node(
                "ssl://selected.example.com:50002/".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(hydrated.tls, Some(trust));
    }

    #[test]
    fn selector_created_before_restore_hydrates_committed_trust() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Bitcoin,
            table.certificate_trust_cache(),
        );
        let incoming = serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
            "ssl://restored.example.com:50002": {
                "PinnedFingerprint": { "sha256": vec![8; 32] }
            }
        }))
        .unwrap();

        table.restore_certificate_trust_store(&incoming).unwrap();

        let hydrated = selector
            .parse_custom_node(
                "ssl://restored.example.com:50002".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(
            hydrated.tls,
            Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![8; 32] })
        );
    }

    #[test]
    fn failed_selected_node_write_does_not_update_existing_shared_trust() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let existing = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![9; 32] }),
            ..crate::node::Node::new_electrum(
                "Existing".into(),
                "ssl://unchanged.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&existing).unwrap();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Bitcoin,
            table.certificate_trust_cache(),
        );
        let conflicting = crate::node::Node {
            name: "Conflicting".into(),
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![10; 32] }),
            ..existing.clone()
        };

        assert!(table.set_selected_node(&conflicting).is_err());

        let hydrated = selector
            .parse_custom_node(
                "ssl://unchanged.example.com:50002".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(hydrated.tls, existing.tls);
    }

    #[test]
    fn failed_restore_does_not_update_existing_shared_trust() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();
        let existing = crate::node::Node {
            tls: Some(crate::node::tls::TlsTrust::PinnedFingerprint { sha256: vec![11; 32] }),
            ..crate::node::Node::new_electrum(
                "Existing".into(),
                "ssl://restore-conflict.example.com:50002".into(),
                Network::Bitcoin,
            )
        };
        table.set_selected_node(&existing).unwrap();
        let selector = crate::node_connect::NodeSelector::with_certificate_trust_cache(
            Network::Bitcoin,
            table.certificate_trust_cache(),
        );
        let conflicting =
            serde_json::from_value::<super::CertificateTrustStore>(serde_json::json!({
                "ssl://restore-conflict.example.com:50002": {
                    "PinnedFingerprint": { "sha256": vec![12; 32] }
                }
            }))
            .unwrap();

        let report = table.restore_certificate_trust_store(&conflicting).unwrap();

        assert_eq!(
            report.conflicting_endpoints,
            vec!["ssl://restore-conflict.example.com:50002".to_string()]
        );

        let hydrated = selector
            .parse_custom_node(
                "ssl://restore-conflict.example.com:50002".into(),
                "Custom Electrum".into(),
                String::new(),
                None,
            )
            .unwrap();

        assert_eq!(hydrated.tls, existing.tls);
    }

    #[test]
    fn test_custom_block_explorer_keys() {
        use super::GlobalConfigKey;

        let key: &str = GlobalConfigKey::CustomBlockExplorer(Network::Bitcoin).into();
        assert_eq!(key, "custom_block_explorer_bitcoin");

        let key: &str = GlobalConfigKey::CustomBlockExplorer(Network::Testnet).into();
        assert_eq!(key, "custom_block_explorer_testnet");

        let key: &str = GlobalConfigKey::CustomBlockExplorer(Network::Testnet4).into();
        assert_eq!(key, "custom_block_explorer_testnet4");

        let key: &str = GlobalConfigKey::CustomBlockExplorer(Network::Signet).into();
        assert_eq!(key, "custom_block_explorer_signet");
    }

    #[test]
    fn custom_block_explorer_setter_validates_normalizes_and_clears_empty() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        let saved = table
            .set_custom_block_explorer(Network::Bitcoin, " https://example.com ".to_string())
            .unwrap();
        assert_eq!(saved.as_deref(), Some("https://example.com/tx/{txid}"));
        assert_eq!(
            table.custom_block_explorer(Network::Bitcoin).as_deref(),
            Some("https://example.com/tx/{txid}")
        );

        let cleared = table.set_custom_block_explorer(Network::Bitcoin, "   ".to_string()).unwrap();
        assert_eq!(cleared, None);
        assert_eq!(table.custom_block_explorer(Network::Bitcoin), None);
    }

    #[test]
    fn block_explorer_option_setter_selects_presets_and_clears_default() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::MempoolSpace
        );

        let saved = table
            .set_block_explorer_option(Network::Bitcoin, BlockExplorerOption::Blockstream)
            .unwrap();
        assert_eq!(saved.as_deref(), Some("https://blockstream.info/tx/{txid}"));
        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::Blockstream
        );

        table
            .set_custom_block_explorer(Network::Bitcoin, "https://example.com".to_string())
            .unwrap();
        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::Custom
        );

        let cleared = table
            .set_block_explorer_option(Network::Bitcoin, BlockExplorerOption::MempoolSpace)
            .unwrap();
        assert_eq!(cleared, None);
        assert_eq!(table.custom_block_explorer(Network::Bitcoin), None);
        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::MempoolSpace
        );
    }

    #[test]
    fn custom_block_explorer_setter_expands_bare_domain_to_known_preset_template() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        let saved = table
            .set_custom_block_explorer(Network::Bitcoin, "blockstream.info/tx".to_string())
            .unwrap();

        assert_eq!(saved.as_deref(), Some("https://blockstream.info/tx/{txid}"));
        assert_eq!(
            table.custom_block_explorer(Network::Bitcoin).as_deref(),
            Some("https://blockstream.info/tx/{txid}")
        );
        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::Blockstream
        );
    }

    #[test]
    fn block_explorer_option_setter_preserves_preset_network_paths() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        let testnet = table
            .set_block_explorer_option(Network::Testnet, BlockExplorerOption::Blockstream)
            .unwrap();
        let signet = table
            .set_block_explorer_option(Network::Signet, BlockExplorerOption::Blockstream)
            .unwrap();

        assert_eq!(testnet.as_deref(), Some("https://blockstream.info/testnet/tx/{txid}"));
        assert_eq!(
            table.selected_block_explorer_option(Network::Testnet),
            BlockExplorerOption::Blockstream
        );
        assert_eq!(signet.as_deref(), Some("https://blockstream.info/signet/tx/{txid}"));
        assert_eq!(
            table.selected_block_explorer_option(Network::Signet),
            BlockExplorerOption::Blockstream
        );
    }

    #[test]
    fn block_explorer_option_setter_rejects_unsupported_preset_networks() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        assert_eq!(
            table
                .set_block_explorer_option(Network::Testnet4, BlockExplorerOption::Blockstream)
                .unwrap_err(),
            super::Error::GlobalConfig(super::GlobalConfigTableError::InvalidCustomBlockExplorer(
                "blockstream.info is not supported on Testnet4".to_string(),
            ))
        );
        assert_eq!(table.custom_block_explorer(Network::Testnet4), None);
    }

    #[test]
    fn custom_block_explorer_input_preview_validates_without_saving() {
        let (_tmp, table) = test_table();

        assert_eq!(
            table
                .preview_custom_block_explorer(Network::Bitcoin, "https://example.com".to_string())
                .unwrap(),
            "https://example.com/tx/4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
        );
        assert!(table.custom_block_explorer(Network::Bitcoin).is_none());

        assert_eq!(
            table
                .preview_custom_block_explorer(
                    Network::Bitcoin,
                    "https://bad.example/{address}".to_string(),
                )
                .unwrap_err(),
            super::Error::GlobalConfig(super::GlobalConfigTableError::InvalidCustomBlockExplorer(
                "Unsupported block explorer template placeholder".to_string(),
            ))
        );
    }

    #[test]
    fn empty_custom_block_explorer_input_preview_uses_default() {
        let (_tmp, table) = test_table();

        assert_eq!(
            table.preview_custom_block_explorer(Network::Signet, "   ".to_string()).unwrap(),
            "https://mutinynet.com/tx/4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
        );
    }

    #[test]
    fn corrupt_stored_custom_block_explorer_falls_back_to_default() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        use super::GlobalConfigKey;

        table
            .set(
                GlobalConfigKey::CustomBlockExplorer(Network::Bitcoin),
                "javascript:alert(1)".to_string(),
            )
            .unwrap();

        let txid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            table.custom_block_explorer_transaction_url(Network::Bitcoin, txid.to_string()),
            "https://mempool.space/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(table.custom_block_explorer(Network::Bitcoin), None);
        assert_eq!(
            table.selected_block_explorer_option(Network::Bitcoin),
            BlockExplorerOption::MempoolSpace
        );
    }

    fn certificate_trust_store(table: &super::GlobalConfigTable) -> super::CertificateTrustStore {
        let Some(value) = table.get(super::GlobalConfigKey::CertificateTrustStore).unwrap() else {
            return super::CertificateTrustStore::default();
        };

        serde_json::from_str(&value).unwrap()
    }

    fn test_table() -> (tempfile::TempDir, super::GlobalConfigTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let table = super::GlobalConfigTable::new(db.clone(), &write_txn);
        write_txn.commit().unwrap();

        (tmp, table)
    }
}

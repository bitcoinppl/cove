use std::sync::Arc;

use cove_util::ResultExt as _;
use redb::TableDefinition;
use tap::TapFallible as _;
use tracing::{error, warn};

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
    node::Node,
    wallet::metadata::{WalletId, WalletMode},
};

use super::{Error, error::SerdeError};
use crate::string_config_accessor;

pub const TABLE: TableDefinition<&'static str, String> = TableDefinition::new("global_config");

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum GlobalConfigKey {
    SelectedWalletId,
    SelectedNetwork,
    SelectedFiatCurrency,
    SelectedNode(Network),
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
    OhttpRelayUrl,
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
            GlobalConfigKey::OhttpRelayUrl => "ohttp_relay_url",
        }
    }
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct GlobalConfigTable {
    db: Arc<redb::Database>,
}

impl GlobalConfigTable {
    pub fn new(db: Arc<redb::Database>, write_txn: &redb::WriteTransaction) -> Self {
        // create table if it doesn't exist
        write_txn.open_table(TABLE).expect("failed to create table");

        Self { db }
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

    #[error("invalid OHTTP relay URL: {0}")]
    InvalidOhttpRelayUrl(String),
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
        let selected_node_key = GlobalConfigKey::SelectedNode(network);
        let node_json = self.get(selected_node_key).unwrap_or(None).unwrap_or_default();
        serde_json::from_str(&node_json).unwrap_or_else(|_| Node::default(network))
    }

    pub fn set_selected_node(&self, node: &Node) -> Result<()> {
        let network = node.network;
        let node_json = serde_json::to_string(node)
            .map_err(|error| SerdeError::SerializationError(error.to_string()))?;

        let selected_node_key = GlobalConfigKey::SelectedNode(network);

        self.set(selected_node_key, node_json)?;
        Updater::send_update(Update::SelectedNodeChanged(node.clone()));

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

    pub fn ohttp_relay_url(&self) -> Option<String> {
        self.get(GlobalConfigKey::OhttpRelayUrl).unwrap_or(None).and_then(|url| {
            let parsed = url::Url::parse(&url).ok()?;
            if parsed.scheme() != "https" {
                return None;
            }

            Some(parsed.to_string())
        })
    }

    pub fn set_ohttp_relay_url(&self, url: String) -> Result<Option<String>> {
        let url = url.trim().to_string();

        if url.is_empty() {
            self.clear_ohttp_relay_url()?;
            return Ok(None);
        }

        let parsed =
            url::Url::parse(&url).map_err_str(GlobalConfigTableError::InvalidOhttpRelayUrl)?;

        if parsed.scheme() != "https" {
            return Err(GlobalConfigTableError::InvalidOhttpRelayUrl(
                "relay URL must use HTTPS".to_string(),
            )
            .into());
        }

        let canonical = parsed.to_string();
        self.set(GlobalConfigKey::OhttpRelayUrl, canonical.clone())?;

        Ok(Some(canonical))
    }

    pub fn clear_ohttp_relay_url(&self) -> Result<()> {
        self.delete(GlobalConfigKey::OhttpRelayUrl)
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
        let write_txn =
            self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub fn delete(&self, key: GlobalConfigKey) -> Result<()> {
        let write_txn =
            self.db.begin_write().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key: &'static str = key.into();
            table.remove(key).map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
        }

        write_txn.commit().map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn ohttp_relay_url_is_none_when_unset() {
        let (_tmp, table) = test_table();

        assert_eq!(table.ohttp_relay_url(), None);
    }

    #[test]
    fn ohttp_relay_url_stores_and_retrieves_https_url() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        let saved = table.set_ohttp_relay_url(" https://relay.example.com ".to_string()).unwrap();

        assert_eq!(saved.as_deref(), Some("https://relay.example.com/"));
        assert_eq!(table.ohttp_relay_url().as_deref(), Some("https://relay.example.com/"));
    }

    #[test]
    fn ohttp_relay_url_rejects_http_url() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        assert_eq!(
            table.set_ohttp_relay_url("http://relay.example.com".to_string()).unwrap_err(),
            super::Error::GlobalConfig(super::GlobalConfigTableError::InvalidOhttpRelayUrl(
                "relay URL must use HTTPS".to_string(),
            ))
        );
        assert_eq!(table.ohttp_relay_url(), None);
    }

    #[test]
    fn ohttp_relay_url_rejects_invalid_url() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        assert_eq!(
            table.set_ohttp_relay_url("not a url".to_string()).unwrap_err(),
            super::Error::GlobalConfig(super::GlobalConfigTableError::InvalidOhttpRelayUrl(
                "relative URL without a base".to_string(),
            ))
        );
        assert_eq!(table.ohttp_relay_url(), None);
    }

    #[test]
    fn ohttp_relay_url_normalizes_stored_url() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        let saved = table.set_ohttp_relay_url("HTTPS://Relay.Example.com".to_string()).unwrap();

        assert_eq!(saved.as_deref(), Some("https://relay.example.com/"));
        assert_eq!(table.ohttp_relay_url().as_deref(), Some("https://relay.example.com/"));
    }

    #[test]
    fn ohttp_relay_url_clears_on_empty_input() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        table.set_ohttp_relay_url("https://relay.example.com".to_string()).unwrap();
        let cleared = table.set_ohttp_relay_url("   ".to_string()).unwrap();

        assert_eq!(cleared, None);
        assert_eq!(table.ohttp_relay_url(), None);
    }

    #[test]
    fn ohttp_relay_url_clear_removes_stored_url() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        table.set_ohttp_relay_url("https://relay.example.com".to_string()).unwrap();
        table.clear_ohttp_relay_url().unwrap();

        assert_eq!(table.ohttp_relay_url(), None);
    }

    #[test]
    fn corrupt_stored_ohttp_relay_url_returns_none() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, table) = test_table();

        use super::GlobalConfigKey;

        table.set(GlobalConfigKey::OhttpRelayUrl, "javascript:alert(1)".to_string()).unwrap();

        assert_eq!(table.ohttp_relay_url(), None);
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

use std::sync::Arc;

use redb::TableDefinition;
use tap::TapFallible as _;
use tracing::{error, warn};

use crate::{
    app::reconcile::{Update, Updater},
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
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

    fn get(&self, key: GlobalConfigKey) -> Result<Option<String>> {
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

    fn set(&self, key: GlobalConfigKey, value: String) -> Result<()> {
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
    use cove_types::Network;

    #[test]
    fn test_selected_node_key() {
        use super::GlobalConfigKey;

        let key: &str = GlobalConfigKey::SelectedNode(Network::Bitcoin).into();
        assert_eq!(key, "selected_node_bitcoin");

        let key: &str = GlobalConfigKey::SelectedNode(Network::Testnet).into();
        assert_eq!(key, "selected_node_testnet");
    }
}

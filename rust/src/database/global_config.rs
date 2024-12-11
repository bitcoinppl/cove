use std::sync::Arc;

use redb::TableDefinition;

use crate::{
    app::reconcile::{Update, Updater},
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
    network::Network,
    node::Node,
    wallet::metadata::WalletId,
};

use super::{error::SerdeError, Error};
use crate::string_config_accessor;

pub const TABLE: TableDefinition<&'static str, String> = TableDefinition::new("global_config");

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum GlobalConfigKey {
    SelectedWalletId,
    SelectedNetwork,
    SelectedNode(Network),
    ColorScheme,
    AuthType,
    HashedPinCode,
}

impl From<GlobalConfigKey> for &'static str {
    fn from(key: GlobalConfigKey) -> Self {
        match key {
            GlobalConfigKey::SelectedWalletId => "selected_wallet_id",
            GlobalConfigKey::SelectedNetwork => "selected_network",
            GlobalConfigKey::SelectedNode(Network::Bitcoin) => "selected_node_bitcoin",
            GlobalConfigKey::SelectedNode(Network::Testnet) => "selected_node_testnet",
            GlobalConfigKey::ColorScheme => "color_scheme",
            GlobalConfigKey::AuthType => "auth_type",
            GlobalConfigKey::HashedPinCode => "hashed_pin_code",
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
        auth_type,
        GlobalConfigKey::AuthType,
        AuthType,
        Update::AuthTypeChanged
    );

    string_config_accessor!(
        color_scheme,
        GlobalConfigKey::ColorScheme,
        ColorSchemeSelection,
        Update::ColorSchemeChanged
    );
}

#[uniffi::export]
impl GlobalConfigTable {
    pub fn select_wallet(&self, id: WalletId) -> Result<()> {
        self.set(GlobalConfigKey::SelectedWalletId, id.to_string())?;

        Ok(())
    }

    pub fn selected_wallet(&self) -> Option<WalletId> {
        let id = self
            .get(GlobalConfigKey::SelectedWalletId)
            .unwrap_or(None)?;

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
            .unwrap_or("bitcoin".to_string());

        let network = match Network::try_from(network.as_str()) {
            Ok(network) => network,
            Err(_) => {
                self.set_selected_network(Network::Bitcoin)
                    .expect("failed to set network, please report this bug");

                Network::Bitcoin
            }
        };

        network
    }

    pub fn set_selected_network(&self, network: Network) -> Result<()> {
        self.set(GlobalConfigKey::SelectedNetwork, network.to_string())?;

        Ok(())
    }

    pub fn selected_node(&self) -> Node {
        let network = self.selected_network();
        let selected_node_key = GlobalConfigKey::SelectedNode(network);

        let node_json = self
            .get(selected_node_key)
            .unwrap_or(None)
            .unwrap_or("".to_string());

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

    #[uniffi::method(name = "colorScheme")]
    pub fn _color_scheme(&self) -> Result<ColorSchemeSelection> {
        self.color_scheme()
    }

    #[uniffi::method(name = "setColorScheme")]
    pub fn _set_color_scheme(&self, color_scheme: ColorSchemeSelection) -> Result<()> {
        self.set_color_scheme(color_scheme)
    }

    fn get(&self, key: GlobalConfigKey) -> Result<Option<String>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        let table = read_txn
            .open_table(TABLE)
            .map_err(|error| Error::TableAccess(error.to_string()))?;

        let key: &'static str = key.into();
        let value = table
            .get(key)
            .map_err(|error| GlobalConfigTableError::Read(error.to_string()))?
            .map(|value| value.value());

        Ok(value)
    }

    fn set(&self, key: GlobalConfigKey, value: String) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .insert(key, value)
                .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }

    pub fn delete(&self, key: GlobalConfigKey) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|error| Error::TableAccess(error.to_string()))?;

            let key: &'static str = key.into();
            table
                .remove(key)
                .map_err(|error| GlobalConfigTableError::Save(error.to_string()))?;
        }

        write_txn
            .commit()
            .map_err(|error| Error::DatabaseAccess(error.to_string()))?;

        Updater::send_update(Update::DatabaseUpdated);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::network::Network;

    #[test]
    fn test_selected_node_key() {
        use super::GlobalConfigKey;

        let key: &str = GlobalConfigKey::SelectedNode(Network::Bitcoin).into();
        assert_eq!(key, "selected_node_bitcoin");

        let key: &str = GlobalConfigKey::SelectedNode(Network::Testnet).into();
        assert_eq!(key, "selected_node_testnet");
    }
}

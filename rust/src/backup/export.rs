use std::sync::Arc;

use strum::IntoEnumIterator as _;
use tracing::warn;
use zeroize::Zeroizing;

use cove_device::keychain::Keychain;
use cove_types::network::Network;

use crate::database::Database;
use crate::database::global_config::GlobalConfigKey;
use crate::label_manager::LabelManager;
use crate::wallet::metadata::{WalletMode, WalletType};

use super::crypto;
use super::error::BackupError;
use super::model::{
    AppSettings, BackupPayload, BackupResult, DescriptorPair, WalletBackup, WalletSecret,
};

struct BackupExporter {
    db: Arc<Database>,
    keychain: &'static Keychain,
    warnings: Vec<String>,
}

impl BackupExporter {
    fn new() -> Self {
        Self { db: Database::global(), keychain: Keychain::global(), warnings: Vec::new() }
    }

    fn warn(&mut self, msg: String) {
        warn!("{msg}");
        self.warnings.push(msg);
    }

    async fn gather_wallets(&mut self) -> Result<Vec<WalletBackup>, BackupError> {
        let mut backups = Vec::new();

        for network in Network::iter() {
            for mode in [WalletMode::Main, WalletMode::Decoy] {
                let wallets = self
                    .db
                    .wallets
                    .get_all(network, mode)
                    .map_err(|e| BackupError::Database(e.to_string()))?;

                for metadata in wallets {
                    let id = &metadata.id;
                    let name = &metadata.name;
                    let mode_tag = if mode == WalletMode::Decoy { " (Decoy)" } else { "" };

                    // serialize metadata to JSON value for forward compatibility
                    let metadata_value = serde_json::to_value(&metadata)
                        .map_err(|e| BackupError::Serialization(e.to_string()))?;

                    // gather secret material based on wallet type
                    let secret = match metadata.wallet_type {
                        WalletType::Hot => match self.keychain.get_wallet_key(id) {
                            Ok(Some(mnemonic)) => WalletSecret::Mnemonic(mnemonic.to_string()),
                            Ok(None) => {
                                return Err(BackupError::Gather(format!(
                                    "hot wallet '{name}' ({network}){mode_tag} has no mnemonic in keychain"
                                )));
                            }
                            Err(e) => {
                                return Err(BackupError::Keychain(format!(
                                    "failed to get mnemonic for wallet '{name}' ({network}){mode_tag}: {e}"
                                )));
                            }
                        },
                        WalletType::Cold => {
                            let is_tap_signer = metadata
                                .hardware_metadata
                                .as_ref()
                                .is_some_and(|hw| hw.is_tap_signer());

                            if is_tap_signer {
                                match self.keychain.get_tap_signer_backup(id) {
                                    Ok(Some(backup)) => WalletSecret::TapSignerBackup(backup),
                                    Ok(None) => {
                                        self.warn(format!("Tap signer wallet '{name}' ({network}){mode_tag} has no backup, exporting without it"));
                                        WalletSecret::None
                                    }
                                    Err(e) => {
                                        return Err(BackupError::Keychain(format!(
                                            "failed to read tap signer backup for wallet '{name}' ({network}){mode_tag}: {e}"
                                        )));
                                    }
                                }
                            } else {
                                WalletSecret::None
                            }
                        }
                        WalletType::XpubOnly | WalletType::WatchOnly => WalletSecret::None,
                    };

                    let xpub = match self.keychain.get_wallet_xpub(id) {
                        Ok(Some(x)) => Some(x.to_string()),
                        Ok(None) => None,
                        Err(e) => {
                            return Err(BackupError::Keychain(format!(
                                "failed to read xpub for wallet '{name}' ({network}){mode_tag}: {e}"
                            )));
                        }
                    };

                    let descriptors = match self.keychain.get_public_descriptor(id) {
                        Ok(Some((ext, int))) => Some(DescriptorPair {
                            external: ext.to_string(),
                            internal: int.to_string(),
                        }),
                        Ok(None) => None,
                        Err(e) => {
                            return Err(BackupError::Keychain(format!(
                                "failed to read descriptors for wallet '{name}' ({network}){mode_tag}: {e}"
                            )));
                        }
                    };

                    // gather labels (non-fatal)
                    let labels_jsonl = match export_labels(id.clone()).await {
                        Ok(labels) => Some(labels),
                        Err(e) => {
                            warn!("failed to export labels for wallet {id}: {e}");
                            self.warn(format!("Failed to export labels for wallet '{name}' ({network}){mode_tag}: {e}"));
                            None
                        }
                    };

                    backups.push(WalletBackup {
                        metadata: metadata_value,
                        secret,
                        descriptors,
                        xpub,
                        labels_jsonl,
                    });
                }
            }
        }

        Ok(backups)
    }

    fn gather_settings(&mut self) -> Result<AppSettings, BackupError> {
        let selected_network = self.get_config(GlobalConfigKey::SelectedNetwork);
        let selected_fiat_currency = self.get_config(GlobalConfigKey::SelectedFiatCurrency);
        let color_scheme = self.get_config(GlobalConfigKey::ColorScheme);

        let mut selected_nodes = Vec::new();
        for network in Network::iter() {
            if let Some(node_json) = self.get_config(GlobalConfigKey::SelectedNode(network)) {
                selected_nodes.push((network.to_string(), node_json));
            }
        }

        Ok(AppSettings { selected_network, selected_fiat_currency, color_scheme, selected_nodes })
    }

    fn get_config(&mut self, key: GlobalConfigKey) -> Option<String> {
        match self.db.global_config.get(key) {
            Ok(value) => value,
            Err(e) => {
                warn!("failed to read config key {key:?}: {e}");
                self.warnings.push(format!("Failed to read a setting ({key:?}), using default"));
                None
            }
        }
    }
}

pub async fn export_all(password: String) -> Result<BackupResult, BackupError> {
    let password = Zeroizing::new(password);
    let password = crypto::clean_password(&password)?;

    let mut exporter = BackupExporter::new();
    let wallets = exporter.gather_wallets().await?;
    let settings = exporter.gather_settings()?;

    let payload = BackupPayload::new(wallets, settings);

    let json =
        serde_json::to_vec(&payload).map_err(|e| BackupError::Serialization(e.to_string()))?;
    let json = Zeroizing::new(json);

    let compressed = crypto::compress(&json)?;
    let compressed = Zeroizing::new(compressed);

    let encrypted = crypto::encrypt(&compressed, &password)?;

    let timestamp = jiff::Timestamp::now().strftime("%Y%m%d_%H%M%S");
    let filename = format!("cove_backup_{timestamp}.covb");

    Ok(BackupResult { data: encrypted, filename, warnings: exporter.warnings })
}

async fn export_labels(id: cove_types::WalletId) -> Result<String, BackupError> {
    let manager = LabelManager::try_new(id.clone()).map_err(|e| {
        let error_msg = e.to_string();

        // v1 encrypted databases are unsupported, delete them
        if error_msg.contains("unsupported") {
            tracing::error!("deleting unsupported v1 wallet database for {id}: {error_msg}");
            if let Err(err) = crate::database::wallet_data::delete_database(&id) {
                tracing::error!("failed to delete v1 wallet database for {id}: {err}");
            }
        } else {
            tracing::error!("failed to open wallet database for {id}: {error_msg}");
        }

        BackupError::Gather(error_msg)
    })?;

    manager.export().await.map_err(|e| BackupError::Gather(e.to_string()))
}

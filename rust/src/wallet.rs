pub mod address;
pub mod balance;
pub mod confirm;
pub mod ffi;
pub mod fingerprint;
pub mod metadata;

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    str::FromStr as _,
    sync::Arc,
};

use crate::{
    consts::ROOT_DATA_DIR,
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    keys::Descriptors,
    mnemonic::MnemonicExt as _,
    network::Network,
    xpub::{self, XpubError},
};
use balance::Balance;
use bdk_file_store::Store;
use bdk_wallet::{
    bitcoin::bip32::Fingerprint as BdkFingerprint, descriptor::ExtendedDescriptor,
    keys::DescriptorPublicKey, KeychainKind,
};
use bip39::Mnemonic;
use fingerprint::Fingerprint;
use metadata::{DiscoveryState, WalletId, WalletMetadata};
use pubport::formats::Format;
use tracing::{debug, error, warn};

pub type Address = address::Address;
pub type AddressWithNetwork = address::AddressWithNetwork;
pub type AddressInfo = address::AddressInfo;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
pub enum WalletError {
    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("unsupported wallet: {0}")]
    UnsupportedWallet(String),

    #[error("failed to save wallet: {0}")]
    PersistError(String),

    #[error("failed to load wallet: {0}")]
    LoadError(String),

    #[error("failed to save in keychain: {0}")]
    KeychainError(#[from] KeychainError),

    #[error("failed to save in database: {0}")]
    DatabaseError(#[from] database::Error),

    #[error("wallet not found")]
    WalletNotFound,

    #[error("metadata not found")]
    MetadataNotFound,

    #[error("failed to parse xpub: {0}")]
    ParseXpubError(#[from] XpubError),

    #[error("tryin to import a wallet that already exists")]
    WalletAlreadyExists(WalletId),
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub network: Network,
    pub bdk: bdk_wallet::PersistedWallet<Store<bdk_wallet::ChangeSet>>,
    pub metadata: WalletMetadata,

    db: Store<bdk_wallet::ChangeSet>,
}

#[derive(
    Debug,
    Clone,
    Default,
    Eq,
    PartialEq,
    Copy,
    Hash,
    Ord,
    PartialOrd,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Enum,
    strum::EnumIter,
)]
pub enum WalletAddressType {
    #[default]
    NativeSegwit,
    WrappedSegwit,
    Legacy,
}

impl Wallet {
    /// Create a new wallet from the given mnemonic save the bdk wallet filestore, save in our database and select it
    pub fn try_new_persisted_and_selected(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();

        let create_wallet = || -> Result<Self, WalletError> {
            // create bdk wallet filestore, set id to metadata id
            let me = Self::try_new_persisted_from_mnemonic_segwit(
                metadata.clone(),
                mnemonic.clone(),
                passphrase,
            )?;

            // save mnemonic for private key
            keychain.save_wallet_key(&me.id, mnemonic.clone())?;

            // save public key in keychain too
            let xpub = mnemonic.xpub(me.network.into());
            keychain.save_wallet_xpub(&me.id, xpub)?;

            // save wallet_metadata to database
            database.wallets.create_wallet(me.metadata.clone())?;

            // set this wallet as the selected wallet
            database.global_config.select_wallet(me.id.clone())?;

            Ok(me)
        };

        // clean up if we fail to create the wallet
        let me = match create_wallet() {
            Ok(me) => me,
            Err(error) => {
                error!("failed to create wallet: {error}");

                keychain.delete_wallet_key(&metadata.id);
                keychain.delete_wallet_xpub(&metadata.id);

                if let Err(error) = delete_data_path(&metadata.id) {
                    warn!("clean up failed, failed to delete wallet data: {error}");
                };

                if let Err(error) = database.wallets.delete(&metadata.id) {
                    warn!("clean up failed, failed to delete wallet: {error}");
                }

                if let Err(error) = database.global_config.clear_selected_wallet() {
                    warn!("clean up failed, failed to clear selected wallet: {error}");
                }

                return Err(error);
            }
        };

        Ok(me)
    }

    /// Try to load an existing wallet from the persisted bdk wallet filestore
    pub fn try_load_persisted(id: WalletId) -> Result<Self, WalletError> {
        let network = Database::global().global_config.selected_network();

        let mut db =
            Store::<bdk_wallet::ChangeSet>::open(id.to_string().as_bytes(), data_path(&id))
                .map_err(|error| WalletError::LoadError(error.to_string()))?;

        let wallet = bdk_wallet::Wallet::load()
            .load_wallet(&mut db)
            .map_err(|error| WalletError::LoadError(error.to_string()))?
            .ok_or(WalletError::WalletNotFound)?;

        let metadata = Database::global()
            .wallets
            .get(&id, network)
            .map_err(WalletError::DatabaseError)?
            .ok_or(WalletError::WalletNotFound)?;

        Ok(Self {
            id,
            network,
            metadata,
            bdk: wallet,
            db,
        })
    }

    /// Create a new watch-only wallet from the given xpub
    pub fn try_new_persisted_from_xpub(xpub: String) -> Result<Self, WalletError> {
        let hardware_export = pubport::Format::try_new_from_str(&xpub)
            .map_err(Into::into)
            .map_err(WalletError::ParseXpubError)?;

        Self::try_new_persisted_from_pubport(hardware_export)
    }

    /// Import from a hardware export
    pub fn try_new_persisted_from_pubport(pubport: pubport::Format) -> Result<Self, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();
        let network = Database::global().global_config.selected_network();

        let id = WalletId::new();
        let mut metadata = WalletMetadata::new_with_id(id.clone(), "", None);

        let mut db = Store::<bdk_wallet::ChangeSet>::open_or_create_new(
            id.to_string().as_bytes(),
            data_path(&id),
        )
        .map_err(|error| WalletError::PersistError(error.to_string()))?;

        let descriptors = match pubport {
            Format::Descriptor(descriptors) => descriptors,
            Format::Json(json) => {
                let descriptors = json.bip84.clone().ok_or(WalletError::ParseXpubError(
                    xpub::XpubError::MissingXpub("No BIP84 xpub found".to_string()),
                ))?;

                metadata.discovery_state = DiscoveryState::StartedJson(Arc::new(json.into()));

                descriptors
            }
            Format::Wasabi(descriptors) => descriptors,
            Format::Electrum(descriptors) => descriptors,
        };

        let fingerprint = descriptors.fingerprint();

        // make sure its not already imported
        if let Some(fingerprint) = fingerprint.as_ref() {
            let fingerprint: Fingerprint = (*fingerprint).into();

            // update the fingerprint
            metadata.master_fingerprint = Some(fingerprint.into());

            let all_fingerprints: Vec<(WalletId, Arc<Fingerprint>)> = Database::global()
                .wallets
                .get_all(network)
                .map(|wallets| {
                    wallets
                        .into_iter()
                        .filter_map(|wallet_metadata| {
                            let fingerprint = wallet_metadata.master_fingerprint?;
                            Some((wallet_metadata.id, fingerprint))
                        })
                        .collect()
                })
                .unwrap_or_default();

            if let Some((id, _)) = all_fingerprints
                .into_iter()
                .find(|(_, f)| f.as_ref() == &fingerprint)
            {
                return Err(WalletError::WalletAlreadyExists(id));
            }
        }

        let fingerprint = fingerprint.map(|s| s.to_string());
        let xpub = descriptors
            .xpub()
            .map_err(Into::into)
            .map_err(WalletError::ParseXpubError)?;

        metadata.name = match fingerprint {
            Some(fingerprint) => format!("HWW Import ({})", fingerprint.to_ascii_uppercase()),
            None => "HWW Import".to_string(),
        };

        let descriptors: Descriptors = descriptors.into();

        let wallet = descriptors
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut db)
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

        // save public key in keychain too
        keychain.save_wallet_xpub(&id, xpub)?;

        // save wallet_metadata to database
        database.wallets.create_wallet(metadata.clone())?;

        Ok(Self {
            id,
            metadata,
            network,
            bdk: wallet,
            db,
        })
    }

    /// The user imported a hww and wants to switch from native segwit to a different address type
    pub fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> Result<(), WalletError> {
        debug!("switching public descriptor wallet to new address type");

        let id = self.id.clone();

        // delete the bdk wallet filestore
        std::fs::remove_file(data_path(&self.id)).map_err(|error| {
            WalletError::PersistError(format!("failed to delete wallet filestore: {error}"))
        })?;

        let mut db = Store::<bdk_wallet::ChangeSet>::open_or_create_new(
            id.to_string().as_bytes(),
            data_path(&id),
        )
        .map_err(|error| WalletError::PersistError(error.to_string()))?;

        let descriptors: Descriptors = descriptors.into();
        let wallet = descriptors
            .into_create_params()
            .network(self.network.into())
            .create_wallet(&mut db)
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

        // switch db and wallet
        self.db = db;
        self.bdk = wallet;
        self.metadata.address_type = address_type;
        self.metadata.discovery_state = DiscoveryState::ChoseAdressType;

        Ok(())
    }

    /// The user imported a hot wallet and wants to switch from native segwit to a different address type
    pub fn switch_mnemonic_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> Result<(), WalletError> {
        debug!("switching mnemonic wallet to new address type");

        // delete the bdk wallet filestore
        std::fs::remove_file(data_path(&self.id)).map_err(|error| {
            WalletError::PersistError(format!("failed to delete wallet filestore: {error}"))
        })?;

        let mnemonic = Keychain::global()
            .get_wallet_key(&self.id)
            .ok()
            .flatten()
            .ok_or(WalletError::WalletNotFound)?;

        let mut me = Self::try_new_persisted_from_mnemonic(
            self.metadata.clone(),
            mnemonic,
            None,
            address_type,
        )?;

        // swap th wallet to the new one
        std::mem::swap(&mut me, self);
        self.metadata.address_type = address_type;
        self.metadata.discovery_state = DiscoveryState::ChoseAdressType;

        Ok(())
    }

    fn try_new_persisted_from_mnemonic_segwit(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        Self::try_new_persisted_from_mnemonic(
            metadata,
            mnemonic,
            passphrase,
            WalletAddressType::NativeSegwit,
        )
    }

    fn try_new_persisted_from_mnemonic(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
        address_type: WalletAddressType,
    ) -> Result<Self, WalletError> {
        let network = Database::global().global_config.selected_network();

        let id = metadata.id.clone();
        let mut db = Store::<bdk_wallet::ChangeSet>::open_or_create_new(
            id.to_string().as_bytes(),
            data_path(&id),
        )
        .map_err(|error| WalletError::PersistError(error.to_string()))?;

        let descriptors = mnemonic.into_descriptors(passphrase, network, address_type);

        let wallet = descriptors
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut db)
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

        Ok(Self {
            id,
            metadata,
            network,
            bdk: wallet,
            db,
        })
    }

    pub fn balance(&self) -> Balance {
        self.bdk.balance().into()
    }

    pub fn get_pub_key(&self) -> Result<DescriptorPublicKey, WalletError> {
        use bdk_wallet::miniscript::descriptor::ShInner;
        use bdk_wallet::miniscript::Descriptor;

        let extended_descriptor: ExtendedDescriptor =
            self.bdk.public_descriptor(KeychainKind::External).clone();

        let key = match extended_descriptor {
            Descriptor::Pkh(pk) => pk.into_inner(),
            Descriptor::Wpkh(pk) => pk.into_inner(),
            Descriptor::Tr(pk) => pk.internal_key().clone(),
            Descriptor::Sh(pk) => match pk.into_inner() {
                ShInner::Wpkh(pk) => pk.into_inner(),
                _ => {
                    return Err(WalletError::UnsupportedWallet(
                        "unsupported wallet bare descriptor not wpkh".to_string(),
                    ))
                }
            },
            // not sure
            Descriptor::Bare(pk) => pk.as_inner().iter_pk().next().unwrap(),
            // multi-sig
            Descriptor::Wsh(_pk) => {
                return Err(WalletError::UnsupportedWallet(
                    "unsupported wallet, multisig".to_string(),
                ))
            }
        };

        Ok(key)
    }

    pub fn get_next_address(&mut self) -> Result<AddressInfo, WalletError> {
        const MAX_ADDRESSES: usize = 25;

        let addresses: Vec<AddressInfo> = self
            .bdk
            .list_unused_addresses(KeychainKind::External)
            .take(MAX_ADDRESSES)
            .map(Into::into)
            .collect();

        // get up to 25 revealed but unused addresses
        if addresses.len() < MAX_ADDRESSES {
            let address_info = self.bdk.reveal_next_address(KeychainKind::External).into();
            self.persist()?;

            return Ok(address_info);
        }

        // if we have already revealed 25 addresses, we cycle back to the first one
        // and present those addresses, until a next unused address is available, if we don't
        // do this we could hit the gap limit and users might use a an adddress past
        // the gap limit and not be able to see it their wallet
        //
        // note: index to use is the index of the address in the list of addresses, not the derivation index
        let index_to_use = if let Some(last_index) = self
            .metadata
            .internal_mut()
            .last_seen_address_index(&addresses)
        {
            (last_index + 1) % MAX_ADDRESSES
        } else {
            0
        };

        let address_info = addresses[index_to_use].clone();
        self.metadata
            .internal_mut()
            .set_last_seen_address_index(&addresses, index_to_use);

        Database::global()
            .wallets
            .update_wallet_metadata(self.metadata.clone())?;

        Ok(address_info)
    }

    pub fn master_fingerprint(&self) -> Result<BdkFingerprint, WalletError> {
        let key = self.get_pub_key()?;
        Ok(key.master_fingerprint())
    }

    pub fn persist(&mut self) -> Result<(), WalletError> {
        self.bdk
            .persist(&mut self.db)
            .map_err(|error| WalletError::PersistError(error.to_string()))?;

        Ok(())
    }
}

#[uniffi::export]
impl Wallet {
    // Create a dummy wallet for xcode previews
    #[uniffi::constructor(name = "previewNewWallet")]
    pub fn preview_new_wallet() -> Self {
        let mnemonic = Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();
        let passphrase = None;
        let metadata = WalletMetadata::preview_new();

        if let Err(error) = delete_data_path(&metadata.id) {
            debug!("clean up failed, failed to delete wallet data: {error}");
        }

        if let Err(error) = Database::global().wallets.delete(&metadata.id) {
            debug!("clean up failed, failed to delete wallet: {error}");
        }

        Self::try_new_persisted_from_mnemonic_segwit(metadata, mnemonic, passphrase).unwrap()
    }

    pub fn id(&self) -> WalletId {
        self.id.clone()
    }
}

impl Deref for Wallet {
    type Target = bdk_wallet::PersistedWallet<Store<bdk_wallet::ChangeSet>>;

    fn deref(&self) -> &Self::Target {
        &self.bdk
    }
}

impl DerefMut for Wallet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bdk
    }
}

impl WalletAddressType {
    pub fn index(&self) -> usize {
        match self {
            WalletAddressType::NativeSegwit => 0,
            WalletAddressType::WrappedSegwit => 1,
            WalletAddressType::Legacy => 2,
        }
    }
}

pub fn delete_data_path(wallet_id: &WalletId) -> Result<(), std::io::Error> {
    let path = data_path(wallet_id);
    std::fs::remove_file(path)?;

    crate::database::wallet_data::delete_database(wallet_id)?;

    Ok(())
}

fn data_path(wallet_id: &WalletId) -> PathBuf {
    let db = format!("bdk_wallet_{}.db", wallet_id.as_str().to_lowercase());
    ROOT_DATA_DIR.join(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint() {
        crate::database::delete_database();

        let mnemonic = Mnemonic::parse_normalized(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();

        let metadata = WalletMetadata::preview_new();

        let wallet =
            Wallet::try_new_persisted_from_mnemonic_segwit(metadata.clone(), mnemonic, None)
                .unwrap();

        let fingerprint = wallet.master_fingerprint();
        let _ = delete_data_path(&metadata.id);

        assert_eq!("73c5da0a", fingerprint.unwrap().to_string().as_str());
    }
}

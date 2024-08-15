pub mod address;
pub mod balance;
pub mod fingerprint;
pub mod metadata;

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    str::FromStr as _,
};

use crate::{
    consts::ROOT_DATA_DIR,
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    mnemonic::MnemonicExt as _,
    network::Network,
};
use balance::Balance;
use bdk_file_store::Store;
use bdk_wallet::{
    bitcoin::bip32::Fingerprint, descriptor::ExtendedDescriptor, keys::DescriptorPublicKey,
    KeychainKind,
};
use bip39::Mnemonic;
use metadata::{WalletId, WalletMetadata};
use tracing::{error, warn};

pub type Address = address::Address;
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
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub network: Network,
    pub bdk: bdk_wallet::PersistedWallet,
    pub metadata: WalletMetadata,

    last_seen_address_index: Option<usize>,
    db: Store<bdk_wallet::ChangeSet>,
}

impl Wallet {
    /// Create a new wallet from the given mnemonic
    /// save the bdk wallet filestore,
    /// save in our database and select it
    pub fn try_new_persisted_and_selected(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();

        let create_wallet = || -> Result<Self, WalletError> {
            // create bdk wallet filestore, set id to metadata id
            let me = Self::try_new_persisted_from_mnemonic(
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
            database.wallets.save_wallet(metadata.clone())?;

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
            last_seen_address_index: None,
            db,
        })
    }

    fn try_new_persisted_from_mnemonic(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Self, WalletError> {
        let network = Database::global().global_config.selected_network();

        let id = metadata.id.clone();
        let mut db = Store::<bdk_wallet::ChangeSet>::open_or_create_new(
            id.to_string().as_bytes(),
            data_path(&id),
        )
        .map_err(|error| WalletError::PersistError(error.to_string()))?;

        let descriptors = mnemonic.into_descriptors(passphrase, network);
        let wallet = descriptors
            .to_create_params()
            .network(network.into())
            .create_wallet(&mut db)
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

        Ok(Self {
            id,
            metadata,
            network,
            bdk: wallet,
            last_seen_address_index: None,
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
        let index_to_use = if let Some(last_index) = self.last_seen_address_index {
            (last_index + 1) % MAX_ADDRESSES
        } else {
            0
        };

        let address_info = addresses[index_to_use].clone();
        self.last_seen_address_index = Some(index_to_use);

        Ok(address_info)
    }

    pub fn master_fingerprint(&self) -> Result<Fingerprint, WalletError> {
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

        Self::try_new_persisted_from_mnemonic(metadata, mnemonic, passphrase).unwrap()
    }

    pub fn id(&self) -> WalletId {
        self.id.clone()
    }
}

impl Deref for Wallet {
    type Target = bdk_wallet::PersistedWallet;

    fn deref(&self) -> &Self::Target {
        &self.bdk
    }
}

impl DerefMut for Wallet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bdk
    }
}

pub fn delete_data_path(wallet_id: &WalletId) -> Result<(), std::io::Error> {
    let path = data_path(wallet_id);
    std::fs::remove_file(path)?;
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
            Wallet::try_new_persisted_from_mnemonic(metadata.clone(), mnemonic, None).unwrap();
        let fingerprint = wallet.master_fingerprint();

        delete_data_path(&metadata.id).unwrap();

        assert_eq!("73c5da0a", fingerprint.unwrap().to_string().as_str());
    }
}

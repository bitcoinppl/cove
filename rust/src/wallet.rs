pub mod balance;
pub mod ffi;
pub mod fingerprint;
pub mod metadata;

use std::{str::FromStr as _, sync::Arc};

use crate::{
    bdk_store::BdkStore,
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    keys::{Descriptor, Descriptors},
    mnemonic::MnemonicExt as _,
    multi_format::MultiFormatError,
    tap_card::tap_signer_reader::DeriveInfo,
    xpub::{self, XpubError},
};
use balance::Balance;
use bdk_wallet::chain::rusqlite::Connection;
use bdk_wallet::{KeychainKind, descriptor::ExtendedDescriptor, keys::DescriptorPublicKey};
use bip39::Mnemonic;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_common::consts::GAP_LIMIT;
use cove_types::{Network, address::AddressInfoWithDerivation};
use cove_util::result_ext::ResultExt as _;
use eyre::Context as _;
use fingerprint::Fingerprint;
use metadata::{DiscoveryState, HardwareWalletMetadata, WalletId, WalletMetadata, WalletType};
use parking_lot::Mutex;
use pubport::formats::Format;
use tracing::{debug, error, warn};

pub use cove_types::address;

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

    #[error("trying to import a wallet that already exists")]
    WalletAlreadyExists(WalletId),

    #[error(transparent)]
    MultiFormatError(#[from] MultiFormatError),

    #[error("failed to parse descriptor: {0}")]
    DescriptorKeyParseError(String),
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub network: Network,
    pub bdk: bdk_wallet::PersistedWallet<Connection>,
    pub metadata: WalletMetadata,
    db: Mutex<Connection>,
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

            let (external_descriptor, internal_descriptor) = {
                let external_descriptor = me.bdk.public_descriptor(KeychainKind::External);
                let internal_descriptor = me.bdk.public_descriptor(KeychainKind::Internal);

                (external_descriptor.clone(), internal_descriptor.clone())
            };

            // save public descriptors in keychain too
            keychain.save_public_descriptor(&me.id, external_descriptor, internal_descriptor)?;

            // save wallet_metadata to database
            database.wallets.save_new_wallet_metadata(me.metadata.clone())?;

            // set this wallet as the selected wallet
            database.global_config.select_wallet(me.id.clone())?;

            Ok(me)
        };

        // clean up if we fail to create the wallet
        let me = match create_wallet() {
            Ok(me) => me,
            Err(error) => {
                error!("failed to create wallet: {error}");

                // delete the secret key, xpub and public descriptor from the keychain
                keychain.delete_wallet_items(&metadata.id);

                if let Err(error) = delete_wallet_specific_data(&metadata.id) {
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
        let mode = Database::global().global_config.wallet_mode();

        let mut store = crate::bdk_store::BdkStore::try_new(&id, network)
            .map_err_str(WalletError::LoadError)?;

        let wallet = bdk_wallet::Wallet::load()
            .load_wallet(&mut store.conn)
            .map_err_str(WalletError::LoadError)?
            .ok_or(WalletError::WalletNotFound)?;

        let mut metadata = Database::global()
            .wallets
            .get(&id, network, mode)
            .map_err(WalletError::DatabaseError)?
            .ok_or(WalletError::WalletNotFound)?;

        // set and save the origin if not set
        // we should be able to remove this because we should always have the origin
        // unless its a xpub only wallet
        if metadata.origin.is_none() && metadata.wallet_type != WalletType::XpubOnly {
            warn!("no origin found, setting using descriptor");
            let extended_descriptor = wallet.public_descriptor(KeychainKind::External);
            let descriptor = Descriptor::from(extended_descriptor.clone());
            let origin = descriptor.full_origin().ok();

            metadata.origin = origin;

            if let Err(error) =
                Database::global().wallets.save_new_wallet_metadata(metadata.clone())
            {
                warn!("failed to save wallet origin into metadata: {error}");
            }
        }

        Ok(Self { id, network, metadata, bdk: wallet, db: Mutex::new(store.conn) })
    }

    /// Create a new watch-only wallet from the given xpub
    pub fn try_new_persisted_from_xpub(xpub: String) -> Result<Self, WalletError> {
        let xpub = xpub.trim();
        let hardware_export = pubport::Format::try_new_from_str(xpub)
            .map_err(Into::into)
            .map_err(WalletError::ParseXpubError);

        if let Ok(hardware_export) = hardware_export {
            return Self::try_new_persisted_from_pubport(hardware_export);
        }

        let xpub = xpub.trim();
        if xpub.starts_with("UR:") || xpub.starts_with("ur:") {
            return Err(MultiFormatError::UrFormatNotSupported.into());
        }

        // already returned if its a valid xpub
        Err(hardware_export.unwrap_err())
    }

    /// Import from a hardware export
    pub fn try_new_persisted_from_pubport(pubport: pubport::Format) -> Result<Self, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();
        let network = database.global_config.selected_network();
        let mode = database.global_config.wallet_mode();

        let id = WalletId::new();
        let mut metadata = WalletMetadata::new_for_hardware(id.clone(), "", None);

        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let pubport_descriptors = match pubport {
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
            Format::KeyExpression(descriptors) => descriptors,
        };

        let fingerprint = pubport_descriptors.fingerprint();

        // make sure its not already imported
        if let Some(fingerprint) = fingerprint.as_ref() {
            // update the fingerprint
            let fingerprint: Fingerprint = (*fingerprint).into();
            metadata.master_fingerprint = Some(fingerprint.into());

            check_for_duplicate_wallet(network, mode, fingerprint)?;
        }

        let fingerprint = fingerprint.map(|s| s.to_string());
        let xpub =
            pubport_descriptors.xpub().map_err(Into::into).map_err(WalletError::ParseXpubError)?;

        let descriptors: Descriptors = pubport_descriptors.into();

        metadata.name = match &fingerprint {
            Some(fingerprint) => format!("Imported {}", fingerprint.to_ascii_uppercase()),
            None => "Imported XPub".to_string(),
        };

        metadata.wallet_type = match &fingerprint {
            Some(_) => WalletType::Cold,
            None => WalletType::XpubOnly,
        };

        // get origin only if its not a watch only wallet
        match metadata.wallet_type {
            WalletType::Hot | WalletType::Cold => {
                metadata.origin = descriptors.origin().ok();
            }
            _ => {}
        }

        let wallet = descriptors
            .clone()
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        // save public key in keychain too
        keychain.save_wallet_xpub(&id, xpub)?;

        // save public descriptor in keychain too
        keychain.save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        database.wallets.save_new_wallet_metadata(metadata.clone())?;

        Ok(Self { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    pub fn try_new_persisted_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive: DeriveInfo,
        backup: Option<Vec<u8>>,
    ) -> Result<Self, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();
        let mode = database.global_config.wallet_mode();
        let network = database.global_config.selected_network();
        assert!(network == derive.network);

        let id = WalletId::new();

        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let descriptors = Descriptors::new_from_tap_signer(&derive)
            .map_err_str(WalletError::DescriptorKeyParseError)?;

        let fingerprint = Fingerprint::from(derive.master_fingerprint());

        // set metadata
        let mut metadata = WalletMetadata::new_for_hardware(id.clone(), "", None);
        metadata.name = "TAPSIGNER".to_string();
        metadata.wallet_mode = mode;
        metadata.hardware_metadata = Some(HardwareWalletMetadata::TapSigner(tap_signer));
        metadata.origin = descriptors.origin().ok();
        metadata.master_fingerprint = Some(Arc::new(fingerprint));
        metadata.wallet_type = WalletType::Cold;

        // make sure its not already imported
        check_for_duplicate_wallet(network, mode, fingerprint)?;

        let xpub =
            descriptors.external.xpub().expect("tap_signer descriptor always made with xpub");

        let wallet = descriptors
            .clone()
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        // save public key in keychain too
        keychain.save_wallet_xpub(&id, xpub)?;

        // save public descriptor in keychain too
        keychain.save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        // if theres a backup for this wallet, save it in the keychain
        if let Some(backup) = backup {
            keychain.save_tap_signer_backup(&id, backup.as_slice())?;
        }

        database.wallets.save_new_wallet_metadata(metadata.clone())?;

        Ok(Self { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
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
        BdkStore::delete_sqlite_store(&self.id).map_err(|error| {
            WalletError::PersistError(format!("failed to delete wallet filestore: {error}"))
        })?;

        let store = BdkStore::try_new(&id, self.network);
        let mut db = store.map_err_str(WalletError::LoadError)?.conn;

        let descriptors: Descriptors = descriptors.into();
        let wallet = descriptors
            .into_create_params()
            .network(self.network.into())
            .create_wallet(&mut db)
            .map_err_str(WalletError::BdkError)?;

        // switch db and wallet
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
        BdkStore::delete_sqlite_store(&self.id).map_err(|error| {
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
        mut metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
        address_type: WalletAddressType,
    ) -> Result<Self, WalletError> {
        let network = Database::global().global_config.selected_network();

        let id = metadata.id.clone();
        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let descriptors = mnemonic.into_descriptors(passphrase, network, address_type);
        let origin = descriptors.origin().ok();

        metadata.master_fingerprint = descriptors.fingerprint().map(|f| Arc::new(f.into()));
        metadata.origin = origin;

        let wallet = descriptors
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        Ok(Self { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    pub fn balance(&self) -> Balance {
        self.bdk.balance().into()
    }

    #[allow(dead_code)]
    pub fn public_external_descriptor(&self) -> crate::keys::Descriptor {
        let extended_descriptor: ExtendedDescriptor =
            self.bdk.public_descriptor(KeychainKind::External).clone();

        crate::keys::Descriptor::from(extended_descriptor)
    }

    #[allow(dead_code)]
    pub fn get_pub_key(&self) -> Result<DescriptorPublicKey, WalletError> {
        use bdk_wallet::miniscript::Descriptor;
        use bdk_wallet::miniscript::descriptor::ShInner;

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
                    ));
                }
            },
            // not sure
            Descriptor::Bare(pk) => pk.as_inner().iter_pk().next().unwrap(),
            // multi-sig
            Descriptor::Wsh(_pk) => {
                return Err(WalletError::UnsupportedWallet(
                    "unsupported wallet, multisig".to_string(),
                ));
            }
        };

        Ok(key)
    }

    pub fn get_next_address(&mut self) -> Result<AddressInfoWithDerivation, WalletError> {
        const MAX_ADDRESSES: usize = (GAP_LIMIT - 5) as usize;

        let addresses: Vec<AddressInfo> = self
            .bdk
            .list_unused_addresses(KeychainKind::External)
            .take(MAX_ADDRESSES)
            .map(Into::into)
            .collect();

        // get up to 25 revealed but unused addresses
        if addresses.len() < MAX_ADDRESSES {
            let address_info =
                AddressInfo::from(self.bdk.reveal_next_address(KeychainKind::External));

            self.persist()?;

            let derivation_path =
                self.bdk.public_descriptor(KeychainKind::External).derivation_path().ok();
            let info = AddressInfoWithDerivation::new(address_info, derivation_path);
            return Ok(info);
        }

        // if we have already revealed 25 addresses, we cycle back to the first one
        // and present those addresses, until a next unused address is available, if we don't
        // do this we could hit the gap limit and users might use a an adddress past
        // the gap limit and not be able to see it their wallet
        //
        // note: index to use is the index of the address in the list of addresses, not the derivation index
        let index_to_use =
            if let Some(last_index) = self.metadata.internal.last_seen_address_index(&addresses) {
                (last_index + 1) % MAX_ADDRESSES
            } else {
                0
            };

        let address_info = addresses[index_to_use].clone();
        self.metadata.internal.set_last_seen_address_index(&addresses, index_to_use);

        Database::global().wallets.update_internal_metadata(&self.metadata)?;

        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();
        let address_info_with_derivation =
            AddressInfoWithDerivation::new(address_info, derivation_path);

        Ok(address_info_with_derivation)
    }

    pub fn persist(&mut self) -> Result<(), WalletError> {
        self.bdk.persist(&mut self.db.lock()).map_err_str(WalletError::PersistError)?;

        Ok(())
    }
}

fn check_for_duplicate_wallet(
    network: Network,
    mode: metadata::WalletMode,
    fingerprint: Fingerprint,
) -> Result<(), WalletError> {
    let all_fingerprints: Vec<(WalletId, Arc<Fingerprint>)> = Database::global()
        .wallets
        .get_all(network, mode)
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

    if let Some((id, _)) = all_fingerprints.into_iter().find(|(_, f)| f.as_ref() == &fingerprint) {
        return Err(WalletError::WalletAlreadyExists(id));
    }

    Ok(())
}

#[uniffi::export]
impl Wallet {
    // Create a dummy wallet for xcode previews
    #[uniffi::constructor(name = "previewNewWallet")]
    pub fn preview_new_wallet() -> Self {
        let mnemonic = Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();
        let passphrase = None;
        let metadata = WalletMetadata::preview_new();

        if let Err(error) = delete_wallet_specific_data(&metadata.id) {
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

impl WalletAddressType {
    pub fn index(&self) -> usize {
        match self {
            WalletAddressType::NativeSegwit => 0,
            WalletAddressType::WrappedSegwit => 1,
            WalletAddressType::Legacy => 2,
        }
    }
}

// delete wallet filestore / sqlite store and wallet data database
pub fn delete_wallet_specific_data(wallet_id: &WalletId) -> eyre::Result<()> {
    BdkStore::delete_wallet_stores(wallet_id)?;
    crate::database::wallet_data::delete_database(wallet_id)
        .context("unable to delete wallet data database")?;

    Ok(())
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

        let fingerprint = wallet.metadata.master_fingerprint.as_ref().unwrap().as_lowercase();

        let _ = delete_wallet_specific_data(&metadata.id);
        assert_eq!("73c5da0a", fingerprint.as_str());
    }
}

#[uniffi::export]
fn describe_wallet_error(error: WalletError) -> String {
    error.to_string()
}

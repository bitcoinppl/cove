pub mod amount_display;
pub mod balance;
pub mod ffi;
pub mod fingerprint;
pub mod metadata;

use std::{str::FromStr as _, sync::Arc};

use crate::{
    app::reconcile::{Update, Updater},
    bdk_store::BdkStore,
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    keys::{Descriptor, Descriptors},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    mnemonic::MnemonicExt as _,
    multi_format::MultiFormatError,
    tap_card::tap_signer_reader::DeriveInfo,
    wallet_identity::{PublicWalletIdentity, existing_public_wallet_by_identity_strict},
    xpub::{self, XpubError},
};
use balance::Balance;
use bdk_wallet::KeychainKind;
use bdk_wallet::bitcoin::bip32::Xpub;
use bdk_wallet::chain::rusqlite::Connection;
use bdk_wallet::chain::spk_client::FullScanRequest;
use bdk_wallet::miniscript::descriptor::ShInner;
use bip39::Mnemonic;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_common::consts::GAP_LIMIT;
use cove_types::{Network, address::AddressInfoWithDerivation};
use cove_util::result_ext::ResultExt as _;
use eyre::Context as _;
use fingerprint::Fingerprint;
use metadata::{
    DiscoveryState, HardwareWalletMetadata, WalletBirthday, WalletId, WalletMetadata, WalletType,
    tap_signer_import_birthday,
};
use parking_lot::Mutex;
use pubport::formats::Format;
use tracing::{debug, error, warn};

pub use cove_types::address;

pub type Address = address::Address;
pub type AddressWithNetwork = address::AddressWithNetwork;
pub type AddressInfo = address::AddressInfo;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
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
    Keychain(#[from] KeychainError),

    #[error("failed to save in database: {0}")]
    Database(#[from] database::Error),

    #[error("wallet not found")]
    WalletNotFound,

    #[error("metadata not found")]
    MetadataNotFound,

    #[error("failed to parse xpub: {0}")]
    ParseXpubError(#[from] XpubError),

    #[error("trying to import a wallet that already exists")]
    WalletAlreadyExists(WalletId),

    #[error(transparent)]
    MultiFormat(#[from] MultiFormatError),

    #[error("failed to parse descriptor: {0}")]
    DescriptorKeyParseError(String),
}

#[derive(Debug, uniffi::Object)]
pub struct Wallet {
    pub id: WalletId,
    pub network: Network,
    pub bdk: bdk_wallet::PersistedWallet<Connection>,
    pub metadata: WalletMetadata,
    // BDK's PersistedWallet<P> takes &mut P by reference on persist/load/create,
    // it doesn't hold the connection itself
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
#[uniffi::export(Display)]
pub enum WalletAddressType {
    #[default]
    #[display("Native Segwit")]
    NativeSegwit,
    #[display("Wrapped Segwit")]
    WrappedSegwit,
    #[display("Legacy")]
    Legacy,
}

impl WalletAddressType {
    /// returns the sort order for this address type
    /// native segwit (0) < wrapped segwit (1) < legacy (2)
    pub const fn sort_order(&self) -> u8 {
        match self {
            Self::NativeSegwit => 0,
            Self::WrappedSegwit => 1,
            Self::Legacy => 2,
        }
    }
}

#[uniffi::export]
impl WalletAddressType {
    #[uniffi::method(name = "sortOrder")]
    fn ffi_sort_order(&self) -> u8 {
        self.sort_order()
    }
}

impl Wallet {
    fn current_database_metadata(&self) -> Result<WalletMetadata, WalletError> {
        Database::global()
            .wallets
            .get(&self.id, self.network, self.metadata.wallet_mode)?
            .ok_or(WalletError::MetadataNotFound)
    }

    fn persist_address_type_switch_metadata(
        &mut self,
        metadata: WalletMetadata,
    ) -> Result<(), WalletError> {
        let metadata = Database::global().wallets.replace_wallet_metadata(metadata)?;

        self.metadata = metadata;

        Ok(())
    }

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
                }

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
            .get(&id, network, mode)?
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

        let pubport_descriptors = match pubport {
            Format::Descriptor(descriptors) => descriptors,
            Format::Json(json) => {
                let (descriptors, address_type) = preferred_json_descriptors(&json)?;

                if should_start_json_discovery(&json, address_type) {
                    metadata.discovery_state =
                        DiscoveryState::StartedJson(Arc::new((*json).into()));
                }

                descriptors
            }
            Format::Wasabi(descriptors) => descriptors,
            Format::Electrum(descriptors) => descriptors,
            Format::KeyExpression(descriptors) => descriptors,
        };

        // compute xpub and descriptors early so they're available for the upgrade path
        let descriptors: Descriptors = pubport_descriptors.into();
        metadata.address_type = address_type_from_descriptors(&descriptors)?;
        let fingerprint = descriptors.fingerprint();
        let xpub = xpub_from_descriptors(&descriptors)?;

        let incoming_identity = PublicWalletIdentity::from_descriptors(&descriptors);

        // check for existing wallet with same public identity, upgrade watch-only to cold
        if let Some(fingerprint) = fingerprint.as_ref() {
            let fingerprint: Fingerprint = (*fingerprint).into();
            metadata.master_fingerprint = Some(fingerprint.into());

            let existing = existing_public_wallet_by_identity_strict(
                &database,
                keychain,
                network,
                mode,
                fingerprint,
                &incoming_identity,
            )
            .map_err_str(WalletError::LoadError)?;

            if let Some(existing_metadata) = existing {
                return Self::upgrade_to_cold(
                    existing_metadata,
                    &metadata,
                    xpub,
                    descriptors,
                    keychain,
                    &database,
                );
            }
        }

        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let fingerprint = fingerprint.map(|s| s.to_string());

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
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

        Ok(Self { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    pub fn try_new_persisted_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive: DeriveInfo,
        backup: Option<Vec<u8>>,
        birthday: Option<WalletBirthday>,
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
        metadata.birthday =
            birthday.or_else(|| tap_signer_import_birthday(network, derive.birth_height));

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
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

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
        self.db = Mutex::new(db);
        let metadata = self.current_database_metadata()?;
        let metadata = metadata_for_address_type_switch(metadata, address_type);
        self.persist_address_type_switch_metadata(metadata)?;

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

        let metadata_for_new_wallet = self.current_database_metadata()?;
        let mut me = Self::try_new_persisted_from_mnemonic(
            metadata_for_new_wallet,
            mnemonic,
            None,
            address_type,
        )?;
        let current_metadata = self.current_database_metadata()?;
        let metadata =
            metadata_for_mnemonic_address_type_switch(current_metadata, &me.metadata, address_type);

        // swap the wallet to the new one
        std::mem::swap(&mut me, self);
        self.persist_address_type_switch_metadata(metadata)?;

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

    pub(crate) fn start_receive_prioritized_full_scan(&self) -> FullScanRequest<KeychainKind> {
        receive_prioritized_full_scan_request(&self.bdk)
    }

    /// Read cached transactions from the BDK wallet
    pub fn transactions(&self) -> Vec<crate::transaction::Transaction> {
        use crate::transaction::{Amount, Transaction};

        let zero = Amount::ZERO;

        let mut transactions = self
            .bdk
            .transactions()
            .map(|tx| {
                let sent_and_received = self.bdk.sent_and_received(&tx.tx_node.tx).into();
                (tx, sent_and_received)
            })
            .map(|(tx, sent_and_received)| Transaction::new(&self.id, sent_and_received, tx))
            .filter(|tx| tx.sent_and_received().amount() > zero)
            .collect::<Vec<Transaction>>();

        transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());
        transactions
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

    pub fn receive_address_at_index(&self, index: u32) -> AddressInfoWithDerivation {
        let address_info = AddressInfo::from(self.bdk.peek_address(KeychainKind::External, index));
        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();

        AddressInfoWithDerivation::new(address_info, derivation_path)
    }

    pub fn receive_address_is_unused(&self, index: u32) -> bool {
        self.bdk.list_unused_addresses(KeychainKind::External).any(|address| address.index == index)
    }

    pub fn mark_receive_address_used(&mut self, index: u32) -> Result<(), WalletError> {
        if self.bdk.mark_used(KeychainKind::External, index) {
            self.persist()?;
        }

        Ok(())
    }

    pub fn persist(&mut self) -> Result<(), WalletError> {
        self.bdk.persist(&mut self.db.lock()).map_err_str(WalletError::PersistError)?;

        Ok(())
    }

    pub fn unreserve_tx_change_addresses(&mut self, tx: &bdk_wallet::bitcoin::Transaction) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                self.bdk.derivation_of_spk(txout.script_pubkey.clone())
            {
                self.bdk.unmark_used(KeychainKind::Internal, index);
            }
        }
    }

    /// Upgrade an existing watch-only wallet to cold by saving the xpub and descriptors
    fn upgrade_to_cold(
        mut metadata: WalletMetadata,
        import_metadata: &WalletMetadata,
        xpub: Xpub,
        descriptors: Descriptors,
        keychain: &Keychain,
        database: &Database,
    ) -> Result<Self, WalletError> {
        if metadata.wallet_type != WalletType::WatchOnly {
            return Err(WalletError::WalletAlreadyExists(metadata.id));
        }

        let id = metadata.id.clone();
        keychain.save_wallet_xpub(&id, xpub)?;

        metadata = metadata_for_cold_upgrade(metadata, import_metadata, &descriptors);

        keychain.save_public_descriptor(
            &id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        database.wallets.update_wallet_metadata(metadata)?;
        database.global_config.select_wallet(id.clone())?;

        Updater::send_update(Update::ClearCachedWalletManager(id.clone()));
        CLOUD_BACKUP_MANAGER.handle_wallet_backup_change_and_reverify(id.clone());
        Self::try_load_persisted(id)
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

fn metadata_for_cold_upgrade(
    mut existing_metadata: WalletMetadata,
    import_metadata: &WalletMetadata,
    descriptors: &Descriptors,
) -> WalletMetadata {
    existing_metadata.wallet_type = WalletType::Cold;
    existing_metadata.origin = descriptors.origin().ok();
    existing_metadata.address_type = import_metadata.address_type;

    if import_metadata.discovery_state != DiscoveryState::Single {
        existing_metadata.discovery_state = import_metadata.discovery_state.clone();
    }

    existing_metadata
}

/// Builds an incremental scan request that checks revealed-unused receive addresses first
///
/// The request still uses unbounded BDK SPK iterators. The progressive scanner owns stop-gap
/// enforcement, so the normal external iterator resumes from index `0` with prioritized indexes
/// filtered out instead of being capped to the gap limit
fn receive_prioritized_full_scan_request(
    wallet: &bdk_wallet::Wallet,
) -> FullScanRequest<KeychainKind> {
    let mut builder = FullScanRequest::builder().chain_tip(wallet.local_chain().tip());

    let priority_spks = wallet
        .list_unused_addresses(KeychainKind::External)
        .take(GAP_LIMIT as usize)
        .map(|address| (address.index, address.address.script_pubkey()))
        .collect::<Vec<_>>();

    let priority_indices = priority_spks.iter().map(|(index, _)| *index).collect::<Vec<_>>();

    if let Some(external_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::External) {
        let external_spks = priority_spks
            .into_iter()
            .chain(external_spks.filter(move |(index, _)| !priority_indices.contains(index)));

        builder = builder.spks_for_keychain(KeychainKind::External, external_spks);
    }

    if let Some(internal_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::Internal) {
        builder = builder.spks_for_keychain(KeychainKind::Internal, internal_spks);
    }

    builder.build()
}

/// Selects the JSON descriptor set that becomes the imported wallet
///
/// A JSON export can carry several standard account types. Cove imports the most modern
/// supported primary wallet and lets discovery scan any supported older alternates separately
fn preferred_json_descriptors(
    json: &pubport::formats::Json,
) -> Result<(pubport::descriptor::Descriptors, WalletAddressType), WalletError> {
    [
        (&json.bip84, WalletAddressType::NativeSegwit),
        (&json.bip49, WalletAddressType::WrappedSegwit),
        (&json.bip44, WalletAddressType::Legacy),
    ]
    .into_iter()
    .find_map(|(descriptors, address_type)| {
        descriptors.as_ref().map(|descriptors| (descriptors.clone(), address_type))
    })
    .ok_or_else(|| {
        WalletError::ParseXpubError(xpub::XpubError::MissingXpub(
            "No supported BIP44, BIP49, or BIP84 xpub found".to_string(),
        ))
    })
}

fn xpub_from_descriptors(descriptors: &Descriptors) -> Result<Xpub, WalletError> {
    descriptors.external.xpub().ok_or(WalletError::ParseXpubError(XpubError::InvalidDescriptor(
        xpub::DescriptorError::NoXpubInDescriptor,
    )))
}

fn address_type_from_descriptors(
    descriptors: &Descriptors,
) -> Result<WalletAddressType, WalletError> {
    let external = address_type_from_descriptor(&descriptors.external)?;
    let internal = address_type_from_descriptor(&descriptors.internal)?;

    if external != internal {
        return Err(WalletError::UnsupportedWallet(
            "external and internal descriptors use different address types".to_string(),
        ));
    }

    Ok(external)
}

fn address_type_from_descriptor(descriptor: &Descriptor) -> Result<WalletAddressType, WalletError> {
    match &descriptor.extended_descriptor {
        bdk_wallet::miniscript::Descriptor::Pkh(_) => Ok(WalletAddressType::Legacy),
        bdk_wallet::miniscript::Descriptor::Wpkh(_) => Ok(WalletAddressType::NativeSegwit),
        bdk_wallet::miniscript::Descriptor::Sh(sh) => match sh.as_inner() {
            ShInner::Wpkh(_) => Ok(WalletAddressType::WrappedSegwit),
            _ => Err(unsupported_descriptor_address_type("non-wrapped-SegWit P2SH")),
        },
        bdk_wallet::miniscript::Descriptor::Tr(_) => {
            Err(unsupported_descriptor_address_type("Taproot"))
        }
        _ => Err(unsupported_descriptor_address_type("this descriptor type")),
    }
}

fn unsupported_descriptor_address_type(name: &str) -> WalletError {
    WalletError::UnsupportedWallet(format!("{name} descriptors are not supported"))
}

/// Returns whether a JSON import needs alternate address-type discovery
///
/// Discovery only starts when the export has a supported alternate that differs from the
/// primary wallet, because the primary descriptor is synced by the main wallet
///
/// Native SegWit (`bip84`) is intentionally absent because it is the preferred primary
/// descriptor when present, not an alternate discovery target
fn should_start_json_discovery(
    json: &pubport::formats::Json,
    address_type: WalletAddressType,
) -> bool {
    [(&json.bip49, WalletAddressType::WrappedSegwit), (&json.bip44, WalletAddressType::Legacy)]
        .into_iter()
        .any(|(descriptors, type_)| descriptors.is_some() && type_ != address_type)
}

impl Wallet {
    pub(crate) fn preview_new_wallet_with_metadata(metadata: WalletMetadata) -> Self {
        let mnemonic = Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();
        let passphrase = None;

        if let Err(error) = delete_wallet_specific_data(&metadata.id) {
            debug!("clean up failed, failed to delete wallet data: {error}");
        }

        if let Err(error) = Database::global().wallets.delete(&metadata.id) {
            debug!("clean up failed, failed to delete wallet: {error}");
        }

        Self::try_new_persisted_from_mnemonic_segwit(metadata, mnemonic, passphrase).unwrap()
    }
}

#[uniffi::export]
impl Wallet {
    // Create a dummy wallet for xcode previews
    #[uniffi::constructor(name = "previewNewWallet")]
    pub fn preview_new_wallet() -> Self {
        let metadata = WalletMetadata::preview_new();
        Self::preview_new_wallet_with_metadata(metadata)
    }

    pub fn id(&self) -> WalletId {
        self.id.clone()
    }
}

impl WalletAddressType {
    pub const fn index(&self) -> usize {
        match self {
            Self::NativeSegwit => 0,
            Self::WrappedSegwit => 1,
            Self::Legacy => 2,
        }
    }
}

fn metadata_for_address_type_switch(
    mut metadata: WalletMetadata,
    address_type: WalletAddressType,
) -> WalletMetadata {
    metadata.address_type = address_type;
    metadata.discovery_state = DiscoveryState::ChoseAdressType;
    metadata.internal.reset_scan_state_for_address_type_switch();
    metadata
}

fn metadata_for_mnemonic_address_type_switch(
    current_metadata: WalletMetadata,
    derived_metadata: &WalletMetadata,
    address_type: WalletAddressType,
) -> WalletMetadata {
    let mut metadata = metadata_for_address_type_switch(current_metadata, address_type);
    metadata.master_fingerprint = derived_metadata.master_fingerprint.clone();
    metadata.origin = derived_metadata.origin.clone();
    metadata
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

    use bdk_wallet::bitcoin::{
        Address as BdkAddress, Amount, BlockHash, Network, hashes::Hash as _,
    };
    use bdk_wallet::chain::{BlockId, ConfirmationBlockTime};
    use bdk_wallet::test_utils::{
        get_funded_wallet_wpkh, get_test_wpkh_and_change_desc, insert_anchor, insert_checkpoint,
        insert_tx,
    };

    const BIP49_YPUB: &str = "ypub6Ww3ibxVfGzLrAH1PNcjyAWenMTbbAosGNB6VvmSEgytSER9azLDWCxoJwW7Ke7icmizBMXrzBx9979FfaHxHcrArf3zbeJJJUZPf663zsP";
    const BIP84_ZPUB: &str = "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs";

    fn test_bdk_wallet() -> bdk_wallet::Wallet {
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();

        bdk_wallet::Wallet::create(external_descriptor, internal_descriptor)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .expect("wallet is created")
    }

    fn pubport_descriptors(descriptor: &str) -> pubport::descriptor::Descriptors {
        pubport::descriptor::Descriptors::try_from_line(descriptor)
            .expect("descriptor fixture is valid")
    }

    fn descriptor_json(
        bip44: Option<pubport::descriptor::Descriptors>,
        bip49: Option<pubport::descriptor::Descriptors>,
        bip84: Option<pubport::descriptor::Descriptors>,
    ) -> pubport::formats::Json {
        pubport::formats::Json { bip44, bip49, bip84, bip86: None }
    }

    fn json_from_export(export: &str) -> pubport::formats::Json {
        let format = pubport::Format::try_new_from_str(export).expect("export fixture is valid");
        let pubport::Format::Json(json) = format else {
            panic!("Expected JSON export");
        };

        *json
    }

    fn bip44_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "pkh([817e7be0/44h/0h/0h]xpub6BoKN14JzSFN1T3cqe9FnrwnXGAsmbgETJyeazoa3F7aMXh4XndvVrJAYyM127FsrH8KFv5XFXDroqXNfZMfsinow7xp93ueYSpnrjBBFs4/<0;1>/*)#tdtrl3y9",
        )
    }

    fn bip49_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "sh(wpkh([817e7be0/49h/0h/0h]xpub6CCKAvUTNursEnaJ8k1d27LfqEUzeAx2N9wFqYE3W1xh7nqgJEBEbLSSmohwDxzsSvcsYqiQqFzRvta65Njbe5o84bF5YXHFqfSH2Dkhonm/<0;1>/*))#8llmt36x",
        )
    }

    fn bip84_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7",
        )
    }

    fn bip86_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "tr([817e7be0/86h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)",
        )
    }

    fn assert_missing_supported_xpub(error: WalletError) {
        let WalletError::ParseXpubError(xpub::XpubError::MissingXpub(message)) = error else {
            panic!("expected missing xpub error");
        };

        assert_eq!(message, "No supported BIP44, BIP49, or BIP84 xpub found");
    }

    #[test]
    fn preferred_json_descriptors_uses_native_segwit_when_present() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(descriptors.external.to_string().starts_with("wpkh("));
    }

    #[test]
    fn preferred_json_descriptors_falls_back_to_wrapped_segwit() {
        let json = descriptor_json(Some(bip44_descriptors()), Some(bip49_descriptors()), None);

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(descriptors.external.to_string().starts_with("sh(wpkh("));
    }

    #[test]
    fn preferred_json_descriptors_falls_back_to_legacy() {
        let json = descriptor_json(Some(bip44_descriptors()), None, None);

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::Legacy);
        assert!(descriptors.external.to_string().starts_with("pkh("));
    }

    #[test]
    fn preferred_json_descriptors_errors_without_supported_xpub() {
        let json = descriptor_json(None, None, None);
        let error = preferred_json_descriptors(&json).unwrap_err();

        assert_missing_supported_xpub(error);
    }

    #[test]
    fn preferred_json_descriptors_rejects_bip86_only_export() {
        let json = json_from_export(
            r#"{
  "xfp": "817e7be0",
  "bip86": {
    "deriv": "m/86'/0'/0'",
    "xpub": "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    "first": "bc1p5cyxnuxmeuwuvkwfem96l9z7k3d5en0fhzc3wkvsgq4wv5q3xpqsv0gz6u"
  }
}"#,
        );
        let error = preferred_json_descriptors(&json).unwrap_err();

        assert_missing_supported_xpub(error);
    }

    #[test]
    fn bare_ypub_import_selects_wrapped_segwit_descriptor() {
        let json = json_from_export(BIP49_YPUB);

        assert!(json.bip49.is_some());
        assert!(json.bip84.is_none());

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(descriptors.external.to_string().starts_with("sh(wpkh("));
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn wrapped_segwit_json_import_material_exposes_nested_xpub() {
        let json = descriptor_json(None, Some(bip49_descriptors()), None);

        let (pubport_descriptors, address_type) = preferred_json_descriptors(&json).unwrap();
        let descriptors: Descriptors = pubport_descriptors.into();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert_eq!(
            descriptors.fingerprint().map(|fingerprint| fingerprint.to_string()),
            Some("817e7be0".to_string())
        );
        assert_eq!(
            xpub_from_descriptors(&descriptors).unwrap().to_string(),
            "xpub6CCKAvUTNursEnaJ8k1d27LfqEUzeAx2N9wFqYE3W1xh7nqgJEBEbLSSmohwDxzsSvcsYqiQqFzRvta65Njbe5o84bF5YXHFqfSH2Dkhonm"
        );
    }

    #[test]
    fn converted_json_import_material_matches_pubport_for_primary_descriptors() {
        for pubport_descriptors in [bip84_descriptors(), bip44_descriptors()] {
            let descriptors: Descriptors = pubport_descriptors.clone().into();

            assert_eq!(descriptors.fingerprint(), pubport_descriptors.fingerprint());
            assert_eq!(
                xpub_from_descriptors(&descriptors).unwrap(),
                pubport_descriptors.xpub().unwrap()
            );
        }
    }

    #[test]
    fn descriptor_address_type_infers_supported_descriptor_formats() {
        for (pubport_descriptors, expected) in [
            (bip84_descriptors(), WalletAddressType::NativeSegwit),
            (bip49_descriptors(), WalletAddressType::WrappedSegwit),
            (bip44_descriptors(), WalletAddressType::Legacy),
        ] {
            let descriptors = Descriptors::from(pubport_descriptors);

            assert_eq!(address_type_from_descriptors(&descriptors).unwrap(), expected);
        }
    }

    #[test]
    fn descriptor_address_type_rejects_taproot() {
        let descriptors = Descriptors::from(bip86_descriptors());
        let error = address_type_from_descriptors(&descriptors).unwrap_err();

        assert_eq!(
            error,
            WalletError::UnsupportedWallet("Taproot descriptors are not supported".to_string())
        );
    }

    #[test]
    fn bare_zpub_import_selects_native_segwit_descriptor() {
        let json = json_from_export(BIP84_ZPUB);

        assert!(json.bip84.is_some());
        assert!(json.bip49.is_none());

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(descriptors.external.to_string().starts_with("wpkh("));
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_starts_for_native_segwit_with_alternates() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_starts_for_wrapped_segwit_with_legacy_alternate() {
        let json = descriptor_json(Some(bip44_descriptors()), Some(bip49_descriptors()), None);

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_does_not_start_for_single_native_segwit_export() {
        let json = descriptor_json(None, None, Some(bip84_descriptors()));

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_does_not_start_for_wrapped_segwit_export() {
        let json = descriptor_json(None, Some(bip49_descriptors()), None);

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn cold_upgrade_metadata_carries_json_discovery_state() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );
        let descriptors = Descriptors::from(bip84_descriptors());

        let mut existing_metadata = WalletMetadata::preview_new();
        existing_metadata.name = "Existing watch-only wallet".to_string();
        existing_metadata.wallet_type = WalletType::WatchOnly;
        existing_metadata.discovery_state = DiscoveryState::Single;
        existing_metadata.address_type = WalletAddressType::Legacy;

        let mut import_metadata = existing_metadata.clone();
        import_metadata.name = "Incoming hardware wallet".to_string();
        import_metadata.wallet_type = WalletType::Cold;
        import_metadata.address_type = WalletAddressType::NativeSegwit;
        import_metadata.discovery_state =
            DiscoveryState::StartedJson(Arc::new(json.clone().into()));

        let updated =
            metadata_for_cold_upgrade(existing_metadata.clone(), &import_metadata, &descriptors);

        assert_eq!(updated.name, existing_metadata.name);
        assert_ne!(updated.name, import_metadata.name);
        assert_eq!(updated.wallet_type, WalletType::Cold);
        assert_eq!(updated.address_type, WalletAddressType::NativeSegwit);
        assert_eq!(updated.origin, descriptors.origin().ok());

        let DiscoveryState::StartedJson(found_json) = updated.discovery_state else {
            panic!("expected JSON discovery to start after cold upgrade");
        };

        assert_eq!(found_json.as_ref().0, json);
    }

    fn scan_indexes(
        request: &mut FullScanRequest<KeychainKind>,
        keychain: KeychainKind,
        count: usize,
    ) -> Vec<u32> {
        request.iter_spks(keychain).take(count).map(|(index, _)| index).collect()
    }

    fn build_tx_with_change(wallet: &mut bdk_wallet::Wallet) -> bdk_wallet::bitcoin::Psbt {
        let address = BdkAddress::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let mut builder = wallet.build_tx();
        builder.add_recipient(address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));
        builder.finish().unwrap()
    }

    fn tx_output_index(
        wallet: &bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
        keychain: KeychainKind,
    ) -> u32 {
        tx.output
            .iter()
            .find_map(|txout| match wallet.derivation_of_spk(txout.script_pubkey.clone()) {
                Some((txout_keychain, index)) if txout_keychain == keychain => Some(index),
                _ => None,
            })
            .unwrap()
    }

    fn unused_addresses_contain(
        wallet: &bdk_wallet::Wallet,
        keychain: KeychainKind,
        index: u32,
    ) -> bool {
        wallet.list_unused_addresses(keychain).any(|address| address.index == index)
    }

    fn unreserve_tx_change_addresses(
        wallet: &mut bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
    ) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                wallet.derivation_of_spk(txout.script_pubkey.clone())
            {
                wallet.unmark_used(KeychainKind::Internal, index);
            }
        }
    }

    #[test]
    fn unreserve_tx_change_addresses_releases_reserved_change_index() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_confirmed_change_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);
        let block_id = BlockId { height: 1, hash: BlockHash::hash(b"confirmed change") };
        let confirmation = ConfirmationBlockTime { block_id, confirmation_time: 1 };

        insert_checkpoint(&mut wallet, block_id);
        insert_tx(&mut wallet, psbt.unsigned_tx.clone());
        insert_anchor(&mut wallet, psbt.unsigned_tx.compute_txid(), confirmation);

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_self_send_receive_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let receive_address = wallet.reveal_next_address(KeychainKind::External);

        assert!(wallet.mark_used(KeychainKind::External, receive_address.index));

        let mut builder = wallet.build_tx();
        builder.add_recipient(receive_address.address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));

        let psbt = builder.finish().unwrap();
        let receive_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::External);

        assert_eq!(receive_address.index, receive_index);
        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));
    }

    #[test]
    fn receive_prioritized_scan_checks_revealed_unused_external_indexes_first() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 7);

        assert_eq!(indexes, vec![1, 3, 4, 0, 2, 5, 6]);
    }

    #[test]
    fn receive_prioritized_scan_deduplicates_priority_indexes_from_normal_external_scan() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 10);
        let unique_indexes = indexes.iter().copied().collect::<std::collections::BTreeSet<_>>();

        assert_eq!(indexes.len(), unique_indexes.len());
    }

    #[test]
    fn receive_prioritized_scan_prefix_is_capped_at_gap_limit() {
        let mut wallet = test_bdk_wallet();
        let gap_limit = u32::from(GAP_LIMIT);
        let _ = wallet.reveal_addresses_to(KeychainKind::External, gap_limit + 2).last();
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, GAP_LIMIT as usize + 2);
        let expected_prefix = (0..gap_limit).collect::<Vec<_>>();

        assert_eq!(&indexes[..GAP_LIMIT as usize], expected_prefix.as_slice());
        assert_eq!(indexes[GAP_LIMIT as usize], gap_limit);
    }

    #[test]
    fn receive_prioritized_scan_prefix_does_not_fill_with_unrevealed_external_indexes() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 4);

        assert_eq!(indexes, vec![1, 2, 0, 3]);
    }

    #[test]
    fn receive_prioritized_scan_keeps_internal_keychain_after_external_keychain() {
        let wallet = test_bdk_wallet();
        let request = receive_prioritized_full_scan_request(&wallet);

        assert_eq!(request.keychains(), vec![KeychainKind::External, KeychainKind::Internal]);
    }

    #[test]
    fn receive_prioritized_scan_construction_does_not_reveal_or_mark_addresses_used() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let last_revealed_before = wallet.spk_index().last_revealed_indices();
        let unused_before = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        let _request = receive_prioritized_full_scan_request(&wallet);

        let last_revealed_after = wallet.spk_index().last_revealed_indices();
        let unused_after = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        assert_eq!(last_revealed_after, last_revealed_before);
        assert_eq!(unused_after, unused_before);
    }

    #[test]
    fn address_type_switch_metadata_preserves_current_fields_and_resets_scan_fields() {
        let mut current_metadata = WalletMetadata::preview_new();
        current_metadata.name = "renamed while discovering".to_string();
        current_metadata.selected_unit = crate::transaction::Unit::Sat;
        current_metadata.sensitive_visible = false;
        current_metadata.details_expanded = true;
        current_metadata.show_labels = false;
        current_metadata.internal.address_index =
            Some(cove_types::AddressIndex { last_seen_index: 4, address_list_hash: 2 });
        current_metadata.internal.last_scan_finished = Some(std::time::Duration::from_secs(10));
        current_metadata.internal.last_height_fetched = Some(cove_types::BlockSizeLast {
            block_height: 1,
            last_seen: std::time::Duration::from_secs(20),
        });
        current_metadata.internal.performed_full_scan_at = Some(30);
        current_metadata.internal.store_type = metadata::StoreType::FileStore;

        let mut stale_actor_metadata = current_metadata.clone();
        stale_actor_metadata.name = "stale actor name".to_string();
        stale_actor_metadata.selected_unit = crate::transaction::Unit::Btc;
        stale_actor_metadata.sensitive_visible = true;
        stale_actor_metadata.details_expanded = false;
        stale_actor_metadata.show_labels = true;

        let updated =
            metadata_for_address_type_switch(current_metadata.clone(), WalletAddressType::Legacy);

        assert_eq!(updated.name, current_metadata.name);
        assert_eq!(updated.selected_unit, current_metadata.selected_unit);
        assert_eq!(updated.sensitive_visible, current_metadata.sensitive_visible);
        assert_eq!(updated.details_expanded, current_metadata.details_expanded);
        assert_eq!(updated.show_labels, current_metadata.show_labels);
        assert_ne!(updated.name, stale_actor_metadata.name);
        assert_ne!(updated.selected_unit, stale_actor_metadata.selected_unit);
        assert_ne!(updated.sensitive_visible, stale_actor_metadata.sensitive_visible);
        assert_ne!(updated.details_expanded, stale_actor_metadata.details_expanded);
        assert_ne!(updated.show_labels, stale_actor_metadata.show_labels);
        assert_eq!(updated.address_type, WalletAddressType::Legacy);
        assert_eq!(updated.discovery_state, DiscoveryState::ChoseAdressType);
        assert_eq!(updated.internal.address_index, None);
        assert_eq!(updated.internal.last_scan_finished, None);
        assert_eq!(updated.internal.last_height_fetched, None);
        assert_eq!(updated.internal.performed_full_scan_at, None);
        assert_eq!(updated.internal.store_type, metadata::StoreType::FileStore);
    }

    #[test]
    fn mnemonic_address_type_switch_metadata_keeps_new_derived_origin() {
        let mut current_metadata = WalletMetadata::preview_new();
        current_metadata.name = "current database name".to_string();
        current_metadata.origin = Some("wpkh([73c5da0a/84'/0'/0'])".to_string());
        current_metadata.internal.last_scan_finished = Some(std::time::Duration::from_secs(10));

        let mut derived_metadata = current_metadata.clone();
        derived_metadata.name =
            "derived metadata should not replace current database name".to_string();
        derived_metadata.origin = Some("pkh([73c5da0a/44'/0'/0'])".to_string());

        let updated = metadata_for_mnemonic_address_type_switch(
            current_metadata.clone(),
            &derived_metadata,
            WalletAddressType::Legacy,
        );

        assert_eq!(updated.name, current_metadata.name);
        assert_ne!(updated.name, derived_metadata.name);
        assert_eq!(updated.origin, derived_metadata.origin);
        assert_ne!(updated.origin, current_metadata.origin);
        assert_eq!(updated.address_type, WalletAddressType::Legacy);
        assert_eq!(updated.discovery_state, DiscoveryState::ChoseAdressType);
        assert_eq!(updated.internal.last_scan_finished, None);
    }

    #[test]
    fn test_fingerprint() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();

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
impl Wallet {
    #[uniffi::constructor]
    pub fn new_from_xpub(xpub: String) -> Result<Self, WalletError> {
        Self::try_new_persisted_from_xpub(xpub)
    }

    #[uniffi::constructor]
    pub fn new_from_export(
        export: Arc<crate::hardware_export::HardwareExport>,
    ) -> Result<Self, WalletError> {
        let export = Arc::unwrap_or_clone(export);
        Self::try_new_persisted_from_pubport(export.into_format())
    }
}

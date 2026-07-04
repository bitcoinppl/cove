pub(crate) mod addressing;
pub mod amount_display;
pub mod balance;
pub(crate) mod builder;
pub mod ffi;
pub mod fingerprint;
pub mod metadata;

use std::{str::FromStr as _, sync::Arc};

use crate::{
    bdk_store::BdkStore,
    database::{self, Database},
    keychain::KeychainError,
    keys::Descriptor,
    multi_format::MultiFormatError,
    tap_card::tap_signer_reader::DeriveInfo,
    xpub::XpubError,
};
use balance::Balance;
use bdk_wallet::KeychainKind;
use bdk_wallet::chain::rusqlite::Connection;
use bip39::Mnemonic;
use cove_types::Network;
use cove_util::result_ext::ResultExt as _;
use eyre::Context as _;
use metadata::{WalletBirthday, WalletId, WalletMetadata, WalletType};
use parking_lot::Mutex;
use tracing::{debug, warn};

use builder::{WalletBuilder, WalletSource};

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
        WalletBuilder::new(WalletSource::PersistedAndSelected { metadata, mnemonic, passphrase })
            .build()
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
        WalletBuilder::new(WalletSource::Xpub(xpub)).build()
    }

    /// Import from a hardware export
    pub fn try_new_persisted_from_pubport(pubport: pubport::Format) -> Result<Self, WalletError> {
        WalletBuilder::new(WalletSource::Pubport(Box::new(pubport))).build()
    }

    pub fn try_new_persisted_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive: DeriveInfo,
        backup: Option<Vec<u8>>,
        birthday: Option<WalletBirthday>,
    ) -> Result<Self, WalletError> {
        WalletBuilder::new(WalletSource::TapSigner { tap_signer, derive, backup, birthday }).build()
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
        WalletBuilder::new(WalletSource::Mnemonic { metadata, mnemonic, passphrase, address_type })
            .build()
    }

    pub fn balance(&self) -> Balance {
        self.bdk.balance().into()
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

    pub fn persist(&mut self) -> Result<(), WalletError> {
        self.bdk.persist(&mut self.db.lock()).map_err_str(WalletError::PersistError)?;

        Ok(())
    }
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

// delete wallet filestore / sqlite store and wallet data database
pub fn delete_wallet_specific_data(wallet_id: &WalletId) -> eyre::Result<()> {
    BdkStore::delete_wallet_stores(wallet_id)?;
    crate::database::wallet_data::delete_database(wallet_id)
        .context("unable to delete wallet data database")?;

    Ok(())
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

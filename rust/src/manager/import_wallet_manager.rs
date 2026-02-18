use std::sync::Arc;

use bip39::{Language, Mnemonic};
use cove_util::result_ext::ResultExt as _;
use flume::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    mnemonic::MnemonicExt as _,
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata, WalletType},
    },
};

use cove_macros::impl_default_for;
use tracing::{info, warn};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletManagerReconcileMessage {
    NoOp,
}

#[uniffi::export(callback_interface)]
pub trait ImportWalletManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: ImportWalletManagerReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustImportWalletManager {
    #[allow(dead_code)]
    pub state: Arc<RwLock<ImportWalletManagerState>>,
    #[allow(dead_code)]
    pub reconciler: Sender<ImportWalletManagerReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<ImportWalletManagerReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImportWalletManagerState {}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletManagerAction {
    NoOp,
}

impl_default_for!(RustImportWalletManager);

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum ImportWalletError {
    #[error("failed to import wallet: {0}")]
    WalletImportError(String),

    #[error("invalid word group: {0}")]
    InvalidWordGroup(String),

    #[error("failed to save wallet to keychain: {0}")]
    Keychain(#[from] KeychainError),

    #[error("wallet already exists")]
    WalletAlreadyExists(WalletId),

    #[error("wallet metadata missing for existing wallet")]
    MissingMetadata(WalletId),

    #[error("failed to save wallet: {0}")]
    Database(#[from] database::Error),

    #[error("failed to create wallet: {0}")]
    BdkError(String),
}

pub type Error = ImportWalletError;

#[uniffi::export]
impl RustImportWalletManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let (sender, receiver) = flume::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(ImportWalletManagerState::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn ImportWalletManagerReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                // call the reconcile method on the frontend
                reconciler.reconcile(field);
            }
        });
    }

    /// Import wallet view from entered words
    #[uniffi::method]
    pub fn import_wallet(&self, entered_words: Vec<Vec<String>>) -> Result<WalletMetadata, Error> {
        let words = entered_words.into_iter().flatten().collect::<Vec<String>>().join(" ");

        let mnemonic = Mnemonic::parse_in_normalized(Language::English, &words)
            .map_err_str(ImportWalletError::InvalidWordGroup)?;

        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        let fingerprint: Fingerprint = mnemonic.xpub(network.into()).fingerprint().into();

        // check if the wallet already exists using the fingerprint
        let existing_wallet = Database::global()
            .wallets
            .get_all(network, mode)
            .unwrap_or_default()
            .into_iter()
            .find(|wallet_metadata| wallet_metadata.matches_fingerprint(fingerprint));

        // new wallet, create it and return
        if existing_wallet.is_none() {
            // get current number of wallets and add one;
            let number_of_wallets = Database::global().wallets.len(network, mode).unwrap_or(0);

            let name = format!("Wallet {}", number_of_wallets + 1);
            let wallet_metadata =
                WalletMetadata::new_imported_from_mnemonic(name, network, fingerprint);

            Wallet::try_new_persisted_and_selected(wallet_metadata.clone(), mnemonic.clone(), None)
                .map_err_str(ImportWalletError::WalletImportError)?;

            return Ok(wallet_metadata);
        }

        // existing wallet
        let mut metadata = existing_wallet.expect("wallet exists, just checked above");
        let id = metadata.id.clone();
        let keychain = Keychain::global();

        // hot wallets with private key already in keychain, don't do anything else
        if metadata.wallet_type == WalletType::Hot && keychain.get_wallet_key(&id)?.is_some() {
            warn!(
                "attempted to import words for existing hot wallet {id}, showing duplicate alert"
            );

            Database::global().global_config.select_wallet(id.clone())?;
            return Err(ImportWalletError::WalletAlreadyExists(id));
        }

        info!("adding mnemonic to existing wallet {id}");

        // save the private key material for an existing wallet.
        keychain.save_wallet_key(&id, mnemonic.clone())?;

        // save xpub/descriptors in keychain too
        let xpub = mnemonic.xpub(network.into());
        keychain.save_wallet_xpub(&id, xpub)?;

        // save public descriptors in keychain too
        let descriptors = mnemonic.clone().into_descriptors(None, network, metadata.address_type);
        keychain.save_public_descriptor(
            &id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        // imported mnemonic means this wallet can now sign locally.
        metadata.wallet_type = WalletType::Hot;
        metadata.hardware_metadata = None;
        metadata.verified = true;

        Database::global().wallets.update_wallet_metadata(metadata.clone())?;
        Database::global().global_config.select_wallet(id)?;

        Ok(metadata)
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub const fn dispatch(&self, action: ImportWalletManagerAction) {
        match action {
            ImportWalletManagerAction::NoOp => {}
        }
    }
}

impl_default_for!(ImportWalletManagerState);
impl ImportWalletManagerState {
    pub const fn new() -> Self {
        Self {}
    }
}

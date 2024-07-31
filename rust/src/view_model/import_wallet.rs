use std::sync::Arc;

use bip39::{Language, Mnemonic};
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{
    database::{self, Database},
    impl_default_for,
    keychain::KeychainError,
    mnemonic::MnemonicExt as _,
    wallet::{
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata},
        Wallet,
    },
};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletViewModelReconcileMessage {
    NoOp,
}

#[uniffi::export(callback_interface)]
pub trait ImportWalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: ImportWalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustImportWalletViewModel {
    pub state: Arc<RwLock<ImportWalletViewModelState>>,
    pub reconciler: Sender<ImportWalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<ImportWalletViewModelReconcileMessage>>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImportWalletViewModelState {}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum ImportWalletViewModelAction {
    NoOp,
}

impl_default_for!(RustImportWalletViewModel);

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum ImportWalletError {
    #[error("failed to import wallet: {0}")]
    WalletImportError(String),

    #[error("invalid word group: {0}")]
    InvalidWordGroup(String),

    #[error("failed to save wallet to keychain: {0}")]
    KeychainError(#[from] KeychainError),

    #[error("wallet already exists")]
    WalletAlreadyExists(WalletId),

    #[error("failed to save wallet: {0}")]
    DatabaseError(#[from] database::Error),

    #[error("failed to create wallet: {0}")]
    BdkError(String),
}

pub type Error = ImportWalletError;

#[uniffi::export]
impl RustImportWalletViewModel {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(ImportWalletViewModelState::new())),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn ImportWalletViewModelReconciler>) {
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
        let words = entered_words
            .into_iter()
            .flatten()
            .collect::<Vec<String>>()
            .join(" ");

        let mnemonic = Mnemonic::parse_in_normalized(Language::English, &words)
            .map_err(|e| ImportWalletError::InvalidWordGroup(e.to_string()))?;

        let network = Database::global().global_config.selected_network();

        // make sure its not already imported
        let fingerprint: Fingerprint = mnemonic.xpub(network.into()).fingerprint().into();
        let all_fingerprints: Vec<(WalletId, Fingerprint)> = Database::global()
            .wallets
            .get_all(network)
            .map(|wallets| {
                wallets
                    .into_iter()
                    .filter_map(|wallet_metadata| {
                        let fingerprint = Fingerprint::try_new(&wallet_metadata.id).ok()?;

                        Some((wallet_metadata.id, fingerprint))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if let Some((id, _)) = all_fingerprints
            .into_iter()
            .find(|(_, f)| f == &fingerprint)
        {
            return Err(ImportWalletError::WalletAlreadyExists(id));
        }

        // get current number of wallets and add one;
        let number_of_wallets = Database::global().wallets.len(network).unwrap_or(0);

        let name = format!("Wallet {}", number_of_wallets + 1);
        let wallet_metadata = WalletMetadata::new_imported(name, network);

        Wallet::try_new_persisted_and_selected(wallet_metadata.clone(), mnemonic.clone(), None)
            .map_err(|e| ImportWalletError::WalletImportError(e.to_string()))?;

        Ok(wallet_metadata)
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: ImportWalletViewModelAction) {
        match action {
            ImportWalletViewModelAction::NoOp => {}
        }
    }
}

impl_default_for!(ImportWalletViewModelState);
impl ImportWalletViewModelState {
    pub fn new() -> Self {
        Self {}
    }
}

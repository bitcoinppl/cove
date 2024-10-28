use std::sync::Arc;

use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;
use tracing::error;

use crate::{
    database::{self, Database},
    keychain::KeychainError,
    mnemonic::{GroupedWord, MnemonicExt as _, NumberOfBip39Words, WordAccess as _},
    pending_wallet::PendingWallet,
    wallet::{fingerprint::Fingerprint, metadata::WalletMetadata, Wallet},
};

type Error = PendingWalletViewModelError;

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum PendingWalletViewModelError {
    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("failed to save wallet to keychain: {0}")]
    WalletCreationError(#[from] WalletCreationError),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PendingWalletViewModelReconcileMessage {
    Words(NumberOfBip39Words),
}

#[uniffi::export(callback_interface)]
pub trait PendingWalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: PendingWalletViewModelReconcileMessage);
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct RustPendingWalletViewModel {
    pub state: Arc<RwLock<PendingWalletViewModelState>>,
    pub reconciler: Sender<PendingWalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<PendingWalletViewModelReconcileMessage>>,
}

#[derive(Debug, Clone, uniffi::Record)]

pub struct PendingWalletViewModelState {
    pub number_of_words: NumberOfBip39Words,
    pub wallet: Arc<PendingWallet>,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PendingWalletViewModelAction {
    UpdateWords(NumberOfBip39Words),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletCreationError {
    #[error("failed to create wallet: {0}")]
    Bdk(String),

    #[error("failed to save wallet to keychain: {0}")]
    Keychain(#[from] KeychainError),

    #[error("failed to save wallet: {0}")]
    Database(#[from] database::Error),

    #[error("persist error: {0}")]
    Persist(String),

    #[error("failed to import hardware wallet: {0}")]
    Import(String),
}

#[uniffi::export]
impl RustPendingWalletViewModel {
    #[uniffi::constructor]
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(PendingWalletViewModelState::new(
                number_of_words,
            ))),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn get_state(&self) -> PendingWalletViewModelState {
        self.state.read().clone()
    }

    #[uniffi::method]
    pub fn number_of_words_count(&self) -> u8 {
        self.state.read().number_of_words.to_word_count() as u8
    }

    #[uniffi::method]
    pub fn bip_39_words(&self) -> Vec<String> {
        self.state.read().wallet.words()
    }

    #[uniffi::method]
    pub fn card_indexes(&self) -> u8 {
        self.state.read().number_of_words.to_word_count() as u8 / 6
    }

    #[uniffi::method]
    pub fn save_wallet(&self) -> Result<WalletMetadata, Error> {
        let network = self.state.read().wallet.network;

        // get current number of wallets and add one;
        let number_of_wallets = Database::global().wallets.len(network).unwrap_or(0);

        let name = format!("Wallet {}", number_of_wallets + 1);
        let fingerprint: Fingerprint = self
            .state
            .read()
            .wallet
            .mnemonic
            .xpub(network.into())
            .fingerprint()
            .into();

        let wallet_metadata = WalletMetadata::new(name, fingerprint);

        // create, persist and select the wallet
        Wallet::try_new_persisted_and_selected(
            wallet_metadata.clone(),
            self.state.read().wallet.mnemonic.clone(),
            None,
        )?;

        Ok(wallet_metadata)
    }

    #[uniffi::method]
    pub fn bip_39_words_grouped(&self) -> Vec<Vec<GroupedWord>> {
        self.state.read().wallet.mnemonic.grouped_words_of(6)
    }

    // boilerplate methods
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn PendingWalletViewModelReconciler>) {
        let reconcile_receiver = self.reconcile_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = reconcile_receiver.recv() {
                // call the reconcile method on the frontend
                reconciler.reconcile(field);
            }
        });
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: PendingWalletViewModelAction) {
        match action {
            PendingWalletViewModelAction::UpdateWords(words) => {
                {
                    let mut state = self.state.write();
                    state.wallet = PendingWallet::new(words, None).into();
                    state.number_of_words = words;
                }

                self.reconciler
                    .send(PendingWalletViewModelReconcileMessage::Words(words))
                    .expect("failed to send update");
            }
        }
    }
}

impl PendingWalletViewModelState {
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        Self {
            number_of_words,
            wallet: PendingWallet::new(number_of_words, None).into(),
        }
    }
}

impl From<crate::wallet::WalletError> for PendingWalletViewModelError {
    fn from(error: crate::wallet::WalletError) -> Self {
        WalletCreationError::from(error).into()
    }
}

impl From<crate::wallet::WalletError> for WalletCreationError {
    fn from(error: crate::wallet::WalletError) -> Self {
        use crate::wallet::WalletError;

        match error {
            WalletError::KeychainError(error) => Self::Keychain(error),
            WalletError::DatabaseError(error) => Self::Database(error),
            WalletError::BdkError(error) => Self::Bdk(error),
            WalletError::PersistError(error) => Self::Persist(error),
            WalletError::ParseXpubError(error) => Self::Import(error.to_string()),
            WalletError::WalletAlreadyExists(id) => {
                Self::Import(format!("wallet already exists: {id}"))
            }

            WalletError::WalletNotFound => unreachable!("no wallet found in creation"),
            WalletError::LoadError(error) => unreachable!("no loading in creation:{error}"),
            WalletError::MetadataNotFound => unreachable!("no metadata found in creation"),
            WalletError::UnsupportedWallet(error) => {
                unreachable!("unreachable unsupported wallet: {error}")
            }
        }
    }
}

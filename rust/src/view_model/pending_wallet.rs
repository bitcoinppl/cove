use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::{
    database::{Database, DatabaseError},
    keychain::{Keychain, KeychainError},
    wallet::{GroupedWord, NumberOfBip39Words, PendingWallet, WordAccess},
};

use super::wallet::WalletId;

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

#[derive(Debug)]
pub enum WalletState {
    Empty,
    Created(bdk_wallet::Wallet),
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
    BdkError(String),

    #[error("failed to save wallet to keychain: {0}")]
    KeychainError(#[from] KeychainError),

    #[error("failed to save wallet: {0}")]
    DatabaseError(#[from] DatabaseError),
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
    pub fn save_wallet(&self) -> Result<WalletId, Error> {
        let state = self.state.read();
        let wallet_id = WalletId::new();

        let keychain = Keychain::global();

        keychain
            .save_wallet_key(wallet_id.clone(), state.wallet.mnemonic.clone())
            .map_err(WalletCreationError::from)?;

        let database = Database::global();
        let mut wallets = database
            .wallets
            .get(state.wallet.network)
            .map_err(WalletCreationError::from)?;

        wallets.push(wallet_id.clone());

        database
            .wallets
            .save(state.wallet.network, wallets)
            .map_err(WalletCreationError::from)?;

        Ok(wallet_id)
    }

    #[uniffi::method]
    pub fn bip_39_words_grouped(&self) -> Vec<Vec<GroupedWord>> {
        self.state.read().wallet.mnemonic.bip_39_words_groups_of(6)
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
                    state.wallet = PendingWallet::new(words, Network::Bitcoin, None).into();
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
            wallet: PendingWallet::new(number_of_words, Network::Bitcoin, None).into(),
        }
    }
}

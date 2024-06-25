use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::wallet::{NumberOfBip39Words, Wallet};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelReconcileMessage {
    Words(NumberOfBip39Words),
}

#[derive(Debug)]
pub enum WalletState {
    Empty,
    Created(bdk_wallet::Wallet),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct GroupedWord {
    pub number: u8,
    pub word: String,
}

#[uniffi::export(callback_interface)]
pub trait WalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: WalletViewModelReconcileMessage);
}

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustWalletViewModel {
    pub state: Arc<RwLock<WalletViewModelState>>,
    pub reconciler: Sender<WalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<WalletViewModelReconcileMessage>>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletViewModelState {
    pub number_of_words: NumberOfBip39Words,
    pub wallet: Wallet,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletViewModelAction {
    UpdateWords(NumberOfBip39Words),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
pub enum WalletCreationError {
    #[error("failed to create wallet: {0}")]
    BdkError(String),
}

#[uniffi::export]
impl RustWalletViewModel {
    #[uniffi::constructor]
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        let (sender, receiver) = crossbeam::channel::bounded(1000);

        Self {
            state: Arc::new(RwLock::new(WalletViewModelState::new(number_of_words))),
            reconciler: sender,
            reconcile_receiver: Arc::new(receiver),
        }
    }

    #[uniffi::method]
    pub fn get_state(&self) -> WalletViewModelState {
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
    pub fn bip_39_words_grouped(&self) -> Vec<Vec<GroupedWord>> {
        let chunk_size = 6;

        self.state
            .read()
            .wallet
            .words()
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                chunk
                    .iter()
                    .enumerate()
                    .map(|(index, word)| GroupedWord {
                        number: ((chunk_index * chunk_size) + index + 1) as u8,
                        word: word.to_string(),
                    })
                    .collect()
            })
            .collect()
    }

    // check if the word group passed in is valid
    #[uniffi::method]
    pub fn is_valid_word_group(&self, group_number: u8, entered_words: Vec<String>) -> bool {
        let actual_words = &self.bip_39_words_grouped()[group_number as usize];

        for (actual_word, entered_word) in actual_words.iter().zip(entered_words.iter()) {
            if actual_word.word != entered_word.to_lowercase().trim() {
                return false;
            }
        }

        true
    }

    // boilerplate methods
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn WalletViewModelReconciler>) {
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
    pub fn dispatch(&self, action: WalletViewModelAction) {
        match action {
            WalletViewModelAction::UpdateWords(words) => {
                {
                    let mut state = self.state.write();
                    state.wallet = Wallet::new(words, Network::Bitcoin, None);
                    state.number_of_words = words;
                }

                self.reconciler
                    .send(WalletViewModelReconcileMessage::Words(words))
                    .expect("failed to send update");
            }
        }
    }
}

impl WalletViewModelState {
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        Self {
            number_of_words,
            wallet: Wallet::new(number_of_words, Network::Bitcoin, None),
        }
    }
}

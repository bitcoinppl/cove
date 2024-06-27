use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::RwLock;

use crate::wallet::{NumberOfBip39Words, PendingWallet};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PendingWalletViewModelReconcileMessage {
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
pub trait PendingWalletViewModelReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: PendingWalletViewModelReconcileMessage);
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct RustWalletViewModel {
    pub state: Arc<RwLock<WalletViewModelState>>,
    pub reconciler: Sender<PendingWalletViewModelReconcileMessage>,
    pub reconcile_receiver: Arc<Receiver<PendingWalletViewModelReconcileMessage>>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletViewModelState {
    pub number_of_words: NumberOfBip39Words,
    pub wallet: Arc<PendingWallet>,
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

    // check if all the word groups are valid
    #[uniffi::method]
    pub fn is_all_words_valid(&self, entered_words: Vec<Vec<String>>) -> bool {
        let state = self.state.read();
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        for (actual_word, entered_word) in state.wallet.words_iter().zip(entered_words) {
            if actual_word != entered_word.to_lowercase().trim() {
                return false;
            }
        }

        true
    }

    // get string of all invalid words
    #[uniffi::method]
    pub fn invalid_words_string(&self, entered_words: Vec<Vec<String>>) -> String {
        let state = self.state.read();
        let entered_words = entered_words.iter().flat_map(|words| words.iter());

        let mut invalid_words = Vec::new();
        for (index, (actual_word, entered_word)) in
            state.wallet.words_iter().zip(entered_words).enumerate()
        {
            if actual_word != entered_word.to_lowercase().trim() {
                invalid_words.push((index + 1).to_string());
            }
        }

        invalid_words.join(", ")
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
    pub fn dispatch(&self, action: WalletViewModelAction) {
        match action {
            WalletViewModelAction::UpdateWords(words) => {
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

impl WalletViewModelState {
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        Self {
            number_of_words,
            wallet: PendingWallet::new(number_of_words, Network::Bitcoin, None).into(),
        }
    }
}

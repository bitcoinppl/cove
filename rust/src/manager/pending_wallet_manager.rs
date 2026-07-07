use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crate::{
    database::{self, Database},
    keychain::KeychainError,
    manager::{
        cloud_backup_manager::CLOUD_BACKUP_MANAGER, deferred_sender::SingleOrMany,
        reconcile_channel::ReconcileChannel,
    },
    mnemonic::{GroupedWord, MnemonicExt as _, NumberOfBip39Words, WordAccess as _},
    multi_format::MultiFormatError,
    pending_wallet::PendingWallet,
    router::{HotWalletRoute, NewWalletRoute, Route},
    wallet::{Wallet, fingerprint::Fingerprint, metadata::WalletMetadata},
    xpub::XpubError,
};

type Error = PendingWalletManagerError;

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum PendingWalletManagerError {
    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("failed to save wallet to keychain: {0}")]
    Creation(#[from] WalletCreationError),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PendingWalletManagerReconcileMessage {
    Words(NumberOfBip39Words),
}

#[uniffi::export(callback_interface)]
pub trait PendingWalletManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the view model changes
    fn reconcile(&self, message: PendingWalletManagerReconcileMessage);
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct RustPendingWalletManager {
    pub state: Arc<RwLock<PendingWalletManagerState>>,
    saved_result: Arc<Mutex<Option<PendingWalletSaveResult>>>,
    pub reconciler: ReconcileChannel<PendingWalletManagerReconcileMessage>,
}

#[derive(Debug, Clone, uniffi::Record)]

pub struct PendingWalletManagerState {
    pub number_of_words: NumberOfBip39Words,
    pub wallet: Arc<PendingWallet>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingWalletSaveResult {
    pub metadata: WalletMetadata,
    pub routes: Vec<Route>,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PendingWalletManagerAction {
    UpdateWords(NumberOfBip39Words),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
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

    #[error("unexpected wallet creation error: {0}")]
    Unexpected(String),

    #[error(transparent)]
    MultiFormat(#[from] MultiFormatError),
}

#[uniffi::export]
impl RustPendingWalletManager {
    #[uniffi::constructor]
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        Self {
            state: Arc::new(RwLock::new(PendingWalletManagerState::new(number_of_words))),
            saved_result: Arc::new(Mutex::new(None)),
            reconciler: ReconcileChannel::new(1000),
        }
    }

    #[uniffi::method]
    pub fn get_state(&self) -> PendingWalletManagerState {
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
    pub fn save_wallet(&self) -> Result<PendingWalletSaveResult, Error> {
        let mut saved_result = self.saved_result.lock();
        if let Some(result) = saved_result.clone() {
            return Ok(result);
        }

        let pending_wallet = self.state.read().wallet.clone();
        let network = pending_wallet.network;
        let mode = Database::global().global_config.wallet_mode();

        // get current number of wallets and add one;
        let number_of_wallets = Database::global().wallets.len(network, mode).unwrap_or(0);

        let name = format!("Wallet {}", number_of_wallets + 1);
        let fingerprint: Fingerprint =
            pending_wallet.mnemonic.xpub(network.into()).fingerprint().into();

        let wallet_metadata = WalletMetadata::new_cove_created_wallet(name, Some(fingerprint));

        // create, persist and select the wallet
        let wallet = Wallet::try_new_persisted_and_selected(
            wallet_metadata,
            pending_wallet.mnemonic.clone(),
            None,
        )?;
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

        let routes = post_save_routes(
            wallet.metadata.id.clone(),
            CLOUD_BACKUP_MANAGER.is_cloud_backup_enabled(),
        );

        let result = PendingWalletSaveResult { metadata: wallet.metadata, routes };
        *saved_result = Some(result.clone());

        Ok(result)
    }

    #[uniffi::method]
    pub fn bip_39_words_grouped(&self) -> Vec<Vec<GroupedWord>> {
        self.state.read().wallet.mnemonic.grouped_words_of(12)
    }

    // boilerplate methods
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<dyn PendingWalletManagerReconciler>) {
        self.reconciler.listen(move |field| match field {
            SingleOrMany::Single(message) => reconciler.reconcile(message),
            SingleOrMany::Many(messages) => {
                for message in messages {
                    reconciler.reconcile(message);
                }
            }
        });
    }

    /// Action from the frontend to change the state of the view model
    #[uniffi::method]
    pub fn dispatch(&self, action: PendingWalletManagerAction) {
        match action {
            PendingWalletManagerAction::UpdateWords(words) => {
                {
                    let mut state = self.state.write();
                    state.wallet = PendingWallet::new(words).into();
                    state.number_of_words = words;
                }

                *self.saved_result.lock() = None;
                self.reconciler.send_sync(PendingWalletManagerReconcileMessage::Words(words));
            }
        }
    }
}

impl PendingWalletManagerState {
    pub fn new(number_of_words: NumberOfBip39Words) -> Self {
        Self { number_of_words, wallet: PendingWallet::new(number_of_words).into() }
    }
}

impl From<crate::wallet::WalletError> for PendingWalletManagerError {
    fn from(error: crate::wallet::WalletError) -> Self {
        WalletCreationError::from(error).into()
    }
}

impl From<crate::wallet::WalletError> for WalletCreationError {
    fn from(error: crate::wallet::WalletError) -> Self {
        use crate::wallet::WalletError;

        match error {
            WalletError::Keychain(error) => Self::Keychain(error),
            WalletError::Database(error) => Self::Database(error),
            WalletError::BdkError(error) => Self::Bdk(error),
            WalletError::PersistError(error) => Self::Persist(error),
            WalletError::MultiFormat(error) => Self::MultiFormat(error),
            WalletError::ParseXpubError(error) => error.into(),
            WalletError::WalletAlreadyExists(id) => {
                Self::Import(format!("wallet already exists: {id}"))
            }

            WalletError::WalletNotFound => {
                Self::Unexpected("wallet not found during creation".to_string())
            }
            WalletError::LoadError(error) => {
                Self::Unexpected(format!("load error during creation: {error}"))
            }
            WalletError::MetadataNotFound => {
                Self::Unexpected("wallet metadata not found during creation".to_string())
            }
            WalletError::UnsupportedWallet(error) => {
                Self::Unexpected(format!("unsupported wallet during creation: {error}"))
            }
            WalletError::DescriptorKeyParseError(error) => {
                Self::Unexpected(format!("descriptor key parse error during creation: {error}"))
            }
        }
    }
}

impl From<XpubError> for WalletCreationError {
    fn from(error: XpubError) -> Self {
        Self::Import(error.to_string())
    }
}

fn post_save_routes(
    wallet_id: crate::wallet::metadata::WalletId,
    cloud_backup_enabled: bool,
) -> Vec<Route> {
    let selected_wallet = Route::SelectedWallet(wallet_id.clone());

    if cloud_backup_enabled {
        return vec![selected_wallet];
    }

    vec![
        selected_wallet,
        Route::NewWallet(NewWalletRoute::HotWallet(HotWalletRoute::VerifyWords(wallet_id))),
    ]
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Once};

    use crate::keychain::{Keychain, KeychainAccess, KeychainError};

    use crate::{
        manager::pending_wallet_manager::post_save_routes,
        mnemonic::NumberOfBip39Words,
        router::{HotWalletRoute, NewWalletRoute, Route},
        wallet::metadata::WalletId,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct TestKeychain(Mutex<HashMap<String, String>>);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.0.lock().remove(&key).is_some()
        }
    }

    fn test_keychain() -> &'static Keychain {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            Keychain::new(Box::<TestKeychain>::default());
        });

        Keychain::global()
    }

    #[test]
    fn save_wallet_returns_same_result_after_success() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::init_test_database();
        test_keychain();

        let manager = RustPendingWalletManager::new(NumberOfBip39Words::Twelve);

        let first = manager.save_wallet().expect("first save should succeed");
        let second = manager.save_wallet().expect("second save should return cached result");

        assert_eq!(second.metadata, first.metadata);
        assert_eq!(second.routes, first.routes);
    }

    #[test]
    fn post_save_routes_skip_word_verification_when_cloud_backup_enabled() {
        let wallet_id = WalletId::new();

        assert_eq!(
            post_save_routes(wallet_id.clone(), true),
            vec![Route::SelectedWallet(wallet_id)]
        );
    }

    #[test]
    fn post_save_routes_include_word_verification_without_cloud_backup() {
        let wallet_id = WalletId::new();

        assert_eq!(
            post_save_routes(wallet_id.clone(), false),
            vec![
                Route::SelectedWallet(wallet_id.clone()),
                Route::NewWallet(NewWalletRoute::HotWallet(HotWalletRoute::VerifyWords(wallet_id))),
            ]
        );
    }
}

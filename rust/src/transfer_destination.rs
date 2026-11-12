//! The user's other wallets a send can pay directly, and the receive addresses handed out to them
//!
//! A destination is any other wallet on the sending wallet's network and in its mode. Main and
//! decoy wallets are stored under separate keys, so a main wallet never lists a decoy wallet or
//! the reverse

use std::time::Duration;

use act_zero::{Addr, call};
use backon::{ExponentialBuilder, Retryable as _};
use cove_types::address::AddressInfoWithDerivation;
use cove_util::result_ext::ResultExt as _;
use tracing::warn;

use crate::{
    database::{
        Database,
        wallet::{WalletInternalMetadataPatch, WalletMetadataPatch},
        wallet_data::WalletDataDb,
    },
    manager::{
        cloud_backup_manager::CLOUD_BACKUP_MANAGER,
        send_flow_manager::alert_state::SendFlowAlertState,
        wallet_manager::{WalletManagerError, actor::WalletActor},
    },
    wallet::{
        Wallet, WalletError,
        metadata::{WalletId, WalletMetadata},
    },
    wallet_lifecycle::{
        WalletConstructionPermit, WalletLifecycleCoordinator, WalletLifecycleFailure,
        WalletStoreWriter,
    },
};

/// Another of the user's wallets that a send can pay directly
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TransferDestination {
    pub metadata: WalletMetadata,
    pub status: TransferDestinationStatus,
}

/// Whether Cove can hand out a destination wallet's next receive address
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TransferDestinationStatus {
    Ready,

    /// The wallet has not finished its first full scan, so its next address may already have
    /// been used on chain. Opening the wallet once runs that scan
    InitialScanIncomplete,
}

/// Why a destination wallet's receive address could not be handed out
#[derive(Debug, thiserror::Error)]
pub(crate) enum TransferAddressError {
    #[error("the wallet is not another wallet on this network and in this mode")]
    NotADestination,

    #[error("the wallet has not finished its first scan")]
    InitialScanIncomplete,

    #[error("unable to read the wallet list: {0}")]
    Database(String),

    #[error("the wallet is busy: {0}")]
    Busy(#[from] WalletLifecycleFailure),

    #[error("unable to load the wallet: {0}")]
    Load(#[from] WalletError),

    #[error("unable to save the wallet's address rotation: {0}")]
    SaveRotation(String),

    #[error("unable to update the wallet's Receive screen: {0}")]
    ReceiveCache(String),

    #[error("the wallet's manager failed: {0}")]
    Manager(WalletManagerError),

    #[error("the wallet's manager closed during the request")]
    ManagerClosed,

    #[error("the address task stopped: {0}")]
    Task(String),
}

impl From<WalletManagerError> for TransferAddressError {
    fn from(error: WalletManagerError) -> Self {
        match error {
            WalletManagerError::InitialScanIncomplete => Self::InitialScanIncomplete,
            error => Self::Manager(error),
        }
    }
}

impl TransferAddressError {
    /// The send flow alert for a destination that could not hand out an address
    pub(crate) fn alert(&self) -> SendFlowAlertState {
        let (title, message) = match self {
            Self::InitialScanIncomplete => (
                "Wallet Not Scanned Yet",
                "Open that wallet once so Cove can scan it, then try again.".to_string(),
            ),
            Self::NotADestination => (
                "Wallet Unavailable",
                "That wallet is not on this network, or is no longer in this wallet list."
                    .to_string(),
            ),
            error => (
                "Unable to Get Address",
                format!("Cove couldn't get a receive address from that wallet: {error}"),
            ),
        };

        SendFlowAlertState::General { title: title.to_string(), message }
    }

    /// A manager closing or another loader holding the store clears on its own in moments
    const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ManagerClosed
                | Self::Busy(
                    WalletLifecycleFailure::ManagerClosing { .. }
                        | WalletLifecycleFailure::ConstructionInProgress { .. }
                )
        )
    }
}

impl TransferDestinationStatus {
    fn of(metadata: &WalletMetadata) -> Self {
        match metadata.internal.performed_full_scan_at {
            Some(_) => Self::Ready,
            None => Self::InitialScanIncomplete,
        }
    }
}

/// The wallets `source` can send to directly, in the wallet list's order
pub(crate) fn transfer_destinations(
    source: &WalletMetadata,
) -> Result<Vec<TransferDestination>, TransferAddressError> {
    let wallets = Database::global()
        .wallets
        .get_all(source.network, source.wallet_mode)
        .map_err_str(TransferAddressError::Database)?;

    let destinations = wallets
        .into_iter()
        .filter(|wallet| wallet.id != source.id)
        .map(|metadata| TransferDestination {
            status: TransferDestinationStatus::of(&metadata),
            metadata,
        })
        .collect();

    Ok(destinations)
}

/// Hand out `destination`'s next receive address for a send from `source`
///
/// The address comes from the destination's Receive rotation, which is saved before returning,
/// so the destination's Receive screen moves past it instead of showing it to someone else
pub(crate) async fn reserve_transfer_address(
    source: &WalletMetadata,
    destination: &WalletId,
) -> Result<AddressInfoWithDerivation, TransferAddressError> {
    let metadata = Database::global()
        .wallets
        .get(destination, source.network, source.wallet_mode)
        .map_err_str(TransferAddressError::Database)?
        .filter(|metadata| metadata.id != source.id)
        .ok_or(TransferAddressError::NotADestination)?;

    if TransferDestinationStatus::of(&metadata) == TransferDestinationStatus::InitialScanIncomplete
    {
        return Err(TransferAddressError::InitialScanIncomplete);
    }

    // a manager that just closed, such as the destination's after switching away from it, keeps
    // its store for a moment while its actor shuts down
    let backoff = ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(50))
        .with_max_delay(Duration::from_millis(500))
        .with_max_times(8);

    (|| reserve_once(destination.clone()))
        .retry(backoff)
        .when(TransferAddressError::is_transient)
        .notify(|error, delay| warn!("retrying transfer address in {delay:?}: {error}"))
        .await
}

/// The result of the blocking step: an address from a closed wallet, or the open wallet's actor
enum ClosedOrOpen {
    Reserved(AddressInfoWithDerivation),
    Open(Addr<WalletActor>),
}

async fn reserve_once(
    destination: WalletId,
) -> Result<AddressInfoWithDerivation, TransferAddressError> {
    // the construction permit is tied to the thread that took it, so the closed-wallet write
    // runs start to finish on one blocking thread
    let step = cove_tokio::task::spawn_blocking(move || match WalletLifecycleCoordinator::global()
        .begin_store_write(
        destination.clone(),
    )? {
        WalletStoreWriter::Actor(actor) => Ok(ClosedOrOpen::Open(actor)),
        WalletStoreWriter::Closed(permit) => {
            reserve_in_closed_wallet(&destination, permit).map(ClosedOrOpen::Reserved)
        }
    })
    .await
    .map_err_str(TransferAddressError::Task)??;

    match step {
        ClosedOrOpen::Reserved(address) => Ok(address),
        ClosedOrOpen::Open(actor) => {
            let address = call!(actor.reserve_transfer_address())
                .await
                .map_err(|_| TransferAddressError::ManagerClosed)??;

            Ok(address)
        }
    }
}

/// Load a wallet no manager has open, take its next address, and save the rotation, all while
/// `_permit` keeps a manager from opening the same store
fn reserve_in_closed_wallet(
    destination: &WalletId,
    _permit: WalletConstructionPermit,
) -> Result<AddressInfoWithDerivation, TransferAddressError> {
    let mut wallet = Wallet::try_load_persisted(destination.clone())?;

    // the list read above may be older than the stored wallet, so check the loaded metadata too
    if wallet.metadata.internal.performed_full_scan_at.is_none() {
        return Err(TransferAddressError::InitialScanIncomplete);
    }

    let (address, address_index) = wallet.get_next_address()?;
    if let Some(address_index) = address_index {
        let patch = WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
            address_index: Some(address_index),
            ..Default::default()
        });

        let result = Database::global()
            .wallets
            .patch_wallet_metadata(destination, wallet.network, wallet.metadata.wallet_mode, patch)
            .map_err_str(TransferAddressError::SaveRotation)?;

        CLOUD_BACKUP_MANAGER.handle_wallet_metadata_update(&result.before, &result.after);
    }

    WalletDataDb::new_or_existing(destination.clone())
        .and_then(|db| db.forget_cached_receive_address(address.info.index))
        .map_err_str(TransferAddressError::ReceiveCache)?;

    Ok(address)
}

#[cfg(test)]
pub(crate) mod test_support {
    use bip39::Mnemonic;
    use rand::RngExt as _;

    use crate::{
        database::{
            Database,
            wallet::{WalletInternalMetadataPatch, WalletMetadataPatch},
        },
        manager::import_wallet_manager::import_mnemonic_with_target,
        network::Network,
        wallet::metadata::{DiscoveryState, WalletMetadata, WalletMode},
    };

    pub(crate) fn import(network: Network, mode: WalletMode) -> WalletMetadata {
        let mnemonic = Mnemonic::from_entropy(&rand::rng().random::<[u8; 16]>()).unwrap();
        import_mnemonic_with_target(mnemonic, network, mode).expect("the wallet imports")
    }

    fn patch(wallet: &WalletMetadata, patch: WalletMetadataPatch) {
        Database::global()
            .wallets
            .patch_wallet_metadata(&wallet.id, wallet.network, wallet.wallet_mode, patch)
            .expect("the metadata patch saves");
    }

    /// Marks the wallet as fully scanned, and past discovery so opening it starts no scanner
    /// that would reach the network
    pub(crate) fn mark_scanned(wallet: &WalletMetadata) {
        patch(
            wallet,
            WalletMetadataPatch::Internal(WalletInternalMetadataPatch {
                performed_full_scan_at: Some(1),
                ..Default::default()
            }),
        );
        patch(wallet, WalletMetadataPatch::DiscoveryState(DiscoveryState::ChoseAdressType));
    }
}

#[cfg(test)]
mod tests {
    use bdk_wallet::KeychainKind;

    use super::{
        test_support::{import, mark_scanned},
        *,
    };
    use crate::{
        database::wallet_data::ReceiveAddressCache, manager::wallet_manager::RustWalletManager,
        network::Network, test_support::wallet_target::WalletTargetGuard,
        wallet::metadata::WalletMode,
    };

    /// Starts from empty wallet lists with main signet selected, and puts everything back after
    fn main_signet() -> WalletTargetGuard {
        crate::database::test_support::init_test_database();
        crate::test_support::init_test_keychain();
        WalletTargetGuard::new(Network::Signet, WalletMode::Main)
    }

    fn stored_external_index(wallet: &WalletId) -> Option<u32> {
        let wallet = Wallet::try_load_persisted(wallet.clone()).expect("the wallet loads");
        wallet.bdk.derivation_index(KeychainKind::External)
    }

    async fn reserve(source: &WalletMetadata, destination: &WalletMetadata) -> u32 {
        reserve_transfer_address(source, &destination.id)
            .await
            .expect("the destination hands out an address")
            .info
            .index
    }

    #[tokio::test]
    async fn destinations_are_the_other_wallets_on_the_same_network_and_mode() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let source = import(Network::Signet, WalletMode::Main);
        let sibling = import(Network::Signet, WalletMode::Main);
        let decoy = import(Network::Signet, WalletMode::Decoy);
        let testnet = import(Network::Testnet, WalletMode::Main);

        let destinations = transfer_destinations(&source).unwrap();
        let ids: Vec<_> = destinations.iter().map(|destination| &destination.metadata.id).collect();
        assert_eq!(ids, vec![&sibling.id], "only the other main signet wallet is listed");
        assert_eq!(destinations[0].status, TransferDestinationStatus::InitialScanIncomplete);

        mark_scanned(&sibling);
        let destinations = transfer_destinations(&source).unwrap();
        assert_eq!(destinations[0].status, TransferDestinationStatus::Ready);

        // the decoy list holds the decoy wallet alone, so main mode never leaks into it
        let decoy_destinations = transfer_destinations(&decoy).unwrap();
        assert!(decoy_destinations.is_empty(), "got {decoy_destinations:?}");

        // the same refusals hold when asked for an address directly
        mark_scanned(&decoy);
        mark_scanned(&testnet);
        for (destination, why) in
            [(&source, "itself"), (&decoy, "a decoy wallet"), (&testnet, "a testnet wallet")]
        {
            let result = reserve_transfer_address(&source, &destination.id).await;
            assert!(
                matches!(result, Err(TransferAddressError::NotADestination)),
                "a main signet wallet cannot send to {why}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_unscanned_destination_hands_out_no_address() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let source = import(Network::Signet, WalletMode::Main);
        let destination = import(Network::Signet, WalletMode::Main);

        let result = reserve_transfer_address(&source, &destination.id).await;
        assert!(matches!(result, Err(TransferAddressError::InitialScanIncomplete)), "{result:?}");
        assert_eq!(stored_external_index(&destination.id), None, "nothing was revealed");
    }

    #[tokio::test]
    async fn a_closed_destination_reveals_each_address_once_and_receive_moves_past_it() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let source = import(Network::Signet, WalletMode::Main);
        let destination = import(Network::Signet, WalletMode::Main);
        mark_scanned(&destination);

        assert_eq!(reserve(&source, &destination).await, 0);
        assert_eq!(reserve(&source, &destination).await, 1, "a second send gets a new address");
        assert_eq!(stored_external_index(&destination.id), Some(1), "the reveals were saved");

        // the destination's own Receive screen takes the next address, not one handed out
        let mut wallet = Wallet::try_load_persisted(destination.id.clone()).unwrap();
        let (receive, _) = wallet.get_next_address().unwrap();
        assert_eq!(receive.info.index, 2);
    }

    #[tokio::test]
    async fn an_open_destination_hands_out_addresses_through_its_actor() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let source = import(Network::Signet, WalletMode::Main);
        let destination = import(Network::Signet, WalletMode::Main);
        mark_scanned(&destination);

        let manager =
            RustWalletManager::try_new(destination.id.clone()).expect("the manager opens");

        // while the manager is open, its actor is the only writer of the store
        let writer = cove_tokio::task::spawn_blocking({
            let id = destination.id.clone();
            move || {
                matches!(
                    WalletLifecycleCoordinator::global().begin_store_write(id),
                    Ok(WalletStoreWriter::Actor(_))
                )
            }
        })
        .await
        .unwrap();
        assert!(writer, "an open wallet's writes go through its actor");

        assert_eq!(reserve(&source, &destination).await, 0);
        assert_eq!(reserve(&source, &destination).await, 1);

        // after the manager closes, a closed-wallet load continues from what the actor saved
        drop(manager);
        assert_eq!(reserve(&source, &destination).await, 2);
    }

    /// Once 25 addresses wait unused, Receive's rotation comes back to addresses its screen may
    /// still have cached, so an address taken for a send leaves that cache, whether the
    /// destination is closed or open
    #[tokio::test]
    async fn a_rotated_address_taken_for_a_send_leaves_the_receive_cache() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let source = import(Network::Signet, WalletMode::Main);
        let destination = import(Network::Signet, WalletMode::Main);
        mark_scanned(&destination);

        {
            let mut wallet = Wallet::try_load_persisted(destination.id.clone()).unwrap();
            for _ in 0..25 {
                wallet.get_next_address().unwrap();
            }
        }

        let db = WalletDataDb::new_or_existing(destination.id.clone()).unwrap();
        let cache_at = |derivation_index| ReceiveAddressCache {
            derivation_index,
            first_shown_at_secs: cove_util::time::unix_timestamp_secs_or_zero(),
            wallet_id: destination.id.clone(),
            network: destination.network,
            address_type: destination.address_type,
        };
        let cached_index =
            || db.get_receive_address_cache().unwrap().map(|cache| cache.derivation_index);

        db.set_receive_address_cache(cache_at(0)).unwrap();
        assert_eq!(reserve(&source, &destination).await, 0, "the rotation starts over");
        assert_eq!(cached_index(), None, "a closed wallet kept the taken address cached");

        let _manager =
            RustWalletManager::try_new(destination.id.clone()).expect("the manager opens");

        db.set_receive_address_cache(cache_at(1)).unwrap();
        assert_eq!(reserve(&source, &destination).await, 1);
        assert_eq!(cached_index(), None, "an open wallet kept the taken address cached");

        // an address the send did not take stays on the Receive screen
        db.set_receive_address_cache(cache_at(0)).unwrap();
        assert_eq!(reserve(&source, &destination).await, 2);
        assert_eq!(cached_index(), Some(0));
    }
}

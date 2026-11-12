//! Labels a send between two of the user's wallets on both sides
//!
//! At broadcast the sending wallet asks which of the user's other wallets on its network and in
//! its mode own the outputs, then labels the transaction "Transfer to <wallet>" in the sender and
//! "Transfer from <wallet>" in each receiving wallet. A wallet that already has a label for the
//! transaction keeps it

use bitcoin::{Transaction, Txid};
use cove_types::TxId;
use cove_util::result_ext::ResultExt as _;
use tracing::warn;

use crate::{
    address_ownership::{AddressOwnership, AddressOwnershipError, AddressOwnershipLookup},
    database::Database,
    label_manager::{LabelManager, LabelManagerError},
    network::Network,
    wallet::metadata::{WalletId, WalletMetadata, WalletMode},
    wallet_lifecycle::WalletLifecycleCoordinator,
};

/// Another of the user's wallets that a send pays
#[derive(Debug, Clone, PartialEq, Eq)]
struct TransferRecipient {
    wallet_id: WalletId,
    wallet_name: String,
}

/// Label `transaction`, just broadcast by `sender`, in the sender and in every other wallet of
/// the user's it pays, returning whether the sender's own label was written
///
/// This reads every wallet store on the sender's network, so async callers should run it on a
/// blocking thread
pub(crate) fn label_transfer(sender: &WalletMetadata, transaction: &Transaction) -> bool {
    let recipients = transfer_recipients(sender, transaction);
    if recipients.is_empty() {
        return false;
    }

    let txid = transaction.compute_txid();
    for recipient in &recipients {
        let wallet = WalletKey {
            id: &recipient.wallet_id,
            network: sender.network,
            mode: sender.wallet_mode,
        };
        label_once(wallet, txid, format!("Transfer from {}", sender.name));
    }

    let names = recipients
        .iter()
        .map(|recipient| recipient.wallet_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let wallet = WalletKey { id: &sender.id, network: sender.network, mode: sender.wallet_mode };
    label_once(wallet, txid, format!("Transfer to {names}"))
}

/// The user's other wallets that `transaction`'s outputs pay, in output order, each once
fn transfer_recipients(
    sender: &WalletMetadata,
    transaction: &Transaction,
) -> Vec<TransferRecipient> {
    let scripts =
        transaction.output.iter().map(|output| output.script_pubkey.clone()).collect::<Vec<_>>();

    let ownerships = match AddressOwnershipLookup::for_wallet(sender).find_scripts_owners(&scripts)
    {
        Ok(ownerships) => ownerships,

        // without the active mode, naming a wallet could reveal the other mode's wallets
        Err(
            AddressOwnershipError::WalletModeChanged
            | AddressOwnershipError::WalletModeUnavailable(_),
        ) => return Vec::new(),

        Err(error) => {
            warn!("unable to look up who owns a transfer's outputs: {error}");
            return Vec::new();
        }
    };

    let mut recipients: Vec<TransferRecipient> = Vec::new();
    for ownership in ownerships {
        let AddressOwnership::Owned { owners } = ownership else { continue };

        // the sender owns its change
        for owner in owners.into_iter().filter(|owner| owner.wallet_id != sender.id) {
            if recipients.iter().any(|recipient| recipient.wallet_id == owner.wallet_id) {
                continue;
            }

            recipients.push(TransferRecipient {
                wallet_id: owner.wallet_id,
                wallet_name: owner.wallet_name,
            });
        }
    }

    recipients
}

/// Label the transaction in one wallet unless it already has a label, returning whether it did
///
/// A wallet deleted since the lookup is skipped, and the persistence operation keeps it from
/// being deleted until the label is written, so its data store never comes back
fn label_once(wallet: WalletKey<'_>, txid: Txid, label: String) -> bool {
    let WalletKey { id, network, mode } = wallet;

    let labeled = WalletLifecycleCoordinator::global()
        .begin_persistence_operation(id.clone())
        .map_err_str(LabelManagerError::Save)
        .and_then(|_operation| {
            let exists = Database::global()
                .wallets
                .get(id, network, mode)
                .map_err_str(LabelManagerError::Get)?
                .is_some();

            if !exists {
                return Ok(false);
            }

            LabelManager::try_new(id.clone())
                .map_err_str(LabelManagerError::Get)?
                .label_transaction_if_unlabeled(TxId(txid), label)
        });

    labeled.unwrap_or_else(|error| {
        warn!("unable to label a transfer wallet_id={id} txid={txid}: {error}");
        false
    })
}

/// Where a wallet is stored
#[derive(Debug, Clone, Copy)]
struct WalletKey<'a> {
    id: &'a WalletId,
    network: Network,
    mode: WalletMode,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bdk_wallet::KeychainKind;
    use bip39::Mnemonic;
    use bitcoin::{Amount, ScriptBuf, TxOut, absolute::LockTime, transaction::Version};

    use super::*;
    use crate::{
        manager::import_wallet_manager::import_mnemonic_with_target,
        network::Network,
        test_support::{
            hardware_signing::external_address,
            wallet_target::{WalletTargetGuard, select_target},
        },
        wallet::{Wallet, metadata::WalletMode},
    };

    /// Starts from empty wallet lists with main signet selected, and puts everything back after
    fn main_signet() -> WalletTargetGuard {
        crate::database::test_support::init_test_database();
        crate::test_support::init_test_keychain();
        WalletTargetGuard::new(Network::Signet, WalletMode::Main)
    }

    fn import(mode: WalletMode) -> WalletMetadata {
        let mnemonic = Mnemonic::from_entropy(&rand::random::<[u8; 16]>()).unwrap();
        import_mnemonic_with_target(mnemonic, Network::Signet, mode).expect("the wallet imports")
    }

    fn script(wallet: &WalletMetadata, keychain: KeychainKind, index: u32) -> ScriptBuf {
        let wallet = Wallet::try_load_persisted(wallet.id.clone()).expect("the wallet loads");
        wallet.bdk.peek_address(keychain, index).address.script_pubkey()
    }

    fn external() -> ScriptBuf {
        external_address(bitcoin::Network::Signet).script_pubkey()
    }

    fn paying(scripts: Vec<ScriptBuf>) -> Transaction {
        let output = scripts
            .into_iter()
            .map(|script_pubkey| TxOut { value: Amount::from_sat(10_000), script_pubkey })
            .collect();

        Transaction { version: Version::TWO, lock_time: LockTime::ZERO, input: vec![], output }
    }

    fn label(wallet: &WalletMetadata, transaction: &Transaction) -> Option<String> {
        let tx_id = Arc::new(TxId(transaction.compute_txid()));
        LabelManager::try_new(wallet.id.clone()).unwrap().transaction_label(tx_id).unwrap()
    }

    #[tokio::test]
    async fn a_send_to_another_wallet_is_labelled_on_both_sides() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let sender = import(WalletMode::Main);
        let receiver = import(WalletMode::Main);
        let decoy = import(WalletMode::Decoy);

        // wallets load from the active mode's list
        select_target(Network::Signet, WalletMode::Decoy);
        let decoy_script = script(&decoy, KeychainKind::External, 0);
        select_target(Network::Signet, WalletMode::Main);

        // the receiver's address, the sender's change, a stranger, and a decoy wallet
        let transaction = paying(vec![
            script(&receiver, KeychainKind::External, 3),
            script(&sender, KeychainKind::Internal, 0),
            external(),
            decoy_script,
        ]);

        assert!(label_transfer(&sender, &transaction), "the sender's label was not written");
        assert_eq!(label(&sender, &transaction), Some(format!("Transfer to {}", receiver.name)));
        assert_eq!(label(&receiver, &transaction), Some(format!("Transfer from {}", sender.name)));
        assert_eq!(label(&decoy, &transaction), None, "a main wallet's send named a decoy wallet");
    }

    #[tokio::test]
    async fn labels_already_on_the_transaction_are_kept() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let sender = import(WalletMode::Main);
        let receiver = import(WalletMode::Main);
        let transaction = paying(vec![script(&receiver, KeychainKind::External, 0)]);
        let txid = TxId(transaction.compute_txid());

        for (wallet, text) in [(&sender, "rent"), (&receiver, "rent paid")] {
            let labels = LabelManager::try_new(wallet.id.clone()).unwrap();
            assert!(labels.label_transaction_if_unlabeled(txid, text.to_string()).unwrap());
        }

        assert!(!label_transfer(&sender, &transaction), "the sender's label was replaced");
        assert_eq!(label(&sender, &transaction).as_deref(), Some("rent"));
        assert_eq!(label(&receiver, &transaction).as_deref(), Some("rent paid"));
    }

    #[tokio::test]
    async fn a_send_to_no_other_wallet_gets_no_label() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let sender = import(WalletMode::Main);
        let transaction = paying(vec![external(), script(&sender, KeychainKind::Internal, 0)]);

        assert!(!label_transfer(&sender, &transaction));
        assert_eq!(label(&sender, &transaction), None);
    }

    /// A main wallet's send that lands after the app switched to decoy mode names no wallets,
    /// since the lookup would otherwise show main wallets while in decoy mode
    #[tokio::test]
    async fn a_send_after_switching_modes_gets_no_labels() {
        let _lock = crate::test_support::global_state_test_lock().lock().await;
        let _target = main_signet();

        let sender = import(WalletMode::Main);
        let receiver = import(WalletMode::Main);
        let transaction = paying(vec![script(&receiver, KeychainKind::External, 0)]);

        select_target(Network::Signet, WalletMode::Decoy);

        assert!(!label_transfer(&sender, &transaction));
        assert_eq!(label(&sender, &transaction), None);
        assert_eq!(label(&receiver, &transaction), None);
    }
}

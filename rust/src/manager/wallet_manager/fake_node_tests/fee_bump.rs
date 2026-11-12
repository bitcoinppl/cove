//! Speeding up a pending payment by replacing it with one that pays a higher fee, against a fake
//! node that enforces Bitcoin Core's replacement rules

use std::sync::Arc;

use act_zero::call;
use bdk_wallet::{SignOptions, Wallet as BdkWallet};
use bip39::Mnemonic;
use bitcoin::{
    Amount, FeeRate as BdkFeeRate, OutPoint, ScriptBuf, Sequence, Transaction as BdkTransaction,
    TxIn, TxOut, Txid, Witness, absolute::LockTime, transaction::Version,
};
use cove_fake_node::TxLocation;

use super::{
    Fixture, ScreenUpdates, fresh_mnemonic, listed_label, paste_hardware_export, receive_script,
    transaction_list, tx_id, wait_for_refresh_to_settle,
};
use crate::{
    manager::{
        import_wallet_manager::import_mnemonic_with_target, wallet_manager::RustWalletManager,
    },
    mnemonic::MnemonicExt as _,
    network::Network,
    transaction::{
        FeeRate,
        fee_bump::{FeeBumpBlocker, FeeBumpError},
        ffi::BitcoinTransaction,
    },
    wallet::{
        Address, WalletAddressType,
        metadata::{WalletMode, WalletType},
    },
};

/// Signs as the hot wallet does, or as a hardware wallet holding `mnemonic` would
fn signer(mnemonic: &Mnemonic, network: Network) -> BdkWallet {
    mnemonic
        .clone()
        .into_descriptors(None, network, WalletAddressType::NativeSegwit)
        .into_create_params()
        .network(network.into())
        .create_wallet_no_persist()
        .expect("the signer wallet builds")
}

fn sign(signer: &BdkWallet, mut psbt: bitcoin::Psbt) -> BdkTransaction {
    assert!(signer.sign(&mut psbt, SignOptions::default()).unwrap(), "fully signed");
    psbt.extract_tx().expect("the signed transaction extracts")
}

/// Someone else's first address
fn outside_address(network: Network) -> Address {
    let script = fresh_mnemonic()
        .into_descriptors(None, network, WalletAddressType::NativeSegwit)
        .external
        .extended_descriptor
        .at_derivation_index(0)
        .unwrap()
        .script_pubkey();

    Address::new(bitcoin::Address::from_script(&script, bitcoin::Network::Signet).unwrap())
}

fn sat_per_vb(sat_per_vb: u32) -> BdkFeeRate {
    BdkFeeRate::from_sat_per_vb_u32(sat_per_vb)
}

/// A hot wallet whose receive addresses 0.. were paid `amounts` in confirmed transactions
async fn funded_wallet(
    fx: &mut Fixture,
    amounts: &[u64],
) -> (Mnemonic, RustWalletManager, ScreenUpdates, Vec<Txid>) {
    let mnemonic = fresh_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), fx.network, WalletMode::Main).unwrap();

    let (manager, mut screen) = fx.open(wallet.id.clone());
    let mut fundings = Vec::new();
    for (index, amount) in (0..).zip(amounts) {
        let script = receive_script(&manager, index).await;
        fundings.push(fx.node.chain().pay(&script, Amount::from_sat(*amount)));
    }

    fx.node.chain().confirm_together(&fundings);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    (mnemonic, manager, screen, fundings)
}

/// Builds, signs and broadcasts a payment as Send does, and waits for the refresh after it
async fn pay(
    manager: &RustWalletManager,
    screen: &mut ScreenUpdates,
    signer: &BdkWallet,
    to: &Address,
    amount: u64,
    fee_rate: BdkFeeRate,
) -> Txid {
    let psbt = call!(manager.actor.build_tx(Amount::from_sat(amount), to.clone(), fee_rate))
        .await
        .expect("the wallet actor answers")
        .expect("the payment builds");

    broadcast(manager, screen, sign(signer, psbt)).await
}

async fn broadcast(
    manager: &RustWalletManager,
    screen: &mut ScreenUpdates,
    tx: BdkTransaction,
) -> Txid {
    let txid = tx.compute_txid();
    screen.seen.clear();
    manager
        .broadcast_transaction(Arc::new(BitcoinTransaction(tx)))
        .await
        .expect("the node accepts the transaction");

    wait_for_refresh_to_settle(screen).await;
    txid
}

async fn refresh(manager: &RustWalletManager, screen: &mut ScreenUpdates) {
    screen.seen.clear();
    manager.force_wallet_scan().await.expect("the refresh starts");
    wait_for_refresh_to_settle(screen).await;
}

async fn blocker(manager: &RustWalletManager, txid: Txid) -> FeeBumpBlocker {
    match manager.fee_bump_target(txid).await {
        Err(FeeBumpError::Blocked(blocker)) => blocker,
        other => panic!("expected {txid} to be blocked, got {other:?}"),
    }
}

/// An unsigned transaction the fake node takes into its mempool without checking scripts, for
/// shapes this wallet never builds itself
fn unchecked_spend(inputs: &[OutPoint], outputs: Vec<TxOut>) -> BdkTransaction {
    let input = inputs
        .iter()
        .map(|outpoint| TxIn {
            previous_output: *outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    BdkTransaction { version: Version::TWO, lock_time: LockTime::ZERO, input, output: outputs }
}

fn output(to: &Address, amount: u64) -> TxOut {
    TxOut { value: Amount::from_sat(amount), script_pubkey: to.script_pubkey() }
}

#[tokio::test(flavor = "current_thread")]
async fn speeding_up_a_payment_replaces_it_at_the_lowest_rate_a_node_accepts() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let (mnemonic, manager, mut screen, _) = funded_wallet(&mut fx, &[100_000]).await;
    let signer = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);

    let sent = pay(&manager, &mut screen, &signer, &destination, 10_000, sat_per_vb(2)).await;
    let details = manager.transaction_details(tx_id(sent)).await.unwrap().details();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details, "Rent".into(), None)
        .expect("the label saves");

    // one step above the original's 2 sat/vB at least
    let target = manager.fee_bump_target(sent).await.expect("the payment can be sped up");
    assert!(target.minimum_fee_rate >= sat_per_vb(3), "{target:?}");

    // the minimum is rounded up to a 0.1 sat/vB step, so only the original's own rate is
    // certain to be refused
    let refused =
        manager.confirm_fee_bump_txn(sent, FeeRate::from(target.original.fee_rate())).await;
    assert!(matches!(refused, Err(FeeBumpError::FeeRateTooLow { .. })), "{refused:?}");

    // the replacement keeps the recipient and amount, and the node takes it at the minimum
    let confirm = manager
        .confirm_fee_bump_txn(sent, FeeRate::from(target.minimum_fee_rate))
        .await
        .expect("the replacement builds");

    assert_eq!(confirm.sending_amount().to_sat(), 10_000);
    assert_eq!(confirm.sending_to().to_string(), destination.to_string());
    assert!(confirm.fee_total.to_sat() > target.original.fee.to_sat());

    let replacement = broadcast(&manager, &mut screen, sign(&signer, confirm.psbt().0)).await;
    assert_eq!(fx.node.chain().location(sent), None, "the node dropped the original");
    assert_eq!(fx.node.chain().location(replacement), Some(TxLocation::Mempool));

    // the list shows the replacement in the original's place, with its label
    let listed = transaction_list(&manager).await;
    assert!(listed.iter().any(|transaction| transaction.id().0 == replacement));
    assert!(listed.iter().all(|transaction| transaction.id().0 != sent));
    assert_eq!(listed_label(&manager, replacement).await.as_deref(), Some("Rent"));
    assert_eq!(blocker(&manager, sent).await, FeeBumpBlocker::Replaced);

    // the replacement can be sped up again, from its own higher rate
    let again = manager.fee_bump_target(replacement).await.expect("the replacement can be sped up");
    assert!(again.minimum_fee_rate > target.minimum_fee_rate);

    fx.node.chain().confirm(replacement);
    refresh(&manager, &mut screen).await;
    assert_eq!(blocker(&manager, replacement).await, FeeBumpBlocker::Confirmed);
}

#[tokio::test(flavor = "current_thread")]
async fn transactions_that_cannot_be_sped_up_say_why() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let (mnemonic, manager, mut screen, _) = funded_wallet(&mut fx, &[100_000]).await;

    let signer = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);
    let other = outside_address(fx.network);

    // a payment received, still pending
    let script = receive_script(&manager, 5).await;
    let received = fx.node.chain().pay(&script, Amount::from_sat(5_000));
    refresh(&manager, &mut screen).await;
    assert_eq!(blocker(&manager, received).await, FeeBumpBlocker::NotSentByWallet);

    // spending a payment's change pends behind it, so the later payment is the one to speed up
    let first = pay(&manager, &mut screen, &signer, &destination, 90_000, sat_per_vb(2)).await;
    let second = pay(&manager, &mut screen, &signer, &destination, 5_000, sat_per_vb(2)).await;
    let second_tx = fx.node.chain().tx(second).unwrap().tx;
    assert!(second_tx.input.iter().any(|input| input.previous_output.txid == first));
    assert_eq!(blocker(&manager, first).await, FeeBumpBlocker::PendingDescendant);
    manager.fee_bump_target(second).await.expect("the later payment can be sped up");

    // two more confirmed coins for the spends below, funded after the payments so neither
    // payment spends them, since a double spend would replace a payment in the wallet's view
    let mut coins = Vec::new();
    for index in [6, 7] {
        let script = receive_script(&manager, index).await;
        coins.push(fx.node.chain().pay(&script, Amount::from_sat(30_000)));
    }

    fx.node.chain().confirm_together(&coins);

    // a payjoin-like spend of this wallet's coin together with someone else's
    let foreign = fx.node.chain().pay(&other.script_pubkey(), Amount::from_sat(10_000));
    let coin = OutPoint { txid: coins[0], vout: 0 };
    let foreign_coin = OutPoint { txid: foreign, vout: 0 };
    let mixed = unchecked_spend(&[coin, foreign_coin], vec![output(&destination, 38_000)]);
    let mixed = fx.node.chain().add_to_mempool(mixed);

    // a batch paying two people
    let coin = OutPoint { txid: coins[1], vout: 0 };
    let batch =
        unchecked_spend(&[coin], vec![output(&destination, 10_000), output(&other, 19_000)]);
    let batch = fx.node.chain().add_to_mempool(batch);

    refresh(&manager, &mut screen).await;
    assert_eq!(blocker(&manager, mixed).await, FeeBumpBlocker::ForeignInputs);
    assert_eq!(blocker(&manager, batch).await, FeeBumpBlocker::MultipleRecipients);

    // a watch-only wallet can't sign a replacement
    manager.set_wallet_type(WalletType::WatchOnly).await.expect("the wallet type changes");
    assert_eq!(blocker(&manager, second).await, FeeBumpBlocker::WatchOnly);
    manager.set_wallet_type(WalletType::Hot).await.expect("the wallet type changes");

    fx.node.chain().confirm_together(&[first, second]);
    refresh(&manager, &mut screen).await;
    assert_eq!(blocker(&manager, second).await, FeeBumpBlocker::Confirmed);
}

#[tokio::test(flavor = "current_thread")]
async fn a_max_send_is_sped_up_from_another_confirmed_coin() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let (mnemonic, manager, mut screen, _) = funded_wallet(&mut fx, &[50_000]).await;
    let signer = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);

    // sending everything leaves no change to pay a higher fee from
    let drain =
        call!(manager.actor.build_ephemeral_drain_tx(destination.clone(), sat_per_vb(2).into()))
            .await
            .unwrap()
            .expect("the max send builds");

    let sent_amount = drain.unsigned_tx.output[0].value;
    let sent = broadcast(&manager, &mut screen, sign(&signer, drain)).await;
    let short = manager.fee_bump_target(sent).await;
    assert!(matches!(short, Err(FeeBumpError::InsufficientFunds { .. })), "{short:?}");

    // a confirmed coin received later pays the extra fee, and the recipient still gets it all
    let script = receive_script(&manager, 3).await;
    let funding = fx.node.chain().pay(&script, Amount::from_sat(20_000));
    fx.node.chain().confirm(funding);
    refresh(&manager, &mut screen).await;

    let target = manager.fee_bump_target(sent).await.expect("the new coin pays for the bump");
    let confirm = manager
        .confirm_fee_bump_txn(sent, FeeRate::from(target.minimum_fee_rate))
        .await
        .expect("the replacement builds");

    assert_eq!(confirm.sending_amount().to_sat(), sent_amount.to_sat());
    let replacement = broadcast(&manager, &mut screen, sign(&signer, confirm.psbt().0)).await;
    let spent = fx.node.chain().tx(replacement).unwrap().tx.input.len();
    assert_eq!(spent, 2, "the replacement spends the original coin and the new one");
}

#[tokio::test(flavor = "current_thread")]
async fn a_replacement_a_sync_finds_first_still_takes_over_from_the_original() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let (mnemonic, manager, mut screen, _) = funded_wallet(&mut fx, &[100_000]).await;
    let signer = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);

    let sent = pay(&manager, &mut screen, &signer, &destination, 10_000, sat_per_vb(2)).await;
    let details = manager.transaction_details(tx_id(sent)).await.unwrap().details();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details, "Rent".into(), None)
        .expect("the label saves");

    let target = manager.fee_bump_target(sent).await.expect("the payment can be sped up");
    let confirm = manager
        .confirm_fee_bump_txn(sent, FeeRate::from(target.minimum_fee_rate))
        .await
        .expect("the replacement builds");

    // the node takes the replacement and a sync finds it before the broadcast result arrives,
    // so the wallet already shows it in the original's place
    let replacement = sign(&signer, confirm.psbt().0);
    let replacement_txid = fx.node.chain().broadcast(replacement.clone()).unwrap();
    refresh(&manager, &mut screen).await;
    assert!(transaction_list(&manager).await.iter().all(|listed| listed.id().0 != sent));

    // the late result still hands the label over, and the replacement stays listed
    broadcast(&manager, &mut screen, replacement).await;
    assert_eq!(listed_label(&manager, replacement_txid).await.as_deref(), Some("Rent"));
    assert_eq!(blocker(&manager, sent).await, FeeBumpBlocker::Replaced);
}

#[tokio::test(flavor = "current_thread")]
async fn a_double_spend_paying_someone_else_keeps_no_label() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let (mnemonic, manager, mut screen, _) = funded_wallet(&mut fx, &[100_000]).await;
    let signer = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);

    let sent = pay(&manager, &mut screen, &signer, &destination, 10_000, sat_per_vb(2)).await;
    let details = manager.transaction_details(tx_id(sent)).await.unwrap().details();
    manager
        .label_manager()
        .insert_or_update_labels_for_txn(details, "Rent".into(), None)
        .expect("the label saves");

    // another device spends the same coin at a higher fee, but pays someone else
    let target = manager.fee_bump_target(sent).await.expect("the payment can be sped up");
    let confirm = manager
        .confirm_fee_bump_txn(sent, FeeRate::from(target.minimum_fee_rate))
        .await
        .expect("the replacement builds");

    let mut psbt = confirm.psbt().0;
    let someone_else = outside_address(fx.network).script_pubkey();
    let recipient = psbt
        .unsigned_tx
        .output
        .iter_mut()
        .find(|output| !signer.is_mine(output.script_pubkey.clone()))
        .expect("the payment has a recipient");
    recipient.script_pubkey = someone_else;

    let double_spend = broadcast(&manager, &mut screen, sign(&signer, psbt)).await;
    assert_eq!(listed_label(&manager, double_spend).await, None);
    assert_eq!(blocker(&manager, sent).await, FeeBumpBlocker::Replaced);
}

#[tokio::test(flavor = "current_thread")]
async fn a_hardware_wallet_signs_the_replacement_it_exports() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();

    let mnemonic = fresh_mnemonic();
    let descriptor =
        mnemonic.clone().into_descriptors(None, fx.network, WalletAddressType::NativeSegwit);
    let script = descriptor.external.extended_descriptor.at_derivation_index(0).unwrap();
    let funding = fx.node.chain().pay(&script.script_pubkey(), Amount::from_sat(100_000));
    fx.node.chain().confirm(funding);

    let wallet = paste_hardware_export(&mnemonic, fx.network);
    assert_eq!(wallet.metadata.wallet_type, WalletType::Cold);
    let id = wallet.metadata.id.clone();
    drop(wallet);

    let (manager, mut screen) = fx.open(id);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    // the hardware wallet signs what the app exports, here and for the replacement
    let hardware = signer(&mnemonic, fx.network);
    let destination = outside_address(fx.network);
    let sent = pay(&manager, &mut screen, &hardware, &destination, 10_000, sat_per_vb(2)).await;

    let target = manager.fee_bump_target(sent).await.expect("the payment can be sped up");
    let confirm = manager
        .confirm_fee_bump_txn(sent, FeeRate::from(target.minimum_fee_rate))
        .await
        .expect("the replacement builds");

    // Send saves the draft before showing the export, and the broadcast settles it
    let confirm = Arc::new(confirm);
    manager.save_unsigned_transaction(confirm.clone()).expect("the draft saves");
    assert_eq!(manager.get_unsigned_transactions().unwrap().len(), 1);

    let psbt = confirm.psbt().0;
    assert!(
        psbt.inputs.iter().all(|input| !input.bip32_derivation.is_empty()),
        "every input says which key signs it"
    );

    let replacement = broadcast(&manager, &mut screen, sign(&hardware, psbt)).await;
    assert_eq!(fx.node.chain().location(sent), None);
    assert_eq!(fx.node.chain().location(replacement), Some(TxLocation::Mempool));
    assert!(manager.get_unsigned_transactions().unwrap().is_empty(), "the draft was settled");
}

//! Pays a real BIP 77 receiver through Cove's Payjoin sender: an in-process directory and OHTTP
//! relays carry the session, a BDK wallet signs for the receiver, and the fake node holds both
//! sides' coins and checks every script of the transaction that is broadcast

use std::time::Duration;

use act_zero::call;
use bitcoin::{Amount, FeeRate as BdkFeeRate};
use cove_fake_node::Protocol;
use cove_types::{address::AddressWithNetwork, fees::FeeRate};

use super::fake_node_tests::{Fixture, fresh_mnemonic, receive_script};
use crate::{
    manager::import_wallet_manager::import_mnemonic_with_target,
    mnemonic::MnemonicExt as _,
    test_support::{
        fake_node::wait_until,
        payjoin::{PayjoinReceiver, PayjoinServices},
    },
    wallet::{Address, WalletAddressType, metadata::WalletMode},
};

/// A hot wallet pays a receiver's pj URI, the receiver adds its own coin, and the node sees one
/// broadcast: the receiver's signed proposal, after which the sender's session record is gone
#[tokio::test(flavor = "current_thread")]
async fn hot_wallet_pays_a_bip77_receiver_with_a_payjoin() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;

    let wallet =
        import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();
    let (manager, mut screen) = fx.open(wallet.id.clone());
    let funding =
        fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(100_000));
    fx.node.chain().confirm(funding);
    manager.start_wallet_scan().await.expect("the scan starts");
    screen.until_scan_settles().await;

    let receiver_wallet = fresh_mnemonic()
        .into_descriptors(None, fx.network, WalletAddressType::NativeSegwit)
        .into_create_params()
        .network(fx.network.into())
        .create_wallet_no_persist()
        .expect("the receiver wallet opens");
    let mut receiver = PayjoinReceiver::start(
        &services,
        fx.node.chain(),
        receiver_wallet,
        Amount::from_sat(50_000),
    )
    .await;

    // the URI goes through the same parser the send screen uses
    let uri = AddressWithNetwork::try_new(&receiver.pj_uri().to_string()).expect("pj URI parses");
    let endpoint = uri.payjoin.as_ref().expect("the URI carries a pj endpoint").endpoint.clone();

    // a Max payment has no change output, which keeps this test clear of the change
    // misdetection in recipient_output_index tracked in #1088
    let fee_rate = FeeRate::from(BdkFeeRate::from_sat_per_vb(2).expect("a valid fee rate"));
    let psbt = call!(manager.actor.build_ephemeral_drain_tx(Address::from(uri.address), fee_rate))
        .await
        .expect("the wallet actor answers")
        .expect("the Max payment builds");
    assert_eq!(psbt.unsigned_tx.output.len(), 1, "a Max payment has no change");

    call!(
        manager.actor.set_payjoin_config_for_test(services.sender_config(Duration::from_secs(60)))
    )
    .await
    .expect("the wallet actor answers");
    call!(manager.actor.start_payjoin_payment_for_test(psbt, endpoint))
        .await
        .expect("the wallet actor answers")
        .expect("the Payjoin session starts");

    let original = tokio::time::timeout(Duration::from_secs(30), receiver.wait_for_original())
        .await
        .expect("the sender posts its Original PSBT");
    receiver.respond(original).await;

    wait_until("the Payjoin to be broadcast and its session cleared", || {
        !fx.node.chain().broadcasts().is_empty()
    })
    .await;

    let session_cleared = || async {
        call!(manager.actor.payjoin_session_for_test())
            .await
            .expect("the wallet actor answers")
            .is_none()
    };

    for _ in 0..50 {
        if session_cleared().await {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(session_cleared().await, "the session record is deleted after the broadcast");

    // the chain keeps each txid once, so a repeated send of the same proposal shows only in the
    // request log, where only a broadcast's POST has the bare /tx path
    let broadcast_requests = fx
        .node
        .requests()
        .into_iter()
        .filter(|request| request.protocol == Protocol::Esplora && request.name == "/tx")
        .count();
    assert_eq!(broadcast_requests, 1, "the wallet sent exactly one broadcast");

    // the fake node records a broadcast only after every input's script verifies
    let broadcasts = fx.node.chain().broadcasts();
    assert_eq!(broadcasts.len(), 1, "the node accepted exactly one transaction");

    let payjoin = &broadcasts[0];
    let receiver_coin = receiver.coin_outpoint();
    assert!(
        payjoin.input.iter().any(|input| input.previous_output == receiver_coin),
        "the broadcast is the receiver's proposal, spending the receiver's coin"
    );
    assert!(payjoin.input.len() >= 2, "the proposal spends the sender's and receiver's coins");
    assert!(
        payjoin
            .input
            .iter()
            .filter(|input| input.previous_output != receiver_coin)
            .all(|input| !input.witness.is_empty()),
        "every sender input is signed"
    );
}

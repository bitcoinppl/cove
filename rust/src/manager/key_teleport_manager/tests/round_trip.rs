//! Transfers between two Cove phones, run through both managers end to end
//!
//! iOS and Android run the same Rust send and receive flows, so one round trip here stands in for
//! either direction between the platforms

use bdk_wallet::bitcoin::{NetworkKind, bip32::Xpriv};
use cove_device::keychain::WalletXprv;

use super::{
    support::{
        ABANDON_ABOUT, ExpectedWallet, WalletTargetGuard, all_wallet_ids, deliver_response,
        every_scope, import_new_wallet, select_target, send_from_cove, start_receiver, wallets_in,
    },
    *,
};

/// Sends a mnemonic wallet from one Cove manager to another and checks the imported wallet
fn assert_mnemonic_transfer_imports_the_sender_wallet(mnemonic: Mnemonic) {
    let expected = ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet);
    let word_count = mnemonic.word_count();
    let (receiver, request) = start_receiver();
    let (packet, password) = send_from_cove(WalletSecret::Mnemonic(mnemonic.clone()), &request);

    deliver_response(&receiver, &packet, &password).unwrap();

    let KeyTeleportManagerState::ReceiveMnemonicReview(review) = receiver.state() else {
        panic!("expected mnemonic review, got {:?}", receiver.state())
    };
    assert_eq!(review.word_count as usize, word_count);
    assert_eq!(receiver.reveal_mnemonic_words().join(" "), mnemonic.to_string());

    let wallet = import_new_wallet(&receiver);
    expected.assert_imported(&wallet);
}

#[test]
fn twelve_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);
    let mnemonic = Mnemonic::from_str(ABANDON_ABOUT).unwrap();

    // BIP84's published m/84'/1'/0'/0/0 vector pins the independent derivation itself
    let expected = ExpectedWallet::from_mnemonic(&mnemonic, Network::Signet);
    assert_eq!(expected.first_address, "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl");

    assert_mnemonic_transfer_imports_the_sender_wallet(mnemonic);
}

#[test]
fn eighteen_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    assert_mnemonic_transfer_imports_the_sender_wallet(
        Mnemonic::from_entropy(&[0x5c; 24]).unwrap(),
    );
}

#[test]
fn twenty_four_word_mnemonic_transfer_imports_the_sender_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    assert_mnemonic_transfer_imports_the_sender_wallet(
        Mnemonic::from_entropy(&[0xa7; 32]).unwrap(),
    );
}

#[test]
fn master_private_key_transfer_imports_an_xprv_wallet_with_the_same_first_address() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    // Cove keeps master keys mainnet-encoded and applies the wallet's network when deriving, so a
    // Signet xprv wallet holds the same string KeyTeleport transfers
    let xprv = Xpriv::new_master(NetworkKind::Main, &[0x3d; 32]).unwrap();
    let expected = ExpectedWallet::from_xprv(xprv, Network::Signet);
    let (receiver, request) = start_receiver();
    let secret = WalletSecret::Xpriv(WalletXprv::try_from(xprv).unwrap());
    let (packet, password) = send_from_cove(secret, &request);

    deliver_response(&receiver, &packet, &password).unwrap();

    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveXprvReview(_)));
    assert_eq!(receiver.reveal_xprv().as_deref(), Some(xprv.to_string().as_str()));

    let wallet = import_new_wallet(&receiver);
    let stored = Keychain::global().get_wallet_secret(&wallet.id).unwrap().unwrap();
    assert_eq!(stored.as_xprv().map(WalletXprv::to_xpriv), Some(xprv));
    expected.assert_imported(&wallet);
}

#[test]
fn teleporting_the_same_wallet_twice_offers_the_existing_wallet() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let imported = import_new_wallet(&receiver);

    // 73c5da0a is the master fingerprint of the BIP84 test mnemonic the sender used
    assert_eq!(imported.name, "KeyTeleport 73C5DA0A");
    let wallets_after_first_import = all_wallet_ids();

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    receiver.handle_action(Action::ImportReceivedWallet).unwrap();

    let KeyTeleportManagerState::ReceiveAlreadyImportedWallet(existing) = receiver.state() else {
        panic!("expected the existing wallet, got {:?}", receiver.state())
    };
    assert_eq!(existing.id, imported.id);
    assert_eq!(existing.name, imported.name);
    assert_eq!(all_wallet_ids(), wallets_after_first_import);
    assert!(Keychain::global().get_key_teleport_receive_session().unwrap().is_none());
}

#[test]
fn received_wallet_lands_only_in_the_request_scope() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Decoy);

    let (receiver, request) = start_receiver();
    assert_eq!((request.network, request.wallet_mode), (Network::Signet, WalletMode::Decoy));
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let wallet = import_new_wallet(&receiver);

    assert_eq!((wallet.network, wallet.wallet_mode), (Network::Signet, WalletMode::Decoy));
    for (network, mode) in every_scope() {
        let present = wallets_in(network, mode).iter().any(|stored| stored.id == wallet.id);
        let is_request_scope = (network, mode) == (Network::Signet, WalletMode::Decoy);
        assert_eq!(present, is_request_scope, "wallet presence in {network:?} {mode:?}");
    }
}

#[test]
fn receive_import_refuses_a_changed_wallet_mode() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let _target = WalletTargetGuard::new(Network::Signet, WalletMode::Main);

    let (receiver, request) = start_receiver();
    let (packet, password) = sender_transfer(&request);
    deliver_response(&receiver, &packet, &password).unwrap();
    let wallets_before = all_wallet_ids();
    select_target(Network::Signet, WalletMode::Decoy);

    let error = receiver.handle_action(Action::ImportReceivedWallet);

    assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionScopeChanged));
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
    assert_eq!(all_wallet_ids(), wallets_before);
}

#[test]
fn response_for_a_replaced_request_is_rejected_and_the_new_request_still_works() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let (receiver, old_request) = start_receiver();
    let (old_packet, old_password) = sender_transfer(&old_request);

    receiver.handle_action(Action::RestartReceive).unwrap();
    let new_request = receive_request(&receiver);
    assert_ne!(new_request.packet, old_request.packet);

    assert!(deliver_response(&receiver, &old_packet, &old_password).is_err());
    assert_eq!(receive_request(&receiver), new_request, "the new request must stay displayed");

    let (packet, password) = sender_transfer(&new_request);
    deliver_response(&receiver, &packet, &password).unwrap();
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
}

#[test]
fn response_for_an_expired_request_is_rejected_and_its_replacement_still_works() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    init_globals();
    let (receiver, expired_request) = start_receiver();
    let (expired_packet, expired_password) = sender_transfer(&expired_request);

    // the request on screen ages past its 24-hour lifetime before the response arrives
    {
        let mut model = receiver.model.lock();
        let Phase::Receive(ReceivePhase::Ready { session, .. }) = &mut model.phase else {
            panic!("expected a displayed receive request")
        };
        session.created_at_secs -= 24 * 60 * 60 + 1;
    }

    let error = deliver_response(&receiver, &expired_packet, &expired_password);

    assert_eq!(error, Err(KeyTeleportAlert::ReceiveSessionExpired));
    let replacement = receive_request(&receiver);
    assert_ne!(replacement.packet, expired_request.packet);
    assert!(deliver_response(&receiver, &expired_packet, &expired_password).is_err());
    assert_eq!(receive_request(&receiver), replacement);

    let (packet, password) = sender_transfer(&replacement);
    deliver_response(&receiver, &packet, &password).unwrap();
    assert!(matches!(receiver.state(), KeyTeleportManagerState::ReceiveMnemonicReview(_)));
}

use bdk_wallet::{
    KeychainKind,
    bitcoin::bip32::Xpriv,
    descriptor::ExtendedDescriptor,
    template::{Bip84, DescriptorTemplate as _},
};
use bip39::Mnemonic;
use cove_device::keychain::Keychain;
use cove_types::address::AddressWithNetwork;

use super::{
    ADDRESS_OWNERSHIP_LOOKUP_GAP, AddressChain, AddressOwnership, AddressOwnershipError,
    AddressOwnershipLookup, OwnedAddress, UncheckedWallet, check_address_ownership,
};
use crate::{
    bdk_store::BdkStore,
    database::Database,
    manager::import_wallet_manager::import_mnemonic_with_target,
    network::Network,
    test_support::wallet_target::{WalletTargetGuard, every_scope, select_target},
    wallet::{
        WalletAddressType,
        metadata::{WalletId, WalletMetadata, WalletMode, WalletType},
    },
};

/// A fresh key set, so wallets other tests leave behind can never own its addresses
fn random_mnemonic() -> Mnemonic {
    Mnemonic::from_entropy(&rand::random::<[u8; 16]>()).unwrap()
}

/// BIP 84 account 0 descriptors built straight from BDK, independent of how Cove stores them
fn bip84_descriptor(
    mnemonic: &Mnemonic,
    network: Network,
    keychain: KeychainKind,
) -> ExtendedDescriptor {
    let network = bitcoin::Network::from(network);
    let xprv = Xpriv::new_master(network, &mnemonic.to_seed("")).unwrap();
    let (descriptor, _, _) = Bip84(xprv, keychain).build(network.into()).unwrap();

    descriptor
}

fn address_at(
    mnemonic: &Mnemonic,
    network: Network,
    keychain: KeychainKind,
    index: u32,
) -> AddressWithNetwork {
    let address = bip84_descriptor(mnemonic, network, keychain)
        .at_derivation_index(index)
        .unwrap()
        .address(bitcoin::Network::from(network))
        .unwrap();

    AddressWithNetwork::try_new(&address.to_string()).unwrap()
}

fn find_owners(address: &AddressWithNetwork) -> AddressOwnership {
    AddressOwnershipLookup::for_active_mode().find_owners(address).unwrap()
}

fn owned(metadata: &WalletMetadata, chain: AddressChain, index: u32) -> OwnedAddress {
    OwnedAddress {
        wallet_id: metadata.id.clone(),
        wallet_name: metadata.name.clone(),
        network: metadata.network,
        chain,
        index,
        account: Some(0),
    }
}

fn owned_by(owners: Vec<OwnedAddress>) -> AddressOwnership {
    AddressOwnership::Owned { owners }
}

fn not_found() -> AddressOwnership {
    AddressOwnership::NotFound { unchecked_wallets: Vec::new() }
}

/// Takes the global-state lock and starts every network and mode with no wallets
///
/// Other tests in this binary leave wallets behind, and the lookup searches every network, so
/// the guard clears them all and restores them when the test ends
fn isolated_wallets(
    network: Network,
    mode: WalletMode,
) -> (tokio::sync::MutexGuard<'static, ()>, WalletTargetGuard) {
    let lock = crate::test_support::global_state_test_lock().blocking_lock();
    crate::test_support::ensure_tokio_runtime();
    crate::database::test_support::init_test_database();
    crate::test_support::init_test_keychain();

    let target = WalletTargetGuard::new(network, mode);
    for (network, mode) in every_scope() {
        Database::global().wallets.save_all_wallets(network, mode, Vec::new()).unwrap();
    }

    (lock, target)
}

/// Reveal one address chain through `index` in the wallet's store, as handing out addresses would
fn reveal_through(metadata: &WalletMetadata, keychain: KeychainKind, index: u32) {
    let mut store = BdkStore::try_new(&metadata.id, metadata.network).unwrap();
    let mut wallet = bdk_wallet::Wallet::load().load_wallet(&mut store.conn).unwrap().unwrap();
    let _revealed: Vec<_> = wallet.reveal_addresses_to(keychain, index).collect();
    wallet.persist(&mut store.conn).unwrap();
}

/// A watch-only wallet with only the keychain descriptor mirror and no wallet store
fn save_store_less_watch_only(
    mnemonic: Option<&Mnemonic>,
    network: Network,
    mode: WalletMode,
) -> WalletMetadata {
    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::new();
    metadata.name = format!("Watch-only {}", metadata.id);
    metadata.network = network;
    metadata.wallet_mode = mode;
    metadata.address_type = WalletAddressType::NativeSegwit;
    metadata.wallet_type = WalletType::WatchOnly;

    if let Some(mnemonic) = mnemonic {
        Keychain::global()
            .save_public_descriptor(
                &metadata.id,
                bip84_descriptor(mnemonic, network, KeychainKind::External),
                bip84_descriptor(mnemonic, network, KeychainKind::Internal),
            )
            .unwrap();
    }

    Database::global().wallets.save_new_wallet_metadata(metadata.clone()).unwrap();
    metadata
}

#[test]
fn names_the_wallet_chain_and_index_of_receive_and_change_addresses() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();

    let receive = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 3);
    assert_eq!(find_owners(&receive), owned_by(vec![owned(&wallet, AddressChain::Receive, 3)]));

    let change = address_at(&mnemonic, Network::Bitcoin, KeychainKind::Internal, 5);
    assert_eq!(find_owners(&change), owned_by(vec![owned(&wallet, AddressChain::Change, 5)]));

    let stranger = address_at(&random_mnemonic(), Network::Bitcoin, KeychainKind::External, 0);
    assert_eq!(find_owners(&stranger), not_found());
}

#[test]
fn searches_each_chain_up_to_the_gap_past_its_last_revealed_address() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();

    reveal_through(&wallet, KeychainKind::External, 200);
    reveal_through(&wallet, KeychainKind::Internal, 40);

    let cases = [
        (KeychainKind::External, AddressChain::Receive, 200),
        (KeychainKind::Internal, AddressChain::Change, 40),
    ];

    for (keychain, chain, last_revealed) in cases {
        let last_searched = last_revealed + ADDRESS_OWNERSHIP_LOOKUP_GAP;

        // revealed, past BDK's own lookahead, and the last address the gap reaches
        for index in [0, last_revealed, last_revealed + 30, last_searched] {
            let address = address_at(&mnemonic, Network::Bitcoin, keychain, index);
            assert_eq!(
                find_owners(&address),
                owned_by(vec![owned(&wallet, chain, index)]),
                "{chain:?} {index}"
            );
        }

        let past_gap = address_at(&mnemonic, Network::Bitcoin, keychain, last_searched + 1);
        assert_eq!(find_owners(&past_gap), not_found(), "{chain:?} past the gap");
    }
}

#[test]
fn main_and_decoy_mode_never_see_each_others_wallets() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let main_mnemonic = random_mnemonic();
    let decoy_mnemonic = random_mnemonic();

    let main_wallet =
        import_mnemonic_with_target(main_mnemonic.clone(), Network::Bitcoin, WalletMode::Main)
            .unwrap();
    let decoy_wallet =
        import_mnemonic_with_target(decoy_mnemonic.clone(), Network::Bitcoin, WalletMode::Decoy)
            .unwrap();

    let main_address = address_at(&main_mnemonic, Network::Bitcoin, KeychainKind::External, 0);
    let decoy_address = address_at(&decoy_mnemonic, Network::Bitcoin, KeychainKind::External, 0);

    select_target(Network::Bitcoin, WalletMode::Main);
    assert_eq!(
        find_owners(&main_address),
        owned_by(vec![owned(&main_wallet, AddressChain::Receive, 0)])
    );
    assert_eq!(find_owners(&decoy_address), not_found(), "main mode found a decoy wallet");

    select_target(Network::Bitcoin, WalletMode::Decoy);
    assert_eq!(
        find_owners(&decoy_address),
        owned_by(vec![owned(&decoy_wallet, AddressChain::Receive, 0)])
    );
    assert_eq!(find_owners(&main_address), not_found(), "decoy mode found a main wallet");
}

#[test]
fn every_network_sharing_the_address_format_is_searched() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();

    // testnet and signet derive the same tb1 addresses from one key set
    let testnet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Testnet, WalletMode::Main).unwrap();
    let signet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Signet, WalletMode::Main).unwrap();
    let mainnet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();

    let test_address = address_at(&mnemonic, Network::Signet, KeychainKind::External, 2);
    let AddressOwnership::Owned { owners } = find_owners(&test_address) else {
        panic!("the testnet and signet wallets own the address");
    };

    let mut owner_ids: Vec<_> = owners.iter().map(|owner| owner.wallet_id.clone()).collect();
    owner_ids.sort();
    let mut expected = vec![testnet.id, signet.id];
    expected.sort();

    assert_eq!(owner_ids, expected, "the mainnet wallet cannot own a tb1 address");
    assert!(owners.iter().all(|owner| owner.chain == AddressChain::Receive && owner.index == 2));

    let main_address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 2);
    assert_eq!(
        find_owners(&main_address),
        owned_by(vec![owned(&mainnet, AddressChain::Receive, 2)])
    );
}

#[test]
fn wallets_without_a_store_are_searched_from_their_keychain_descriptors() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let watch_only =
        save_store_less_watch_only(Some(&mnemonic), Network::Bitcoin, WalletMode::Main);

    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::Internal, 7);
    assert_eq!(find_owners(&address), owned_by(vec![owned(&watch_only, AddressChain::Change, 7)]));

    // the mirror has no record of revealed addresses, so its window starts at index 0
    let past_gap = address_at(
        &mnemonic,
        Network::Bitcoin,
        KeychainKind::External,
        ADDRESS_OWNERSHIP_LOOKUP_GAP,
    );
    assert_eq!(find_owners(&past_gap), not_found());

    // the lookup never creates a store for a wallet that has none
    let store_paths = BdkStore::wallet_store_artifact_paths(&watch_only.id);
    assert!(store_paths.iter().all(|path| !path.exists()), "{store_paths:?}");
}

#[test]
fn a_wallet_with_no_descriptors_is_reported_as_unchecked() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let degraded = save_store_less_watch_only(None, Network::Bitcoin, WalletMode::Main);

    let address = address_at(&random_mnemonic(), Network::Bitcoin, KeychainKind::External, 0);
    assert_eq!(
        find_owners(&address),
        AddressOwnership::NotFound {
            unchecked_wallets: vec![UncheckedWallet {
                wallet_id: degraded.id.clone(),
                wallet_name: degraded.name,
                network: Network::Bitcoin,
            }],
        }
    );
}

#[test]
fn pasted_bitcoin_uris_are_checked_and_garbage_is_rejected() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();

    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 1);
    let uri = format!("bitcoin:{}?amount=0.001", address.address);
    let ownership = cove_tokio::task::block_on(check_address_ownership(uri)).unwrap();
    assert_eq!(ownership, owned_by(vec![owned(&wallet, AddressChain::Receive, 1)]));

    let garbage = cove_tokio::task::block_on(check_address_ownership("not an address".into()));
    assert!(matches!(garbage, Err(AddressOwnershipError::InvalidAddress(_))), "{garbage:?}");
}

#[test]
fn a_wallet_scoped_lookup_stays_on_that_wallets_network_and_active_mode() {
    let (_lock, _target) = isolated_wallets(Network::Signet, WalletMode::Main);
    let mnemonic = random_mnemonic();

    let testnet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Testnet, WalletMode::Main).unwrap();
    let signet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Signet, WalletMode::Main).unwrap();
    let decoy =
        import_mnemonic_with_target(random_mnemonic(), Network::Signet, WalletMode::Decoy).unwrap();

    // transaction outputs carry scripts, not addresses
    let script =
        address_at(&mnemonic, Network::Signet, KeychainKind::Internal, 4).address.script_pubkey();

    let signet_scope = AddressOwnershipLookup::for_wallet(&signet);
    assert_eq!(
        signet_scope.find_script_owners(&script).unwrap(),
        owned_by(vec![owned(&signet, AddressChain::Change, 4)]),
        "the testnet wallet shares the script but not the network"
    );

    let testnet_scope = AddressOwnershipLookup::for_wallet(&testnet);
    assert_eq!(
        testnet_scope.find_script_owners(&script).unwrap(),
        owned_by(vec![owned(&testnet, AddressChain::Change, 4)])
    );

    // a decoy wallet's scope is refused while main mode is active, so it cannot list main wallets
    let decoy_scope = AddressOwnershipLookup::for_wallet(&decoy);
    assert_eq!(
        decoy_scope.find_script_owners(&script),
        Err(AddressOwnershipError::WalletModeChanged)
    );
}

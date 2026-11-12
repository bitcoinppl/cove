use bdk_wallet::{
    KeychainKind,
    bitcoin::bip32::Xpriv,
    descriptor::ExtendedDescriptor,
    descriptor::IntoWalletDescriptor as _,
    template::{Bip84, DescriptorTemplate as _},
};
use bip39::Mnemonic;
use bitcoin::secp256k1::Secp256k1;
use cove_device::keychain::Keychain;
use cove_types::address::AddressWithNetwork;

use super::{
    ADDRESS_OWNERSHIP_LOOKUP_GAP, AddressChain, AddressOwnership, AddressOwnershipError,
    AddressOwnershipLookup, OwnedAddress, UncheckedWallet, check_address_ownership,
    check_address_ownership_with,
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
    AddressOwnershipLookup::for_active_mode().unwrap().find_owners(address).unwrap()
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

/// A descriptor for any BIP 84 account, parsed the way imported descriptors are
fn bip84_account_descriptor(
    mnemonic: &Mnemonic,
    network: Network,
    account: u32,
    keychain: KeychainKind,
) -> ExtendedDescriptor {
    let bitcoin_network = bitcoin::Network::from(network);
    let xprv = Xpriv::new_master(bitcoin_network, &mnemonic.to_seed("")).unwrap();
    let coin_type = u32::from(network != Network::Bitcoin);
    let chain = match keychain {
        KeychainKind::External => 0,
        KeychainKind::Internal => 1,
    };

    let (descriptor, _) = format!("wpkh({xprv}/84'/{coin_type}'/{account}'/{chain}/*)")
        .into_wallet_descriptor(&Secp256k1::new(), bitcoin_network.into())
        .unwrap();

    descriptor
}

fn bip84_pair(mnemonic: &Mnemonic, network: Network) -> (ExtendedDescriptor, ExtendedDescriptor) {
    (
        bip84_descriptor(mnemonic, network, KeychainKind::External),
        bip84_descriptor(mnemonic, network, KeychainKind::Internal),
    )
}

/// A watch-only wallet with only the keychain descriptor mirror and no wallet store
fn save_store_less_watch_only(
    descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
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

    if let Some((external, internal)) = descriptors {
        Keychain::global().save_public_descriptor(&metadata.id, external, internal).unwrap();
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
    let watch_only = save_store_less_watch_only(
        Some(bip84_pair(&mnemonic, Network::Bitcoin)),
        Network::Bitcoin,
        WalletMode::Main,
    );

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
fn a_wallet_still_on_the_legacy_file_store_is_reported_as_unchecked() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let legacy = save_store_less_watch_only(
        Some(bip84_pair(&mnemonic, Network::Bitcoin)),
        Network::Bitcoin,
        WalletMode::Main,
    );

    // only opening the wallet migrates its file store, which may hold addresses past the gap
    let [file_store_path, ..] = BdkStore::wallet_store_artifact_paths(&legacy.id);
    std::fs::write(&file_store_path, b"legacy").unwrap();

    // the keychain mirror would find index 0, but it must not stand in for the unmigrated store
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 0);
    let ownership = find_owners(&address);
    std::fs::remove_file(&file_store_path).unwrap();

    assert_eq!(
        ownership,
        AddressOwnership::NotFound { unchecked_wallets: vec![UncheckedWallet::from(&legacy)] }
    );
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
        signet_scope.find_scripts_owners(std::slice::from_ref(&script)).unwrap(),
        vec![owned_by(vec![owned(&signet, AddressChain::Change, 4)])],
        "the testnet wallet shares the script but not the network"
    );

    // a batch, such as a transaction's outputs, answers each script in the order given
    let receive =
        address_at(&mnemonic, Network::Signet, KeychainKind::External, 2).address.script_pubkey();
    let stranger = address_at(&random_mnemonic(), Network::Signet, KeychainKind::External, 0)
        .address
        .script_pubkey();

    assert_eq!(
        signet_scope.find_scripts_owners(&[stranger, script.clone(), receive]).unwrap(),
        vec![
            not_found(),
            owned_by(vec![owned(&signet, AddressChain::Change, 4)]),
            owned_by(vec![owned(&signet, AddressChain::Receive, 2)]),
        ]
    );

    let testnet_scope = AddressOwnershipLookup::for_wallet(&testnet);
    assert_eq!(
        testnet_scope.find_scripts_owners(std::slice::from_ref(&script)).unwrap(),
        vec![owned_by(vec![owned(&testnet, AddressChain::Change, 4)])]
    );

    // a decoy wallet's scope is refused while main mode is active, so it cannot list main wallets
    let decoy_scope = AddressOwnershipLookup::for_wallet(&decoy);
    assert_eq!(
        decoy_scope.find_scripts_owners(std::slice::from_ref(&script)),
        Err(AddressOwnershipError::WalletModeChanged)
    );
}

#[test]
fn the_account_comes_from_the_chain_that_owns_the_address() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();

    // two-line descriptor imports may give each chain its own account
    let external = bip84_account_descriptor(&mnemonic, Network::Bitcoin, 0, KeychainKind::External);
    let internal = bip84_account_descriptor(&mnemonic, Network::Bitcoin, 1, KeychainKind::Internal);
    let change_address =
        internal.at_derivation_index(2).unwrap().address(bitcoin::Network::Bitcoin).unwrap();

    let wallet =
        save_store_less_watch_only(Some((external, internal)), Network::Bitcoin, WalletMode::Main);

    let address = AddressWithNetwork::try_new(&change_address.to_string()).unwrap();
    assert_eq!(
        find_owners(&address),
        owned_by(vec![OwnedAddress {
            account: Some(1),
            ..owned(&wallet, AddressChain::Change, 2)
        }])
    );
}

#[test]
fn a_lookup_leaves_the_wallet_store_untouched() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();
    reveal_through(&wallet, KeychainKind::External, 10);

    let [legacy_store, store, ..] = BdkStore::wallet_store_artifact_paths(&wallet.id);
    let store_before = std::fs::read(&store).unwrap();

    // past the revealed addresses, so the result proves the store, not the keychain, was read
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 105);
    assert_eq!(find_owners(&address), owned_by(vec![owned(&wallet, AddressChain::Receive, 105)]));

    assert!(std::fs::read(&store).unwrap() == store_before, "the lookup changed the wallet store");
    assert!(!legacy_store.exists(), "the lookup created a legacy file store");
}

/// Run SQL against a wallet's store, as an older or damaged install would have left it
fn alter_store(metadata: &WalletMetadata, sql: &str) {
    let store = BdkStore::try_new(&metadata.id, metadata.network).unwrap();
    store.conn.execute_batch(sql).unwrap();
}

#[test]
fn stores_on_an_older_bdk_schema_are_still_searched_past_their_revealed_addresses() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();
    reveal_through(&wallet, KeychainKind::External, 150);

    // a store last opened before BDK 3 has no locked-outpoint or derived-script tables
    alter_store(
        &wallet,
        "DROP TABLE bdk_wallet_locked_outpoints; \
         DROP TABLE bdk_descriptor_derived_spks; \
         UPDATE bdk_schemas SET version = 0 WHERE name IN ('bdk_wallet', 'bdk_keychaintxout');",
    );

    // only the store knows the chain was revealed through 150
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 240);
    assert_eq!(find_owners(&address), owned_by(vec![owned(&wallet, AddressChain::Receive, 240)]));
}

#[test]
fn an_unreadable_store_is_reported_as_unchecked_instead_of_searched_from_the_keychain() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();
    reveal_through(&wallet, KeychainKind::External, 150);
    alter_store(&wallet, "DROP TABLE bdk_wallet;");

    // the keychain mirror would search only 0 to 99 and wrongly report index 240 as not owned
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 240);
    assert_eq!(
        find_owners(&address),
        AddressOwnership::NotFound { unchecked_wallets: vec![UncheckedWallet::from(&wallet)] }
    );
}

#[test]
fn a_mode_switch_before_the_check_resumes_discards_the_result() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 0);

    // the search finds the main wallet, then decoy mode turns on before the check resumes
    let check = check_address_ownership_with(address.address.to_string(), |search| async move {
        let ownership = search();
        assert!(matches!(ownership, Ok(AddressOwnership::Owned { .. })), "{ownership:?}");

        select_target(Network::Bitcoin, WalletMode::Decoy);
        ownership
    });

    let result = cove_tokio::task::block_on(check);
    assert!(matches!(result, Err(AddressOwnershipError::WalletModeChanged)), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_store_path_that_cannot_be_checked_is_reported_as_unchecked() {
    let (_lock, _target) = isolated_wallets(Network::Bitcoin, WalletMode::Main);
    let mnemonic = random_mnemonic();
    let wallet =
        import_mnemonic_with_target(mnemonic.clone(), Network::Bitcoin, WalletMode::Main).unwrap();

    // a symlink to itself fails every metadata check, as an unreadable directory would
    let store_path = crate::bdk_store::sqlite_data_path(&wallet.id);
    let _ = std::fs::remove_file(&store_path);
    std::os::unix::fs::symlink(&store_path, &store_path).unwrap();

    // the keychain mirror would find index 0, but it must not stand in for a store that may exist
    let address = address_at(&mnemonic, Network::Bitcoin, KeychainKind::External, 0);
    let ownership = find_owners(&address);
    std::fs::remove_file(&store_path).unwrap();

    assert_eq!(
        ownership,
        AddressOwnership::NotFound { unchecked_wallets: vec![UncheckedWallet::from(&wallet)] }
    );
}

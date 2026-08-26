use std::sync::Arc;

use bdk_wallet::KeychainKind;
use bdk_wallet::chain::spk_client::FullScanRequest;
use bdk_wallet::descriptor::ExtendedDescriptor;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_common::consts::GAP_LIMIT;
use cove_types::address::AddressInfoWithDerivation;
use cove_util::result_ext::ResultExt as _;
use tracing::{debug, warn};

use crate::{
    bdk_store::{BdkStore, StoreReplacementActivation},
    keychain::Keychain,
    keys::Descriptors,
    wallet_lifecycle::{WalletLifecycleCoordinator, WalletPersistenceOperation},
    wallet_secret::WalletSecretExt as _,
};

use super::{
    AddressInfo, AddressTypeSwitchMetadata, AddressTypeSwitchOutcome, Wallet, WalletAddressType,
    WalletError, WalletStorage,
    metadata::{DiscoveryState, WalletMetadata},
};

/// Heal wallet metadata whose address type lags the persisted store
///
/// An address-type switch publishes the replacement store with one atomic
/// rename and only then commits metadata, so a crash between the two leaves
/// metadata behind. The store is the durable truth: move metadata forward to
/// match it and clear the scan lifecycle so the new descriptors get a full scan
pub(crate) fn healed_metadata_for_store(
    mut metadata: WalletMetadata,
    wallet: &bdk_wallet::Wallet,
) -> WalletMetadata {
    let external_descriptor = wallet.public_descriptor(KeychainKind::External);
    let Some(store_address_type) =
        address_type_for_descriptor_type(external_descriptor.desc_type())
    else {
        return metadata;
    };

    if store_address_type == metadata.address_type {
        return metadata;
    }

    warn!(
        id = %metadata.id,
        from = %metadata.address_type,
        to = %store_address_type,
        "wallet store does not match the metadata address type; healing metadata forward"
    );

    metadata.address_type = store_address_type;
    metadata.discovery_state = DiscoveryState::ChoseAdressType;
    let descriptor = crate::keys::Descriptor::from(external_descriptor.clone());
    if let Ok(origin) = descriptor.full_origin() {
        metadata.origin = Some(origin);
    }
    metadata.internal.reset_local_scan_state();

    if let Err(error) =
        crate::database::Database::global().wallets.replace_wallet_metadata(metadata.clone())
    {
        warn!(%error, "unable to persist healed wallet metadata; the next load retries");
    }

    metadata
}

/// Return the descriptor pair published in the wallet store
///
/// The BDK store is authoritative after an address-type switch. A matching
/// keychain pair is only a recovery fallback when the store cannot be loaded
pub(crate) fn authoritative_public_descriptors(
    metadata: &WalletMetadata,
) -> Result<(ExtendedDescriptor, ExtendedDescriptor), WalletError> {
    authoritative_public_descriptor_mirror(metadata)?.ok_or_else(|| {
        WalletError::LoadError(
            "wallet store and matching keychain descriptors are unavailable".to_string(),
        )
    })
}

/// Return the published descriptor mirror when one can be recovered safely
///
/// A missing store and keychain pair is not an error for backup compatibility
/// A stale keychain pair is an error because using it would publish addresses
/// that conflict with the wallet metadata
pub(crate) fn authoritative_public_descriptor_mirror(
    metadata: &WalletMetadata,
) -> Result<Option<(ExtendedDescriptor, ExtendedDescriptor)>, WalletError> {
    let operation = WalletLifecycleCoordinator::global()
        .begin_persistence_operation(metadata.id.clone())
        .map_err_str(WalletError::LoadError)?;

    match load_persistent_store_with_persistence_operation(
        &metadata.id,
        metadata.network,
        &operation,
    ) {
        Ok((wallet, _storage)) => {
            heal_public_descriptors_for_store(&metadata.id, &wallet);

            Ok(Some(public_descriptors(&wallet)))
        }
        Err(load_error) => {
            let keychain_pair = Keychain::global().get_public_descriptor(&metadata.id)?;
            let Some((external, internal)) = keychain_pair else {
                return Ok(None);
            };

            if descriptor_pair_matches_address_type(&external, &internal, metadata.address_type) {
                return Ok(Some((external, internal)));
            }

            Err(WalletError::LoadError(format!(
                "{load_error}; keychain descriptors do not match published wallet metadata"
            )))
        }
    }
}

/// Retry the keychain mirror from the descriptor pair published in the store
pub(crate) fn heal_public_descriptors_for_store(
    wallet_id: &crate::wallet::metadata::WalletId,
    wallet: &bdk_wallet::Wallet,
) {
    let published = public_descriptors(wallet);
    let keychain = Keychain::global();
    if matches!(keychain.get_public_descriptor(wallet_id), Ok(Some(current)) if current == published)
    {
        return;
    }

    if let Err(error) = keychain.save_public_descriptor(wallet_id, published.0, published.1) {
        warn!(id = %wallet_id, %error, "unable to heal wallet descriptors; the next load retries");
    }
}

fn public_descriptors(wallet: &bdk_wallet::Wallet) -> (ExtendedDescriptor, ExtendedDescriptor) {
    (
        wallet.public_descriptor(KeychainKind::External).clone(),
        wallet.public_descriptor(KeychainKind::Internal).clone(),
    )
}

fn descriptor_pair_matches_address_type(
    external: &ExtendedDescriptor,
    internal: &ExtendedDescriptor,
    address_type: WalletAddressType,
) -> bool {
    address_type_for_descriptor_type(external.desc_type()) == Some(address_type)
        && address_type_for_descriptor_type(internal.desc_type()) == Some(address_type)
}

/// The Cove address type a descriptor produces, when Cove supports it
fn address_type_for_descriptor_type(
    descriptor_type: bdk_wallet::miniscript::descriptor::DescriptorType,
) -> Option<WalletAddressType> {
    use bdk_wallet::miniscript::descriptor::DescriptorType;

    match descriptor_type {
        DescriptorType::Wpkh => Some(WalletAddressType::NativeSegwit),
        DescriptorType::ShWpkh => Some(WalletAddressType::WrappedSegwit),
        DescriptorType::Pkh => Some(WalletAddressType::Legacy),
        _ => None,
    }
}

impl Wallet {
    pub(crate) fn start_receive_prioritized_full_scan(&self) -> FullScanRequest<KeychainKind> {
        receive_prioritized_full_scan_request(&self.bdk)
    }

    /// The user imported a hww and wants to switch from native segwit to a different address type
    pub(crate) fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitchOutcome, WalletError> {
        debug!("switching public descriptor wallet to new address type: {address_type:?}");

        let descriptors: Descriptors = descriptors.into();
        self.switch_to_descriptors(descriptors)
    }

    /// The user imported a hot wallet and wants to switch from native segwit to a different address type
    pub(crate) fn switch_private_wallet_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitchOutcome, WalletError> {
        debug!("switching private wallet to new address type");

        let secret = Keychain::global()
            .get_wallet_secret(&self.id)
            .ok()
            .flatten()
            .ok_or(WalletError::WalletNotFound)?;

        let descriptors = secret.into_descriptors(self.network, address_type);
        self.switch_to_descriptors(descriptors)
    }

    fn switch_to_descriptors(
        &mut self,
        descriptors: Descriptors,
    ) -> Result<AddressTypeSwitchOutcome, WalletError> {
        let switch_metadata = address_type_switch_metadata_from_descriptors(&descriptors);

        if !self.uses_persistent_storage() {
            let (bdk, storage) = in_memory_wallet(&self.id, &descriptors, self.network)?;
            self.bdk = bdk;
            self.storage = storage;
            return Ok(AddressTypeSwitchOutcome {
                metadata: switch_metadata,
                persistence: crate::wallet::AddressTypeSwitchPersistence::InMemory,
                durability_error: None,
                store_reload_error: None,
            });
        }

        // build the replacement fully before touching the live store
        let prepared = BdkStore::prepare_replacement(&self.id, |conn| {
            descriptors
                .clone()
                .into_create_params()
                .network(self.network.into())
                .create_wallet(conn)
                .map(|_| ())
                .map_err(|error| eyre::eyre!("unable to create replacement wallet: {error}"))
        })
        .map_err_str(WalletError::PersistError)?;

        // close the old connection so its WAL is checkpointed before the swap;
        // the old descriptor placeholder keeps the wallet usable if activation fails midway
        let old_descriptors = Descriptors {
            external: self.bdk.public_descriptor(KeychainKind::External).clone().into(),
            internal: self.bdk.public_descriptor(KeychainKind::Internal).clone().into(),
        };
        let (old_placeholder_bdk, old_placeholder_storage) =
            in_memory_wallet(&self.id, &old_descriptors, self.network)?;
        let (new_placeholder_bdk, new_placeholder_storage) =
            in_memory_wallet(&self.id, &descriptors, self.network)?;
        drop(std::mem::replace(&mut self.bdk, old_placeholder_bdk));
        drop(std::mem::replace(&mut self.storage, old_placeholder_storage));

        // activation is one atomic rename: a crash leaves either the old or the
        // new store on disk, and wallet load heals metadata to match what it finds
        let activation = prepared.activate();

        match activation {
            StoreReplacementActivation::NotPublished(error) => {
                match load_persistent_store(&self.id, self.network) {
                    Ok((bdk, storage)) => {
                        self.bdk = bdk;
                        self.storage = storage;
                    }
                    Err(load_error) => {
                        return Err(WalletError::PersistError(format!(
                            "unable to reload wallet store after failed switch: {load_error}; {error}"
                        )));
                    }
                }

                Err(WalletError::PersistError(error.to_string()))
            }
            StoreReplacementActivation::Published { durability_error } => {
                let mut durability_error = durability_error.map(|error| error.to_string());
                let reloaded_store = load_persistent_store(&self.id, self.network);
                let store_reload_error = self.apply_published_store_reload(
                    reloaded_store,
                    (new_placeholder_bdk, new_placeholder_storage),
                );

                if let Err(error) = self.persist_published_descriptors() {
                    let error = format!("unable to persist published wallet descriptors: {error}");
                    durability_error = Some(match durability_error {
                        Some(existing) => format!("{existing}; {error}"),
                        None => error,
                    });
                }

                Ok(AddressTypeSwitchOutcome {
                    metadata: switch_metadata,
                    persistence: crate::wallet::AddressTypeSwitchPersistence::PublishedPersistent,
                    durability_error,
                    store_reload_error,
                })
            }
        }
    }

    fn apply_published_store_reload(
        &mut self,
        result: Result<
            (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
            WalletError,
        >,
        replacement: (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    ) -> Option<String> {
        match result {
            Ok((bdk, storage)) => {
                self.bdk = bdk;
                self.storage = storage;
                None
            }
            Err(error) => {
                // the store is published, so keep the replacement descriptors live
                self.bdk = replacement.0;
                self.storage = replacement.1;
                Some(error.to_string())
            }
        }
    }

    fn persist_published_descriptors(&self) -> Result<(), WalletError> {
        Keychain::global()
            .save_public_descriptor(
                &self.id,
                self.bdk.public_descriptor(KeychainKind::External).clone(),
                self.bdk.public_descriptor(KeychainKind::Internal).clone(),
            )
            .map_err(WalletError::from)
    }

    /// Return the next receive address and, when the presented rotation moved,
    /// the address-index value the caller must persist through the metadata owner
    pub fn get_next_address(
        &mut self,
    ) -> Result<(AddressInfoWithDerivation, Option<cove_types::AddressIndex>), WalletError> {
        const MAX_ADDRESSES: usize = (GAP_LIMIT - 5) as usize;

        let addresses: Vec<AddressInfo> = self
            .bdk
            .list_unused_addresses(KeychainKind::External)
            .take(MAX_ADDRESSES)
            .map(Into::into)
            .collect();

        // get up to 25 revealed but unused addresses
        if addresses.len() < MAX_ADDRESSES {
            let address_info =
                AddressInfo::from(self.bdk.reveal_next_address(KeychainKind::External));

            self.persist()?;

            let derivation_path =
                self.bdk.public_descriptor(KeychainKind::External).derivation_path().ok();
            let info = AddressInfoWithDerivation::new(address_info, derivation_path);
            return Ok((info, None));
        }

        // if we have already revealed 25 addresses, we cycle back to the first one
        // and present those addresses, until a next unused address is available, if we don't
        // do this we could hit the gap limit and users might use a an adddress past
        // the gap limit and not be able to see it their wallet
        //
        // note: index to use is the index of the address in the list of addresses, not the derivation index
        let index_to_use =
            if let Some(last_index) = self.metadata.internal.last_seen_address_index(&addresses) {
                (last_index + 1) % MAX_ADDRESSES
            } else {
                0
            };

        let address_info = addresses[index_to_use].clone();
        let address_index = cove_types::AddressIndex {
            last_seen_index: index_to_use as u8,
            address_list_hash: cove_util::calculate_hash(&addresses),
        };

        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();
        let address_info_with_derivation =
            AddressInfoWithDerivation::new(address_info, derivation_path);

        Ok((address_info_with_derivation, Some(address_index)))
    }

    pub fn receive_address_at_index(&self, index: u32) -> AddressInfoWithDerivation {
        let address_info = AddressInfo::from(self.bdk.peek_address(KeychainKind::External, index));
        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();

        AddressInfoWithDerivation::new(address_info, derivation_path)
    }

    pub fn receive_address_is_unused(&self, index: u32) -> bool {
        self.bdk.list_unused_addresses(KeychainKind::External).any(|address| address.index == index)
    }

    pub fn mark_receive_address_used(&mut self, index: u32) -> Result<(), WalletError> {
        if self.bdk.mark_used(KeychainKind::External, index) {
            self.persist()?;
        }

        Ok(())
    }

    pub fn unreserve_tx_change_addresses(&mut self, tx: &bdk_wallet::bitcoin::Transaction) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                self.bdk.derivation_of_spk(txout.script_pubkey.clone())
            {
                self.bdk.unmark_used(KeychainKind::Internal, index);
            }
        }
    }
}

fn in_memory_wallet(
    id: &super::metadata::WalletId,
    descriptors: &Descriptors,
    network: cove_types::Network,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    let mut store = BdkStore::in_memory(id, network).map_err_str(WalletError::LoadError)?;
    let wallet = descriptors
        .clone()
        .into_create_params()
        .network(network.into())
        .create_wallet(&mut store.conn)
        .map_err_str(WalletError::BdkError)?;

    Ok((wallet, WalletStorage::in_memory(store.conn)))
}

fn load_persistent_store(
    id: &super::metadata::WalletId,
    network: cove_types::Network,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    let store = BdkStore::try_new(id, network).map_err_str(WalletError::LoadError)?;
    load_wallet_from_store(store)
}

fn load_persistent_store_with_persistence_operation(
    id: &super::metadata::WalletId,
    network: cove_types::Network,
    operation: &WalletPersistenceOperation,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    let store = BdkStore::try_new_with_persistence_operation(id, network, operation)
        .map_err_str(WalletError::LoadError)?;
    load_wallet_from_store(store)
}

fn load_wallet_from_store(
    mut store: BdkStore,
) -> Result<
    (bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>, WalletStorage),
    WalletError,
> {
    let wallet = bdk_wallet::Wallet::load()
        .load_wallet(&mut store.conn)
        .map_err_str(WalletError::LoadError)?
        .ok_or(WalletError::WalletNotFound)?;

    Ok((wallet, WalletStorage::persistent(store.conn)))
}

/// Builds an incremental scan request that checks revealed-unused receive addresses first
///
/// The request still uses unbounded BDK SPK iterators. The progressive scanner owns stop-gap
/// enforcement, so the normal external iterator resumes from index `0` with prioritized indexes
/// filtered out instead of being capped to the gap limit
fn receive_prioritized_full_scan_request(
    wallet: &bdk_wallet::Wallet,
) -> FullScanRequest<KeychainKind> {
    let mut builder = FullScanRequest::builder().chain_tip(wallet.local_chain().tip());

    let priority_spks = wallet
        .list_unused_addresses(KeychainKind::External)
        .take(GAP_LIMIT as usize)
        .map(|address| (address.index, address.address.script_pubkey()))
        .collect::<Vec<_>>();

    let priority_indices = priority_spks.iter().map(|(index, _)| *index).collect::<Vec<_>>();

    if let Some(external_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::External) {
        let external_spks = priority_spks
            .into_iter()
            .chain(external_spks.filter(move |(index, _)| !priority_indices.contains(index)));

        builder = builder.spks_for_keychain(KeychainKind::External, external_spks);
    }

    if let Some(internal_spks) = wallet.spk_index().unbounded_spk_iter(KeychainKind::Internal) {
        builder = builder.spks_for_keychain(KeychainKind::Internal, internal_spks);
    }

    builder.build()
}

fn address_type_switch_metadata_from_descriptors(
    descriptors: &Descriptors,
) -> AddressTypeSwitchMetadata {
    AddressTypeSwitchMetadata {
        master_fingerprint: descriptors
            .fingerprint()
            .map(|fingerprint| Arc::new(fingerprint.into())),
        origin: descriptors.origin().ok(),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    impl Wallet {
        pub(crate) fn use_in_memory_placeholder_for_test(&mut self) {
            let descriptors = Descriptors {
                external: self.bdk.public_descriptor(KeychainKind::External).clone().into(),
                internal: self.bdk.public_descriptor(KeychainKind::Internal).clone().into(),
            };
            let (bdk, storage) = in_memory_wallet(&self.id, &descriptors, self.network)
                .expect("in-memory placeholder is created");

            self.bdk = bdk;
            self.storage = storage;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bdk_wallet::bitcoin::{
        Address as BdkAddress, Amount, BlockHash, Network, hashes::Hash as _,
    };
    use bdk_wallet::chain::{BlockId, ConfirmationBlockTime};
    use bdk_wallet::test_utils::{
        get_funded_wallet_wpkh, get_test_wpkh_and_change_desc, insert_anchor, insert_checkpoint,
        insert_tx,
    };
    use bip39::Mnemonic;

    use crate::keys::{Descriptor, DescriptorSecretKey};

    use super::*;

    fn test_bdk_wallet() -> bdk_wallet::Wallet {
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();

        bdk_wallet::Wallet::create(external_descriptor, internal_descriptor)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .expect("wallet is created")
    }

    #[test]
    fn published_reload_failure_keeps_live_replacement_and_reports_recovery() {
        let mut wallet = Wallet::preview_new_wallet();
        let replacement_key = DescriptorSecretKey::new(
            wallet.network,
            Mnemonic::from_str(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .expect("replacement mnemonic is valid"),
            None,
        );
        let replacement_descriptors = Descriptors {
            external: Descriptor::new_bip44(
                &replacement_key,
                KeychainKind::External,
                wallet.network,
            ),
            internal: Descriptor::new_bip44(
                &replacement_key,
                KeychainKind::Internal,
                wallet.network,
            ),
        };
        let replacement = in_memory_wallet(&wallet.id, &replacement_descriptors, wallet.network)
            .expect("replacement placeholder is created");

        let recovery_error = wallet.apply_published_store_reload(
            Err(WalletError::LoadError("injected reload failure".to_string())),
            replacement,
        );

        assert!(matches!(
            wallet.bdk.public_descriptor(KeychainKind::External).desc_type(),
            bdk_wallet::miniscript::descriptor::DescriptorType::Pkh
        ));
        assert_eq!(
            recovery_error.as_deref(),
            Some("failed to load wallet: injected reload failure")
        );
    }

    fn scan_indexes(
        request: &mut FullScanRequest<KeychainKind>,
        keychain: KeychainKind,
        count: usize,
    ) -> Vec<u32> {
        request.iter_spks(keychain).take(count).map(|(index, _)| index).collect()
    }

    fn build_tx_with_change(wallet: &mut bdk_wallet::Wallet) -> bdk_wallet::bitcoin::Psbt {
        let address = BdkAddress::from_str("bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5")
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let mut builder = wallet.build_tx();
        builder.add_recipient(address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));
        builder.finish().unwrap()
    }

    fn tx_output_index(
        wallet: &bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
        keychain: KeychainKind,
    ) -> u32 {
        tx.output
            .iter()
            .find_map(|txout| match wallet.derivation_of_spk(txout.script_pubkey.clone()) {
                Some((txout_keychain, index)) if txout_keychain == keychain => Some(index),
                _ => None,
            })
            .unwrap()
    }

    fn unused_addresses_contain(
        wallet: &bdk_wallet::Wallet,
        keychain: KeychainKind,
        index: u32,
    ) -> bool {
        wallet.list_unused_addresses(keychain).any(|address| address.index == index)
    }

    fn unreserve_tx_change_addresses(
        wallet: &mut bdk_wallet::Wallet,
        tx: &bdk_wallet::bitcoin::Transaction,
    ) {
        for txout in &tx.output {
            if let Some((KeychainKind::Internal, index)) =
                wallet.derivation_of_spk(txout.script_pubkey.clone())
            {
                wallet.unmark_used(KeychainKind::Internal, index);
            }
        }
    }

    #[test]
    fn unreserve_tx_change_addresses_releases_reserved_change_index() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_confirmed_change_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let psbt = build_tx_with_change(&mut wallet);
        let change_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::Internal);
        let block_id = BlockId { height: 1, hash: BlockHash::hash(b"confirmed change") };
        let confirmation = ConfirmationBlockTime { block_id, confirmation_time: 1 };

        insert_checkpoint(&mut wallet, block_id);
        insert_tx(&mut wallet, psbt.unsigned_tx.clone());
        insert_anchor(&mut wallet, psbt.unsigned_tx.compute_txid(), confirmation);

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::Internal, change_index));
    }

    #[test]
    fn unreserve_tx_change_addresses_keeps_self_send_receive_index_used() {
        let (mut wallet, _) = get_funded_wallet_wpkh();
        let receive_address = wallet.reveal_next_address(KeychainKind::External);

        assert!(wallet.mark_used(KeychainKind::External, receive_address.index));

        let mut builder = wallet.build_tx();
        builder.add_recipient(receive_address.address.script_pubkey(), Amount::from_sat(10_000));
        builder.fee_absolute(Amount::from_sat(1_000));

        let psbt = builder.finish().unwrap();
        let receive_index = tx_output_index(&wallet, &psbt.unsigned_tx, KeychainKind::External);

        assert_eq!(receive_address.index, receive_index);
        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));

        unreserve_tx_change_addresses(&mut wallet, &psbt.unsigned_tx);

        assert!(!unused_addresses_contain(&wallet, KeychainKind::External, receive_index));
    }

    #[test]
    fn receive_prioritized_scan_checks_revealed_unused_external_indexes_first() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 7);

        assert_eq!(indexes, vec![1, 3, 4, 0, 2, 5, 6]);
    }

    #[test]
    fn receive_prioritized_scan_deduplicates_priority_indexes_from_normal_external_scan() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 4).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        assert!(wallet.mark_used(KeychainKind::External, 2));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 10);
        let unique_indexes = indexes.iter().copied().collect::<std::collections::BTreeSet<_>>();

        assert_eq!(indexes.len(), unique_indexes.len());
    }

    #[test]
    fn receive_prioritized_scan_prefix_is_capped_at_gap_limit() {
        let mut wallet = test_bdk_wallet();
        let gap_limit = u32::from(GAP_LIMIT);
        let _ = wallet.reveal_addresses_to(KeychainKind::External, gap_limit + 2).last();
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, GAP_LIMIT as usize + 2);
        let expected_prefix = (0..gap_limit).collect::<Vec<_>>();

        assert_eq!(&indexes[..GAP_LIMIT as usize], expected_prefix.as_slice());
        assert_eq!(indexes[GAP_LIMIT as usize], gap_limit);
    }

    #[test]
    fn receive_prioritized_scan_prefix_does_not_fill_with_unrevealed_external_indexes() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let mut request = receive_prioritized_full_scan_request(&wallet);

        let indexes = scan_indexes(&mut request, KeychainKind::External, 4);

        assert_eq!(indexes, vec![1, 2, 0, 3]);
    }

    #[test]
    fn receive_prioritized_scan_keeps_internal_keychain_after_external_keychain() {
        let wallet = test_bdk_wallet();
        let request = receive_prioritized_full_scan_request(&wallet);

        assert_eq!(request.keychains(), vec![KeychainKind::External, KeychainKind::Internal]);
    }

    #[test]
    fn receive_prioritized_scan_construction_does_not_reveal_or_mark_addresses_used() {
        let mut wallet = test_bdk_wallet();
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 2).last();
        assert!(wallet.mark_used(KeychainKind::External, 0));
        let last_revealed_before = wallet.spk_index().last_revealed_indices();
        let unused_before = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        let _request = receive_prioritized_full_scan_request(&wallet);

        let last_revealed_after = wallet.spk_index().last_revealed_indices();
        let unused_after = wallet
            .list_unused_addresses(KeychainKind::External)
            .map(|address| address.index)
            .collect::<Vec<_>>();

        assert_eq!(last_revealed_after, last_revealed_before);
        assert_eq!(unused_after, unused_before);
    }
}

use std::sync::Arc;

use bdk_wallet::KeychainKind;
use bdk_wallet::chain::spk_client::FullScanRequest;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_common::consts::GAP_LIMIT;
use cove_device::keychain::WalletSecret;
use cove_types::address::AddressInfoWithDerivation;
use cove_util::result_ext::ResultExt as _;
use tracing::debug;

use crate::{
    bdk_store::BdkStore,
    keychain::Keychain,
    keys::{Descriptor, Descriptors},
};

use super::{AddressInfo, AddressTypeSwitchMetadata, Wallet, WalletAddressType, WalletError};

impl Wallet {
    pub(crate) fn start_receive_prioritized_full_scan(&self) -> FullScanRequest<KeychainKind> {
        receive_prioritized_full_scan_request(&self.bdk)
    }

    /// The user imported a hww and wants to switch from native segwit to a different address type
    pub(crate) fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitchMetadata, WalletError> {
        debug!("switching public descriptor wallet to new address type: {address_type:?}");

        let id = self.id.clone();

        let is_persistent = self.uses_persistent_storage();

        if is_persistent {
            // delete the bdk wallet filestore
            BdkStore::delete_sqlite_store(&self.id)
                .map_err_prefix("failed to delete wallet filestore", WalletError::PersistError)?;
        }

        let store = if is_persistent {
            BdkStore::try_new(&id, self.network)
        } else {
            BdkStore::in_memory(&id, self.network)
        };
        let mut db = store.map_err_str(WalletError::LoadError)?.conn;

        let descriptors: Descriptors = descriptors.into();
        let wallet = descriptors
            .into_create_params()
            .network(self.network.into())
            .create_wallet(&mut db)
            .map_err_str(WalletError::BdkError)?;

        // switch db and wallet
        self.bdk = wallet;
        self.storage = if is_persistent {
            super::WalletStorage::persistent(db)
        } else {
            super::WalletStorage::in_memory(db)
        };
        Ok(address_type_switch_metadata(self))
    }

    /// The user imported a hot wallet and wants to switch from native segwit to a different address type
    pub(crate) fn switch_private_wallet_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> Result<AddressTypeSwitchMetadata, WalletError> {
        debug!("switching private wallet to new address type");

        let previous_metadata = self.metadata.clone();

        if self.uses_persistent_storage() {
            // delete the bdk wallet filestore
            BdkStore::delete_sqlite_store(&self.id)
                .map_err_prefix("failed to delete wallet filestore", WalletError::PersistError)?;
        }

        let secret = Keychain::global()
            .get_wallet_secret(&self.id)
            .ok()
            .flatten()
            .ok_or(WalletError::WalletNotFound)?;

        let mut me = match secret {
            WalletSecret::Mnemonic(mnemonic) => Self::try_new_persisted_from_mnemonic(
                previous_metadata.clone(),
                mnemonic,
                None,
                address_type,
            )?,
            WalletSecret::Xpriv(xpriv) => {
                Self::try_new_persisted_from_xpriv(previous_metadata, xpriv, address_type)?
            }
        };

        let switch_metadata = AddressTypeSwitchMetadata {
            master_fingerprint: me.metadata.master_fingerprint.clone(),
            origin: me.metadata.origin.clone(),
        };

        // swap only wallet storage; the actor owns metadata replacement
        std::mem::swap(&mut self.bdk, &mut me.bdk);
        std::mem::swap(&mut self.storage, &mut me.storage);

        Ok(switch_metadata)
    }

    pub fn get_next_address(&mut self) -> Result<AddressInfoWithDerivation, WalletError> {
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
            return Ok(info);
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
        self.metadata.internal.set_last_seen_address_index(&addresses, index_to_use);

        let public_descriptor = self.bdk.public_descriptor(KeychainKind::External);
        let derivation_path = public_descriptor.derivation_path().ok();
        let address_info_with_derivation =
            AddressInfoWithDerivation::new(address_info, derivation_path);

        Ok(address_info_with_derivation)
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

fn address_type_switch_metadata(wallet: &Wallet) -> AddressTypeSwitchMetadata {
    let descriptors = Descriptors {
        external: Descriptor::from(wallet.bdk.public_descriptor(KeychainKind::External).clone()),
        internal: Descriptor::from(wallet.bdk.public_descriptor(KeychainKind::Internal).clone()),
    };

    AddressTypeSwitchMetadata {
        master_fingerprint: descriptors
            .fingerprint()
            .map(|fingerprint| Arc::new(fingerprint.into())),
        origin: descriptors.origin().ok(),
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

    use super::*;

    fn test_bdk_wallet() -> bdk_wallet::Wallet {
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();

        bdk_wallet::Wallet::create(external_descriptor, internal_descriptor)
            .network(Network::Regtest)
            .create_wallet_no_persist()
            .expect("wallet is created")
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

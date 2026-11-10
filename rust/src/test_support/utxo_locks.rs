//! A persisted watch-only Signet wallet whose chain a test builds offline, for the UTXO lock
//! checklist rows
//!
//! Locks live in the wallet's label database and are enforced by the wallet actor, so these
//! tests need the real persisted wallet, label database and wallet manager rather than a
//! preview wallet with an in-memory label store

use std::sync::Arc;

use bdk_wallet::{
    KeychainKind,
    bitcoin::{
        self, Amount as BdkAmount, BlockHash, OutPoint, Transaction, TxIn, TxOut, Txid,
        absolute::LockTime, hashes::Hash as _, transaction::Version,
    },
    chain::{BlockId, ConfirmationBlockTime},
    test_utils::{insert_anchor, insert_checkpoint, insert_tx},
};

use bip329::OutputRecord;

use crate::{
    database::{Database, test_support::RestoreWalletTarget, wallet_data::WalletDataDb},
    manager::wallet_manager::RustWalletManager,
    test_support::hardware_signing::{
        SIGNER_MNEMONIC, TestSigner, external_address, init_signet_globals,
    },
    wallet::Wallet,
};

/// Fee each offline spend leaves out of its outputs
const SPEND_FEE_SATS: u64 = 1_000;

/// A watch-only wallet whose confirmed chain is built transaction by transaction
pub(crate) struct OfflineChain {
    wallet: Wallet,
    anchor: ConfirmationBlockTime,
    restore: RestoreWalletTarget,
}

/// The wallet manager over an [`OfflineChain`], as the app opens it
pub(crate) struct LockTestWallet {
    pub(crate) manager: Arc<RustWalletManager>,
    pub(crate) labels: WalletDataDb,
    // dropped last so the network is restored after the manager shuts down
    _restore: RestoreWalletTarget,
}

impl OfflineChain {
    /// Resets the global database and imports an empty watch-only Signet wallet
    ///
    /// Hold `global_state_test_lock` for the whole test before calling this
    pub(crate) fn new() -> Self {
        let restore = init_signet_globals();
        let signer = TestSigner::new(SIGNER_MNEMONIC, bitcoin::Network::Signet);

        // the import keeps the bdk store open, so it is dropped before the chain loads it again
        let id = Wallet::try_new_persisted_from_xpub(signer.public_descriptor().to_string())
            .expect("watch-only wallet imports")
            .id;

        let mut wallet = Wallet::try_load_persisted(id).expect("wallet loads");
        let block = BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) };
        insert_checkpoint(&mut wallet.bdk, block);

        let anchor = ConfirmationBlockTime { block_id: block, confirmation_time: 1 };
        Self { wallet, anchor, restore }
    }

    /// Confirms one transaction paying a new receive address per amount, in output order
    pub(crate) fn receive(&mut self, amounts_sats: &[u64]) -> Vec<OutPoint> {
        let output = amounts_sats
            .iter()
            .map(|amount| TxOut {
                value: BdkAmount::from_sat(*amount),
                script_pubkey: self
                    .wallet
                    .bdk
                    .reveal_next_address(KeychainKind::External)
                    .address
                    .script_pubkey(),
            })
            .collect();

        let txid = self.confirm(Vec::new(), output);
        (0..amounts_sats.len())
            .map(|vout| OutPoint { txid, vout: vout.try_into().expect("vout fits") })
            .collect()
    }

    /// Confirms one transaction spending `outpoints` to an address outside the wallet
    pub(crate) fn spend_to_external(&mut self, outpoints: &[OutPoint]) -> Txid {
        let total: BdkAmount = outpoints
            .iter()
            .map(|outpoint| {
                self.wallet
                    .bdk
                    .get_utxo(*outpoint)
                    .expect("spent output is the wallet's")
                    .txout
                    .value
            })
            .sum();

        let input = outpoints
            .iter()
            .map(|outpoint| TxIn { previous_output: *outpoint, ..TxIn::default() })
            .collect();

        let output = vec![TxOut {
            value: total - BdkAmount::from_sat(SPEND_FEE_SATS),
            script_pubkey: external_address(bitcoin::Network::Signet).script_pubkey(),
        }];

        self.confirm(input, output)
    }

    fn confirm(&mut self, input: Vec<TxIn>, output: Vec<TxOut>) -> Txid {
        let transaction =
            Transaction { version: Version::TWO, lock_time: LockTime::ZERO, input, output };

        let txid = transaction.compute_txid();

        insert_tx(&mut self.wallet.bdk, transaction);
        insert_anchor(&mut self.wallet.bdk, txid, self.anchor);

        txid
    }

    /// Persists the chain, records the first full scan a real scan would leave, and opens the
    /// wallet manager the app would
    pub(crate) fn open(self) -> LockTestWallet {
        let Self { wallet, restore, .. } = self;

        // the manager loads the bdk store again, so this handle is released first
        let id = {
            let mut wallet = wallet;
            wallet.persist().expect("funded wallet persists");

            let mut metadata = wallet.metadata.clone();
            metadata.internal.performed_full_scan_at = Some(1);
            Database::global().wallets.replace_wallet_metadata(metadata).expect("metadata saves");

            wallet.id.clone()
        };

        let manager = RustWalletManager::try_new(id.clone()).expect("wallet manager loads");
        let labels = WalletDataDb::new_or_existing(id).expect("wallet label database opens");

        LockTestWallet { manager: Arc::new(manager), labels, _restore: restore }
    }
}

impl LockTestWallet {
    pub(crate) fn output_record(&self, outpoint: OutPoint) -> Option<OutputRecord> {
        self.labels
            .labels
            .get_output_record(outpoint)
            .expect("output record reads")
            .map(|record| record.item)
    }

    pub(crate) fn is_locked(&self, outpoint: OutPoint) -> bool {
        self.labels.labels.locked_output_outpoints().expect("locks read").contains(&outpoint)
    }

    pub(crate) fn lock(&self, outpoints: &[OutPoint]) {
        self.labels
            .labels
            .set_output_spendability_for_outpoints(outpoints.iter().copied(), false)
            .expect("outputs lock");
    }
}

use std::collections::{BTreeMap, HashSet};

use bdk_wallet::chain::{
    BlockId, CheckPoint, ConfirmationBlockTime, Indexed, TxUpdate,
    bitcoin::{OutPoint, TxOut, Txid},
    spk_client::SpkWithExpectedTxids,
};

use crate::{ProgressTracker, ScanProgress, ScanUpdate};

pub(crate) struct SpkBatcher<I> {
    spks: I,
}

impl<I> SpkBatcher<I> {
    pub(crate) fn new(spks: I) -> Self {
        Self { spks }
    }
}

impl<I> SpkBatcher<I>
where
    I: Iterator<Item = Indexed<SpkWithExpectedTxids>>,
{
    pub(crate) fn next_batch(&mut self, batch_size: usize) -> Vec<Indexed<SpkWithExpectedTxids>> {
        (0..batch_size.max(1)).map_while(|_| self.spks.next()).collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StopGapTracker {
    gap_limit: usize,
    last_revealed_index: Option<u32>,
    unused_count: usize,
    last_active_index: Option<u32>,
}

impl StopGapTracker {
    pub(crate) fn new(stop_gap: usize, last_revealed_index: Option<u32>) -> Self {
        Self {
            gap_limit: stop_gap.max(1),
            last_revealed_index,
            unused_count: 0,
            last_active_index: None,
        }
    }

    pub(crate) fn record_spk(&mut self, spk_index: u32, used: bool) -> StopGapOutcome {
        if used {
            self.last_active_index =
                Some(self.last_active_index.unwrap_or(spk_index).max(spk_index));
            self.unused_count = 0;
        } else if self.last_revealed_index.is_none_or(|last_revealed| spk_index > last_revealed) {
            self.unused_count = self.unused_count.saturating_add(1);
        }

        StopGapOutcome {
            last_active_index: self.last_active_index,
            reached_stop_gap: self.unused_count >= self.gap_limit,
        }
    }

    pub(crate) fn last_active_index(&self) -> Option<u32> {
        self.last_active_index
    }

    pub(crate) fn reached_stop_gap(&self) -> bool {
        self.unused_count >= self.gap_limit
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct StopGapOutcome {
    pub(crate) last_active_index: Option<u32>,
    pub(crate) reached_stop_gap: bool,
}

pub(crate) struct ScanAccumulator<K> {
    progress: ProgressTracker<K>,
    inserted_txs: HashSet<Txid>,
    tx_update: TxUpdate<ConfirmationBlockTime>,
    last_active_indices: BTreeMap<K, u32>,
}

impl<K> ScanAccumulator<K>
where
    K: Ord + Clone,
{
    pub(crate) fn new(stop_gap: usize) -> Self {
        Self {
            progress: ProgressTracker::new(stop_gap),
            inserted_txs: HashSet::new(),
            tx_update: TxUpdate::default(),
            last_active_indices: BTreeMap::new(),
        }
    }

    pub(crate) fn checked(&mut self, keychain: K, used: bool) -> ScanProgress<K> {
        self.progress.checked(keychain, used)
    }

    pub(crate) fn insert_txid(&mut self, txid: Txid) -> bool {
        self.inserted_txs.insert(txid)
    }

    pub(crate) fn finish_keychain(&mut self, keychain: K, result: KeychainScanResult) {
        self.tx_update.extend(result.update);
        if let Some(last_active_index) = result.last_active_index {
            self.last_active_indices.insert(keychain, last_active_index);
        }
    }

    pub(crate) fn into_response_parts(self) -> (TxUpdate<ConfirmationBlockTime>, BTreeMap<K, u32>) {
        (self.tx_update, self.last_active_indices)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct KeychainScanResult {
    pub(crate) update: TxUpdate<ConfirmationBlockTime>,
    pub(crate) last_active_index: Option<u32>,
}

impl KeychainScanResult {
    pub(crate) fn new(
        update: TxUpdate<ConfirmationBlockTime>,
        last_active_index: Option<u32>,
    ) -> Self {
        Self { update, last_active_index }
    }
}

pub(crate) fn scan_update_for_keychain<K>(
    keychain: K,
    partial_update: &TxUpdate<ConfirmationBlockTime>,
    chain_update: Option<CheckPoint>,
    last_active_index: Option<u32>,
) -> ScanUpdate<K>
where
    K: Ord,
{
    ScanUpdate {
        chain_update,
        tx_update: partial_update.clone(),
        last_active_indices: last_active_index
            .map(|index| BTreeMap::from([(keychain, index)]))
            .unwrap_or_default(),
    }
}

pub(crate) fn insert_evicted_ats(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    spk: &SpkWithExpectedTxids,
    found_txids: &HashSet<Txid>,
    start_time: u64,
) {
    insert_evicted_ats_from_expected(update, &spk.expected_txids, found_txids, start_time);
}

pub(crate) fn insert_evicted_ats_from_expected(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    expected_txids: &HashSet<Txid>,
    found_txids: &HashSet<Txid>,
    start_time: u64,
) {
    update
        .evicted_ats
        .extend(expected_txids.difference(found_txids).map(|&txid| (txid, start_time)));
}

pub(crate) enum TxStatusPlan {
    Confirmed(ConfirmationBlockTime),
    Seen,
}

pub(crate) fn insert_tx_status(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    start_time: u64,
    txid: Txid,
    status: TxStatusPlan,
) {
    match status {
        TxStatusPlan::Confirmed(anchor) => {
            update.anchors.insert((anchor, txid));
        }
        TxStatusPlan::Seen => {
            update.seen_ats.insert((txid, start_time));
        }
    }
}

pub(crate) fn confirmed_status(
    height: u32,
    hash: bdk_wallet::bitcoin::BlockHash,
    time: u64,
) -> TxStatusPlan {
    TxStatusPlan::Confirmed(ConfirmationBlockTime {
        block_id: BlockId { height, hash },
        confirmation_time: time,
    })
}

pub(crate) fn prevout_fetch_plan(update: &TxUpdate<ConfirmationBlockTime>) -> Vec<OutPoint> {
    let mut unique_txs = HashSet::<Txid>::new();
    let mut outpoints = Vec::new();

    for tx in &update.txs {
        if tx.is_coinbase() || !unique_txs.insert(tx.compute_txid()) {
            continue;
        }

        outpoints.extend(tx.input.iter().map(|input| input.previous_output));
    }

    outpoints
}

pub(crate) fn insert_prevout_txouts(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    prevouts: impl IntoIterator<Item = (OutPoint, TxOut)>,
) {
    update.txouts.extend(prevouts);
}

#[cfg(test)]
mod tests {
    use bdk_wallet::chain::bitcoin::{ScriptBuf, Transaction, absolute, transaction};
    use bdk_wallet::chain::spk_client::SpkWithExpectedTxids;

    use super::{SpkBatcher, StopGapTracker, prevout_fetch_plan};

    #[test]
    fn stop_gap_ignores_unused_indexes_through_last_revealed() {
        let mut tracker = StopGapTracker::new(2, Some(3));

        for index in 0..=3 {
            let outcome = tracker.record_spk(index, false);
            assert!(!outcome.reached_stop_gap);
        }

        assert!(!tracker.record_spk(4, false).reached_stop_gap);
        assert!(tracker.record_spk(5, false).reached_stop_gap);
    }

    #[test]
    fn stop_gap_tracks_max_active_index_and_resets_gap() {
        let mut tracker = StopGapTracker::new(2, None);

        tracker.record_spk(4, true);
        tracker.record_spk(1, true);

        assert_eq!(tracker.last_active_index(), Some(4));
        assert!(!tracker.record_spk(5, false).reached_stop_gap);
        assert!(tracker.record_spk(6, false).reached_stop_gap);
    }

    #[test]
    fn spk_batcher_uses_minimum_batch_size_of_one() {
        let spks = (0..2).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));
        let mut batcher = SpkBatcher::new(spks);

        assert_eq!(batcher.next_batch(0).len(), 1);
        assert_eq!(batcher.next_batch(0).len(), 1);
        assert!(batcher.next_batch(0).is_empty());
    }

    #[test]
    fn prevout_fetch_plan_skips_coinbase_transactions() {
        let mut update = bdk_wallet::chain::TxUpdate::default();
        update.txs.push(
            Transaction {
                version: transaction::Version::TWO,
                lock_time: absolute::LockTime::ZERO,
                input: Vec::new(),
                output: Vec::new(),
            }
            .into(),
        );

        assert!(prevout_fetch_plan(&update).is_empty());
    }
}

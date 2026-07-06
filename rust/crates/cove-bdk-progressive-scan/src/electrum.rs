use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use bdk_electrum::{
    BdkElectrumClient,
    electrum_client::{self, ElectrumApi, HeaderNotification},
};
use bdk_wallet::chain::{
    BlockId, CheckPoint, ConfirmationBlockTime, Indexed, TxGraph, TxUpdate,
    bitcoin::{BlockHash, Txid},
    spk_client::{FullScanResponse, SpkWithExpectedTxids},
};

use crate::{
    Error, ProgressiveScanner, Result, ScanEvent,
    core::{
        KeychainScanResult, ScanAccumulator, SpkBatcher, StopGapTracker, TxStatusPlan,
        insert_evicted_ats, insert_tx_status, prevout_fetch_plan, scan_update_for_keychain,
    },
    event::{clone_full_scan_response, send_complete_unless_cancelled, send_progress, send_update},
};

const CHAIN_SUFFIX_LENGTH: u32 = 8;

pub struct ProgressiveElectrumScanner<K, E> {
    scanner: ProgressiveScanner<K>,
    client: Arc<BdkElectrumClient<E>>,
    tx_graph: Option<TxGraph<ConfirmationBlockTime>>,
    batch_size: usize,
    fetch_prev_txouts: bool,
}

impl<K, E> ProgressiveElectrumScanner<K, E>
where
    K: Ord + Clone,
    E: ElectrumApi,
{
    pub fn new(
        scanner: ProgressiveScanner<K>,
        client: impl Into<Arc<BdkElectrumClient<E>>>,
    ) -> Self {
        Self {
            scanner,
            client: client.into(),
            tx_graph: None,
            batch_size: 10,
            fetch_prev_txouts: false,
        }
    }

    pub fn tx_graph(mut self, tx_graph: TxGraph<ConfirmationBlockTime>) -> Self {
        self.tx_graph = Some(tx_graph);
        self
    }

    pub fn batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }

    pub fn fetch_prev_txouts(mut self, fetch_prev_txouts: bool) -> Self {
        self.fetch_prev_txouts = fetch_prev_txouts;
        self
    }

    pub fn run(self) -> Result<FullScanResponse<K>> {
        if let Some(tx_graph) = self.tx_graph {
            self.client.populate_tx_cache(tx_graph.full_txs().map(|tx_node| tx_node.tx));
        }

        let parts = self.scanner.into_parts();
        let mut request = parts.request;
        let start_time = request.start_time();
        let tip_and_latest_blocks = match request.chain_tip() {
            Some(chain_tip) => Some(fetch_tip_and_latest_blocks(&self.client.inner, chain_tip)?),
            None => None,
        };

        let mut scan = ScanAccumulator::<K>::new(parts.stop_gap);
        for keychain in request.keychains() {
            if parts.cancel_token.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let spks = request
                .iter_spks(keychain.clone())
                .map(|(spk_i, spk)| (spk_i, SpkWithExpectedTxids::from(spk)));
            let keychain_result = populate_with_spks(
                self.client.as_ref(),
                start_time,
                &parts.events,
                tip_and_latest_blocks.as_ref(),
                &parts.cancel_token,
                &mut scan,
                keychain.clone(),
                spks,
                parts.last_revealed_indices.get(&keychain).copied(),
                parts.stop_gap,
                self.batch_size,
                self.fetch_prev_txouts,
            )?;
            scan.finish_keychain(keychain, keychain_result);
        }

        if parts.cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let (tx_update, last_active_indices) = scan.into_response_parts();
        let chain_update = match tip_and_latest_blocks {
            Some((chain_tip, latest_blocks)) => {
                Some(chain_update(chain_tip, &latest_blocks, tx_update.anchors.iter().cloned())?)
            }
            None => None,
        };
        let response = FullScanResponse { tx_update, chain_update, last_active_indices };

        send_complete_unless_cancelled(
            &parts.events,
            &parts.cancel_token,
            clone_full_scan_response(&response),
        )?;

        Ok(response)
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_with_spks<K, E>(
    client: &BdkElectrumClient<E>,
    start_time: u64,
    events: &flume::Sender<ScanEvent<K>>,
    tip_and_latest_blocks: Option<&(CheckPoint, BTreeMap<u32, BlockHash>)>,
    cancel_token: &tokio_util::sync::CancellationToken,
    scan: &mut ScanAccumulator<K>,
    keychain: K,
    spks_with_expected_txids: impl Iterator<Item = Indexed<SpkWithExpectedTxids>>,
    last_revealed_index: Option<u32>,
    stop_gap: usize,
    batch_size: usize,
    fetch_prev_txouts: bool,
) -> Result<KeychainScanResult>
where
    K: Ord + Clone,
    E: ElectrumApi,
{
    let mut stop_gap = StopGapTracker::new(stop_gap, last_revealed_index);
    let mut batcher = SpkBatcher::new(spks_with_expected_txids);
    let mut update = TxUpdate::<ConfirmationBlockTime>::default();

    loop {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let spks = batcher.next_batch(batch_size);
        if spks.is_empty() {
            return Ok(KeychainScanResult::new(update, stop_gap.last_active_index()));
        }

        let spk_histories = client
            .inner
            .batch_script_get_history(spks.iter().map(|(_, script)| script.spk.as_script()))?;

        if spk_histories.len() != spks.len() {
            return Err(Error::Electrum(electrum_client::Error::Message(format!(
                "electrum batch history response length mismatch: requested {}, received {}",
                spks.len(),
                spk_histories.len()
            ))));
        }

        let mut partial_update = TxUpdate::<ConfirmationBlockTime>::default();
        let mut pending_anchors = Vec::new();

        for ((spk_index, spk), spk_history) in spks.into_iter().zip(spk_histories) {
            let used = !spk_history.is_empty();
            let scan_progress = scan.checked(keychain.clone(), used);
            send_progress(events, scan_progress);
            stop_gap.record_spk(spk_index, used);

            let spk_history_set = spk_history.iter().map(|res| res.tx_hash).collect::<HashSet<_>>();
            insert_evicted_ats(&mut partial_update, &spk, &spk_history_set, start_time);

            for tx_res in spk_history {
                if !scan.insert_txid(tx_res.tx_hash) {
                    continue;
                }

                partial_update.txs.push(client.fetch_tx(tx_res.tx_hash)?);

                match tx_res.height.try_into() {
                    Ok(height) if height > 0 => pending_anchors.push((tx_res.tx_hash, height)),
                    _ => {
                        insert_tx_status(
                            &mut partial_update,
                            start_time,
                            tx_res.tx_hash,
                            TxStatusPlan::Seen,
                        );
                    }
                }
            }
        }

        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        if fetch_prev_txouts {
            fetch_prev_txout(client, &mut partial_update)?;
        }

        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        if !pending_anchors.is_empty() {
            let anchors = batch_fetch_anchors(client, &pending_anchors)?;
            for (txid, anchor) in anchors {
                insert_tx_status(
                    &mut partial_update,
                    start_time,
                    txid,
                    TxStatusPlan::Confirmed(anchor),
                );
            }
        }

        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        if !partial_update.is_empty() {
            let partial_chain_update = match tip_and_latest_blocks {
                Some((chain_tip, latest_blocks)) => Some(chain_update(
                    chain_tip.clone(),
                    latest_blocks,
                    partial_update.anchors.iter().cloned(),
                )?),
                None => None,
            };
            let scan_update = scan_update_for_keychain(
                keychain.clone(),
                &partial_update,
                partial_chain_update,
                stop_gap.last_active_index(),
            );

            send_update(events, scan_update)?;
            update.extend(partial_update);
        }

        if stop_gap.reached_stop_gap() {
            return Ok(KeychainScanResult::new(update, stop_gap.last_active_index()));
        }
    }
}

fn batch_fetch_anchors<E>(
    client: &BdkElectrumClient<E>,
    txs_with_heights: &[(Txid, usize)],
) -> std::result::Result<Vec<(Txid, ConfirmationBlockTime)>, electrum_client::Error>
where
    E: ElectrumApi,
{
    let mut results = Vec::with_capacity(txs_with_heights.len());
    let mut needed_heights =
        txs_with_heights.iter().map(|&(_, height)| height as u32).collect::<Vec<_>>();
    needed_heights.sort_unstable();
    needed_heights.dedup();

    let headers = client.inner.batch_block_header(needed_heights.iter())?;
    let height_to_header = needed_heights.into_iter().zip(headers).collect::<HashMap<u32, _>>();
    let proofs = client.inner.batch_transaction_get_merkle(txs_with_heights.iter())?;
    if proofs.len() != txs_with_heights.len() {
        return Err(electrum_client::Error::Message(format!(
            "merkle proof batch returned {} proofs for {} requested transactions",
            proofs.len(),
            txs_with_heights.len()
        )));
    }

    for ((txid, height), proof) in txs_with_heights.iter().copied().zip(proofs) {
        let mut header = *height_to_header.get(&(height as u32)).ok_or_else(|| {
            electrum_client::Error::Message(format!(
                "block header for height {height} not returned by server"
            ))
        })?;
        let mut valid =
            electrum_client::utils::validate_merkle_proof(&txid, &header.merkle_root, &proof);
        if !valid {
            header = client.inner.block_header(height)?;
            valid =
                electrum_client::utils::validate_merkle_proof(&txid, &header.merkle_root, &proof);
        }

        if valid {
            let hash = header.block_hash();
            let anchor = ConfirmationBlockTime {
                confirmation_time: header.time as u64,
                block_id: BlockId { height: height as u32, hash },
            };
            results.push((txid, anchor));
        }
    }

    Ok(results)
}

fn fetch_prev_txout<E>(
    client: &BdkElectrumClient<E>,
    tx_update: &mut TxUpdate<ConfirmationBlockTime>,
) -> std::result::Result<(), electrum_client::Error>
where
    E: ElectrumApi,
{
    for outpoint in prevout_fetch_plan(tx_update) {
        let prev_tx = client.fetch_tx(outpoint.txid)?;
        let txout = prev_tx
            .output
            .get(outpoint.vout as usize)
            .ok_or_else(|| {
                electrum_client::Error::Message(format!("prevout {outpoint} does not exist"))
            })?
            .clone();
        tx_update.txouts.insert(outpoint, txout);
    }
    Ok(())
}

fn fetch_tip_and_latest_blocks(
    client: &impl ElectrumApi,
    prev_tip: CheckPoint,
) -> std::result::Result<(CheckPoint, BTreeMap<u32, BlockHash>), electrum_client::Error> {
    let HeaderNotification { height, .. } = client.block_headers_subscribe()?;
    let new_tip_height = height as u32;

    if new_tip_height < prev_tip.height() {
        return Ok((prev_tip, BTreeMap::new()));
    }

    let mut new_blocks = {
        let start_height = new_tip_height.saturating_sub(CHAIN_SUFFIX_LENGTH - 1);
        let hashes = client
            .block_headers(start_height as _, CHAIN_SUFFIX_LENGTH as _)?
            .headers
            .into_iter()
            .map(|header| header.block_hash());
        (start_height..).zip(hashes).collect::<BTreeMap<u32, _>>()
    };

    let agreement_cp = {
        let mut agreement_cp = Option::<CheckPoint>::None;
        for cp in prev_tip.iter() {
            let cp_block = cp.block_id();
            let hash = match new_blocks.get(&cp_block.height) {
                Some(&hash) => hash,
                None => {
                    assert!(
                        new_tip_height >= cp_block.height,
                        "already checked that electrum's tip cannot be smaller"
                    );
                    let hash = client.block_header(cp_block.height as _)?.block_hash();
                    new_blocks.insert(cp_block.height, hash);
                    hash
                }
            };
            if hash == cp_block.hash {
                agreement_cp = Some(cp);
                break;
            }
        }
        agreement_cp.ok_or_else(|| {
            electrum_client::Error::Message("cannot find agreement block with server".to_string())
        })?
    };

    let extension = new_blocks
        .iter()
        .filter({
            let agreement_height = agreement_cp.height();
            move |(height, _)| **height > agreement_height
        })
        .map(|(&height, &hash)| BlockId { height, hash });
    let new_tip = agreement_cp
        .extend(extension)
        .expect("extension heights already checked to be greater than agreement height");

    Ok((new_tip, new_blocks))
}

fn chain_update(
    mut tip: CheckPoint,
    latest_blocks: &BTreeMap<u32, BlockHash>,
    anchors: impl Iterator<Item = (ConfirmationBlockTime, Txid)>,
) -> std::result::Result<CheckPoint, electrum_client::Error> {
    for (anchor, _txid) in anchors {
        let height = anchor.block_id.height;
        if tip.get(height).is_none() && height <= tip.height() {
            let hash = match latest_blocks.get(&height) {
                Some(&hash) => hash,
                None => anchor.block_id.hash,
            };
            tip = tip.insert(BlockId { hash, height });
        }
    }
    Ok(tip)
}

#[cfg(test)]
mod tests {
    use std::{borrow::Borrow, collections::BTreeMap};

    use bdk_electrum::{
        BdkElectrumClient,
        electrum_client::{
            self, Batch, BroadcastPackageRes, ElectrumApi, EstimationMode, GetBalanceRes,
            GetHeadersRes, GetHistoryRes, GetMerkleRes, ListUnspentRes, MempoolInfoRes, Param,
            RawHeaderNotification, ScriptStatus, ServerFeaturesRes, TxidFromPosRes,
        },
    };
    use bdk_wallet::{
        KeychainKind,
        chain::{
            BlockId, CheckPoint, ConfirmationBlockTime, TxGraph,
            bitcoin::{Script, ScriptBuf, Transaction, Txid, block},
            spk_client::{FullScanRequest, SpkWithExpectedTxids},
        },
    };
    use tokio_util::sync::CancellationToken;

    use crate::{
        Error, ProgressiveScanner, ScanEvent,
        core::ScanAccumulator,
        test_fixtures::{
            QueuedResponse, ResponseQueue, SharedVec, block_hash, collect_events, empty_spks,
            event_channel, external_request, revealed_external_request, test_transaction, txid,
        },
    };

    use super::{batch_fetch_anchors, chain_update, populate_with_spks};

    #[derive(Debug, Clone)]
    struct FakeElectrum {
        fail_history: bool,
        histories: ResponseQueue<GetHistoryRes>,
        transactions: BTreeMap<Txid, Transaction>,
        fetched_txids: SharedVec<Txid>,
        history_response_limit: Option<usize>,
        merkle_response_limit: Option<usize>,
    }

    impl FakeElectrum {
        fn empty_history() -> Self {
            Self {
                fail_history: false,
                histories: ResponseQueue::empty(),
                transactions: BTreeMap::new(),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn history_error() -> Self {
            Self {
                fail_history: true,
                histories: ResponseQueue::empty(),
                transactions: BTreeMap::new(),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn with_mempool_transaction(tx: Transaction) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: ResponseQueue::with_responses([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    Vec::new(),
                ]),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn with_duplicate_mempool_transaction(tx: Transaction) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: ResponseQueue::with_responses([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                ]),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn with_same_mempool_transaction_on_external_and_internal_histories(
            tx: Transaction,
        ) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: ResponseQueue::with_responses([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                ]),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn with_mempool_history(txid: Txid) -> Self {
            Self {
                fail_history: false,
                histories: ResponseQueue::with_responses([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    Vec::new(),
                ]),
                transactions: BTreeMap::new(),
                fetched_txids: SharedVec::default(),
                history_response_limit: None,
                merkle_response_limit: None,
            }
        }

        fn with_history_response_limit(limit: usize) -> Self {
            Self { history_response_limit: Some(limit), ..Self::empty_history() }
        }

        fn with_merkle_response_limit(limit: usize) -> Self {
            Self { merkle_response_limit: Some(limit), ..Self::empty_history() }
        }

        fn fetched_txids(&self) -> Vec<Txid> {
            self.fetched_txids.snapshot()
        }
    }

    impl ElectrumApi for FakeElectrum {
        fn raw_call(
            &self,
            _: &str,
            _: impl IntoIterator<Item = Param>,
        ) -> Result<serde_json::Value, electrum_client::Error> {
            unreachable!()
        }

        fn batch_call(&self, _: &Batch) -> Result<Vec<serde_json::Value>, electrum_client::Error> {
            unreachable!()
        }

        fn block_headers_subscribe_raw(
            &self,
        ) -> Result<RawHeaderNotification, electrum_client::Error> {
            unreachable!()
        }

        fn block_headers_pop_raw(
            &self,
        ) -> Result<Option<RawHeaderNotification>, electrum_client::Error> {
            unreachable!()
        }

        fn block_header_raw(&self, _: usize) -> Result<Vec<u8>, electrum_client::Error> {
            unreachable!()
        }

        fn block_headers(
            &self,
            _: usize,
            _: usize,
        ) -> Result<GetHeadersRes, electrum_client::Error> {
            unreachable!()
        }

        fn estimate_fee(
            &self,
            _: usize,
            _: Option<EstimationMode>,
        ) -> Result<f64, electrum_client::Error> {
            unreachable!()
        }

        fn relay_fee(&self) -> Result<f64, electrum_client::Error> {
            unreachable!()
        }

        fn script_subscribe(
            &self,
            _: &Script,
        ) -> Result<Option<ScriptStatus>, electrum_client::Error> {
            unreachable!()
        }

        fn batch_script_subscribe<'s, I>(
            &self,
            _: I,
        ) -> Result<Vec<Option<ScriptStatus>>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<&'s Script>,
        {
            unreachable!()
        }

        fn script_unsubscribe(&self, _: &Script) -> Result<bool, electrum_client::Error> {
            unreachable!()
        }

        fn script_pop(&self, _: &Script) -> Result<Option<ScriptStatus>, electrum_client::Error> {
            unreachable!()
        }

        fn script_get_balance(&self, _: &Script) -> Result<GetBalanceRes, electrum_client::Error> {
            unreachable!()
        }

        fn batch_script_get_balance<'s, I>(
            &self,
            _: I,
        ) -> Result<Vec<GetBalanceRes>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<&'s Script>,
        {
            unreachable!()
        }

        fn script_get_history(
            &self,
            _: &Script,
        ) -> Result<Vec<GetHistoryRes>, electrum_client::Error> {
            unreachable!()
        }

        fn batch_script_get_history<'s, I>(
            &self,
            scripts: I,
        ) -> Result<Vec<Vec<GetHistoryRes>>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<&'s Script>,
        {
            if self.fail_history {
                return Err(electrum_client::Error::Message("history failed".to_string()));
            }

            let mut histories = Vec::new();
            let limit = self.history_response_limit.unwrap_or(usize::MAX);

            for _ in scripts.into_iter().take(limit) {
                let history = match self.histories.pop() {
                    QueuedResponse::Response(history) => history,
                    QueuedResponse::Error => {
                        return Err(electrum_client::Error::Message("history failed".to_string()));
                    }
                    QueuedResponse::Exhausted => Vec::new(),
                };
                histories.push(history);
            }

            Ok(histories)
        }

        fn script_list_unspent(
            &self,
            _: &Script,
        ) -> Result<Vec<ListUnspentRes>, electrum_client::Error> {
            unreachable!()
        }

        fn batch_script_list_unspent<'s, I>(
            &self,
            _: I,
        ) -> Result<Vec<Vec<ListUnspentRes>>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<&'s Script>,
        {
            unreachable!()
        }

        fn transaction_get_raw(&self, _: &Txid) -> Result<Vec<u8>, electrum_client::Error> {
            unreachable!()
        }

        fn batch_transaction_get_raw<'t, I>(
            &self,
            _: I,
        ) -> Result<Vec<Vec<u8>>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<&'t Txid>,
        {
            unreachable!()
        }

        fn batch_block_header_raw<I>(&self, _: I) -> Result<Vec<Vec<u8>>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<u32>,
        {
            unreachable!()
        }

        fn batch_block_header<I>(&self, _: I) -> Result<Vec<block::Header>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<u32>,
        {
            Ok(Vec::new())
        }

        fn batch_estimate_fee<I>(&self, _: I) -> Result<Vec<f64>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<usize>,
        {
            unreachable!()
        }

        fn transaction_broadcast_raw(&self, _: &[u8]) -> Result<Txid, electrum_client::Error> {
            unreachable!()
        }

        fn transaction_broadcast_package_raw<T: AsRef<[u8]>>(
            &self,
            _: &[T],
        ) -> Result<BroadcastPackageRes, electrum_client::Error> {
            unreachable!()
        }

        fn transaction_get_merkle(
            &self,
            _: &Txid,
            _: usize,
        ) -> Result<GetMerkleRes, electrum_client::Error> {
            unreachable!()
        }

        fn batch_transaction_get_merkle<I>(
            &self,
            txs: I,
        ) -> Result<Vec<GetMerkleRes>, electrum_client::Error>
        where
            I: IntoIterator + Clone,
            I::Item: Borrow<(Txid, usize)>,
        {
            Ok(txs
                .into_iter()
                .take(self.merkle_response_limit.unwrap_or(usize::MAX))
                .map(|tx| {
                    let (_, height) = *tx.borrow();
                    GetMerkleRes { block_height: height, pos: 0, merkle: Vec::new() }
                })
                .collect())
        }

        fn txid_from_pos(&self, _: usize, _: usize) -> Result<Txid, electrum_client::Error> {
            unreachable!()
        }

        fn txid_from_pos_with_merkle(
            &self,
            _: usize,
            _: usize,
        ) -> Result<TxidFromPosRes, electrum_client::Error> {
            unreachable!()
        }

        fn server_features(&self) -> Result<ServerFeaturesRes, electrum_client::Error> {
            unreachable!()
        }

        fn mempool_get_info(&self) -> Result<MempoolInfoRes, electrum_client::Error> {
            unreachable!()
        }

        fn ping(&self) -> Result<(), electrum_client::Error> {
            unreachable!()
        }

        fn calls_made(&self) -> Result<usize, electrum_client::Error> {
            unreachable!()
        }

        fn transaction_get(&self, txid: &Txid) -> Result<Transaction, electrum_client::Error> {
            self.fetched_txids.push(*txid);

            self.transactions
                .get(txid)
                .cloned()
                .ok_or_else(|| electrum_client::Error::Message(format!("missing tx {txid}")))
        }
    }

    #[test]
    fn missing_batch_header_returns_provider_error() {
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());

        let result = batch_fetch_anchors(&client, &[(txid(1), 2)]);

        assert!(
            matches!(result, Err(electrum_client::Error::Message(message)) if message == "block header for height 2 not returned by server")
        );
    }

    #[test]
    fn short_merkle_batch_returns_provider_error() {
        let client = BdkElectrumClient::new(FakeElectrum::with_merkle_response_limit(0));

        let result = batch_fetch_anchors(&client, &[(txid(1), 2)]);

        assert!(
            matches!(result, Err(electrum_client::Error::Message(message)) if message == "merkle proof batch returned 0 proofs for 1 requested transactions")
        );
    }

    #[test]
    fn empty_histories_emit_progress_without_updates_until_stop_gap() {
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();
        let mut scan = ScanAccumulator::new(2);
        let spks = empty_spks(5);

        let result = populate_with_spks(
            &client,
            0,
            &events,
            None,
            &cancel_token,
            &mut scan,
            "external",
            spks,
            None,
            2,
            1,
            false,
        )
        .expect("scan succeeds");

        let events = collect_events(receiver);
        assert_eq!(result.last_active_index, None);
        assert!(result.update.is_empty());
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event, ScanEvent::Progress(_))));
    }

    #[test]
    fn successful_empty_scan_sends_complete_once_without_update() {
        let request = external_request(0, 5);
        let (events, receiver) = event_channel();
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());

        let response = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run()
            .expect("scan succeeds");

        let events = collect_events(receiver);
        assert!(response.tx_update.is_empty());
        assert!(response.last_active_indices.is_empty());
        assert_eq!(
            events.iter().filter(|event| matches!(event, ScanEvent::Complete(_))).count(),
            1,
        );
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Update(_))));
    }

    #[test]
    fn revealed_range_is_scanned_before_stop_gap_extension() {
        let (request, last_revealed_indices) = revealed_external_request(3);
        let (events, receiver) = event_channel();
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());

        let response = ProgressiveScanner::builder()
            .request(request)
            .last_revealed_indices(last_revealed_indices)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run()
            .expect("scan succeeds");

        let external_progress_count = receiver
            .try_iter()
            .filter(|event| {
                matches!(
                    event,
                    ScanEvent::Progress(progress) if progress.keychain == KeychainKind::External
                )
            })
            .count();

        assert!(response.tx_update.is_empty());
        assert_eq!(external_progress_count, 6);
    }

    #[test]
    fn complete_send_failure_returns_channel_closed() {
        let request = external_request(0, 5);
        let (events, receiver) = event_channel();
        drop(receiver);
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run();

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn mempool_history_emits_update_and_final_last_active_index() {
        let request = external_request(7, 5);
        let (events, receiver) = event_channel();
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let client = BdkElectrumClient::new(FakeElectrum::with_mempool_transaction(tx));

        let response = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run()
            .expect("scan succeeds");

        let events = collect_events(receiver);
        let updates = events
            .iter()
            .filter_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(response.last_active_indices.get("external"), Some(&0));
        assert!(response.tx_update.seen_ats.contains(&(txid, 7)));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].last_active_indices.get("external"), Some(&0));
        assert!(updates[0].tx_update.seen_ats.contains(&(txid, 7)));
    }

    #[test]
    fn out_of_order_active_indexes_report_max_last_active_index() {
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let fake = FakeElectrum {
            fail_history: false,
            histories: ResponseQueue::with_responses([
                vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                Vec::new(),
                Vec::new(),
            ]),
            transactions: BTreeMap::from([(txid, tx)]),
            fetched_txids: SharedVec::default(),
            history_response_limit: None,
            merkle_response_limit: None,
        };
        let client = BdkElectrumClient::new(fake);
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();
        let mut scan = ScanAccumulator::new(2);
        let spks = [4, 1, 5, 6]
            .into_iter()
            .map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = populate_with_spks(
            &client,
            7,
            &events,
            None,
            &cancel_token,
            &mut scan,
            "external",
            spks,
            None,
            2,
            2,
            false,
        )
        .expect("scan succeeds");
        let update = receiver
            .try_iter()
            .find_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .expect("partial update is emitted");

        assert_eq!(result.last_active_index, Some(4));
        assert_eq!(update.last_active_indices.get("external"), Some(&4));
    }

    #[test]
    fn duplicate_transaction_history_fetches_and_pushes_transaction_once() {
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let fake = FakeElectrum::with_duplicate_mempool_transaction(tx);
        let client = BdkElectrumClient::new(fake.clone());
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();
        let mut scan = ScanAccumulator::new(2);
        let spks = empty_spks(2);

        let result = populate_with_spks(
            &client,
            7,
            &events,
            None,
            &cancel_token,
            &mut scan,
            "external",
            spks,
            None,
            2,
            2,
            false,
        )
        .expect("scan succeeds");
        let update = receiver
            .try_iter()
            .find_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .expect("partial update is emitted");

        assert_eq!(result.last_active_index, Some(1));
        assert_eq!(fake.fetched_txids(), vec![txid]);
        assert_eq!(update.tx_update.txs.len(), 1);
        assert_eq!(result.update.txs.len(), 1);
        assert!(update.tx_update.seen_ats.contains(&(txid, 7)));
        assert!(result.update.seen_ats.contains(&(txid, 7)));
    }

    #[test]
    fn same_transaction_on_external_and_internal_histories_fetches_and_pushes_once() {
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let fake =
            FakeElectrum::with_same_mempool_transaction_on_external_and_internal_histories(tx);
        let client = BdkElectrumClient::new(fake.clone());
        let request = FullScanRequest::builder_at(7)
            .spks_for_keychain(
                "external",
                (0..2).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .spks_for_keychain(
                "internal",
                (0..2).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, _receiver) = event_channel();

        let response = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(1)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run()
            .expect("scan succeeds");

        assert_eq!(fake.fetched_txids(), vec![txid]);
        assert_eq!(response.tx_update.txs.len(), 1);
        assert_eq!(response.last_active_indices.get("external"), Some(&0));
        assert_eq!(response.last_active_indices.get("internal"), Some(&0));
        assert!(response.tx_update.seen_ats.contains(&(txid, 7)));
    }

    #[test]
    fn tx_graph_populates_transaction_cache_before_scan() {
        let request = external_request(7, 5);
        let (events, _receiver) = event_channel();
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let mut tx_graph = TxGraph::<ConfirmationBlockTime>::default();
        let _ = tx_graph.insert_tx(tx);
        let client = BdkElectrumClient::new(FakeElectrum::with_mempool_history(txid));

        let response = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .tx_graph(tx_graph)
            .batch_size(1)
            .run()
            .expect("scan succeeds from cached transaction");

        assert!(response.tx_update.seen_ats.contains(&(txid, 7)));
    }

    #[test]
    fn chain_update_inserts_confirmed_anchor_before_tip() {
        let base = BlockId { height: 1, hash: block_hash(1) };
        let tip = BlockId { height: 3, hash: block_hash(3) };
        let anchor_hash = block_hash(2);
        let tip = CheckPoint::from_block_ids([base, tip]).expect("valid checkpoint chain");
        let latest_blocks = BTreeMap::from([(2, anchor_hash)]);
        let anchor = ConfirmationBlockTime {
            block_id: BlockId { height: 2, hash: block_hash(200) },
            confirmation_time: 123,
        };

        let update = chain_update(tip, &latest_blocks, [(anchor, txid(9))].into_iter())
            .expect("chain update succeeds");

        assert_eq!(update.get(2).map(|checkpoint| checkpoint.hash()), Some(anchor_hash));
    }

    #[test]
    fn update_send_failure_returns_channel_closed() {
        let request = external_request(7, 5);
        let (events, receiver) = event_channel();
        drop(receiver);
        let client =
            BdkElectrumClient::new(FakeElectrum::with_mempool_transaction(test_transaction()));

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run();

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn cancelled_scan_returns_cancelled_without_complete() {
        let request = external_request(0, 5);
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();
        let client = BdkElectrumClient::new(FakeElectrum::empty_history());

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .cancel_token(cancel_token)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run();

        let events = collect_events(receiver);
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn provider_error_does_not_emit_complete() {
        let request = external_request(0, 5);
        let (events, receiver) = event_channel();
        let client = BdkElectrumClient::new(FakeElectrum::history_error());

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run();

        let events = collect_events(receiver);
        assert!(matches!(result, Err(Error::Electrum(_))));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn short_batch_history_response_returns_error() {
        let request = external_request(0, 5);
        let (events, receiver) = event_channel();
        let client = BdkElectrumClient::new(FakeElectrum::with_history_response_limit(1));

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(2)
            .run();

        let events = collect_events(receiver);
        assert!(matches!(
            result,
            Err(Error::Electrum(electrum_client::Error::Message(message)))
                if message.contains("response length mismatch")
        ));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }
}

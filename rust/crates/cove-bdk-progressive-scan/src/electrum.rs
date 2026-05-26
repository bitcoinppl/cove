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
    Error, ProgressTracker, ProgressiveScanner, Result, ScanEvent, ScanUpdate,
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

        let mut tx_update = TxUpdate::<ConfirmationBlockTime>::default();
        let mut last_active_indices = BTreeMap::<K, u32>::default();
        let mut inserted_txs = HashSet::<Txid>::new();
        let mut progress = ProgressTracker::new(parts.stop_gap);
        for keychain in request.keychains() {
            if parts.cancel_token.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let spks = request
                .iter_spks(keychain.clone())
                .map(|(spk_i, spk)| (spk_i, SpkWithExpectedTxids::from(spk)));
            if let Some(last_active_index) = populate_with_spks(
                self.client.as_ref(),
                start_time,
                &parts.events,
                tip_and_latest_blocks.as_ref(),
                &parts.cancel_token,
                &mut progress,
                keychain.clone(),
                &mut inserted_txs,
                &mut tx_update,
                spks,
                parts.last_revealed_indices.get(&keychain).copied(),
                parts.stop_gap,
                self.batch_size,
                self.fetch_prev_txouts,
            )? {
                last_active_indices.insert(keychain, last_active_index);
            }
        }

        if parts.cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

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
    progress: &mut ProgressTracker<K>,
    keychain: K,
    inserted_txs: &mut HashSet<Txid>,
    tx_update: &mut TxUpdate<ConfirmationBlockTime>,
    mut spks_with_expected_txids: impl Iterator<Item = Indexed<SpkWithExpectedTxids>>,
    last_revealed_index: Option<u32>,
    stop_gap: usize,
    batch_size: usize,
    fetch_prev_txouts: bool,
) -> Result<Option<u32>>
where
    K: Ord + Clone,
    E: ElectrumApi,
{
    let mut stop_gap_unused_count = 0_usize;
    let mut last_active_index = Option::<u32>::None;
    let gap_limit = stop_gap.max(1);

    loop {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let spks =
            (0..batch_size).map_while(|_| spks_with_expected_txids.next()).collect::<Vec<_>>();
        if spks.is_empty() {
            return Ok(last_active_index);
        }

        let spk_histories = client
            .inner
            .batch_script_get_history(spks.iter().map(|(_, script)| script.spk.as_script()))?;

        let mut partial_update = TxUpdate::<ConfirmationBlockTime>::default();
        let mut pending_anchors = Vec::new();
        for ((spk_index, spk), spk_history) in spks.into_iter().zip(spk_histories) {
            let used = !spk_history.is_empty();
            let scan_progress = progress.checked(keychain.clone(), used);
            send_progress(events, scan_progress);

            if spk_history.is_empty() {
                if last_revealed_index.is_none_or(|last_revealed| spk_index > last_revealed) {
                    stop_gap_unused_count = stop_gap_unused_count.saturating_add(1);
                }
            } else {
                last_active_index = Some(spk_index);
                stop_gap_unused_count = 0;
            }

            let spk_history_set = spk_history.iter().map(|res| res.tx_hash).collect::<HashSet<_>>();
            partial_update.evicted_ats.extend(
                spk.expected_txids.difference(&spk_history_set).map(|&txid| (txid, start_time)),
            );

            for tx_res in spk_history {
                if inserted_txs.insert(tx_res.tx_hash) {
                    partial_update.txs.push(client.fetch_tx(tx_res.tx_hash)?);
                }

                match tx_res.height.try_into() {
                    Ok(height) if height > 0 => pending_anchors.push((tx_res.tx_hash, height)),
                    _ => {
                        partial_update.seen_ats.insert((tx_res.tx_hash, start_time));
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
                partial_update.anchors.insert((anchor, txid));
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
            let scan_update = ScanUpdate {
                chain_update: partial_chain_update,
                tx_update: partial_update.clone(),
                last_active_indices: last_active_index
                    .map(|index| BTreeMap::from([(keychain.clone(), index)]))
                    .unwrap_or_default(),
            };

            send_update(events, scan_update)?;
            tx_update.extend(partial_update);
        }

        if stop_gap_unused_count >= gap_limit {
            return Ok(last_active_index);
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
    let mut no_dup = HashSet::<Txid>::new();
    for tx in &tx_update.txs {
        if !tx.is_coinbase() && no_dup.insert(tx.compute_txid()) {
            for vin in &tx.input {
                let outpoint = vin.previous_output;
                let vout = outpoint.vout;
                let prev_tx = client.fetch_tx(outpoint.txid)?;
                let txout = prev_tx
                    .output
                    .get(vout as usize)
                    .ok_or_else(|| {
                        electrum_client::Error::Message(format!(
                            "prevout {outpoint} does not exist"
                        ))
                    })?
                    .clone();
                tx_update.txouts.insert(outpoint, txout);
            }
        }
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

#[cfg(test)]
mod tests {
    use std::{
        borrow::Borrow,
        collections::{BTreeMap, HashSet, VecDeque},
        str::FromStr as _,
        sync::{Arc, Mutex},
    };

    use bdk_electrum::{
        BdkElectrumClient,
        electrum_client::{
            self, Batch, BroadcastPackageRes, ElectrumApi, EstimationMode, GetBalanceRes,
            GetHeadersRes, GetHistoryRes, GetMerkleRes, ListUnspentRes, MempoolInfoRes, Param,
            RawHeaderNotification, ScriptStatus, ServerFeaturesRes, TxidFromPosRes,
        },
    };
    use bdk_wallet::{
        KeychainKind, Wallet,
        bitcoin::Network,
        chain::{
            BlockId, CheckPoint, ConfirmationBlockTime, TxGraph, TxUpdate,
            bitcoin::{
                BlockHash, Script, ScriptBuf, Transaction, Txid, absolute, block, transaction,
            },
            spk_client::{FullScanRequest, SpkWithExpectedTxids},
        },
        test_utils::get_test_wpkh_and_change_desc,
    };
    use tokio_util::sync::CancellationToken;

    use crate::{Error, ProgressTracker, ProgressiveScanner, ScanEvent};

    use super::{batch_fetch_anchors, chain_update, populate_with_spks};

    #[derive(Debug, Clone)]
    struct FakeElectrum {
        fail_history: bool,
        histories: Arc<Mutex<VecDeque<Vec<GetHistoryRes>>>>,
        transactions: BTreeMap<Txid, Transaction>,
        fetched_txids: Arc<Mutex<Vec<Txid>>>,
        merkle_response_limit: Option<usize>,
    }

    impl FakeElectrum {
        fn empty_history() -> Self {
            Self {
                fail_history: false,
                histories: Arc::new(Mutex::new(VecDeque::new())),
                transactions: BTreeMap::new(),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn history_error() -> Self {
            Self {
                fail_history: true,
                histories: Arc::new(Mutex::new(VecDeque::new())),
                transactions: BTreeMap::new(),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn with_mempool_transaction(tx: Transaction) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: Arc::new(Mutex::new(VecDeque::from([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    Vec::new(),
                ]))),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn with_duplicate_mempool_transaction(tx: Transaction) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: Arc::new(Mutex::new(VecDeque::from([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                ]))),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn with_same_mempool_transaction_on_external_and_internal_histories(
            tx: Transaction,
        ) -> Self {
            let txid = tx.compute_txid();
            Self {
                fail_history: false,
                histories: Arc::new(Mutex::new(VecDeque::from([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                ]))),
                transactions: BTreeMap::from([(txid, tx)]),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn with_mempool_history(txid: Txid) -> Self {
            Self {
                fail_history: false,
                histories: Arc::new(Mutex::new(VecDeque::from([
                    vec![GetHistoryRes { height: 0, tx_hash: txid, fee: None }],
                    Vec::new(),
                    Vec::new(),
                ]))),
                transactions: BTreeMap::new(),
                fetched_txids: Arc::new(Mutex::new(Vec::new())),
                merkle_response_limit: None,
            }
        }

        fn with_merkle_response_limit(limit: usize) -> Self {
            Self { merkle_response_limit: Some(limit), ..Self::empty_history() }
        }

        fn fetched_txids(&self) -> Vec<Txid> {
            self.fetched_txids.lock().expect("fetched txid lock not poisoned").clone()
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

            let mut histories = self.histories.lock().expect("history lock not poisoned");

            Ok(scripts.into_iter().map(|_| histories.pop_front().unwrap_or_default()).collect())
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
            self.fetched_txids.lock().expect("fetched txid lock not poisoned").push(*txid);

            self.transactions
                .get(txid)
                .cloned()
                .ok_or_else(|| electrum_client::Error::Message(format!("missing tx {txid}")))
        }
    }

    fn test_transaction() -> Transaction {
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    fn txid(byte: u8) -> Txid {
        Txid::from_str(&format!("{byte:02x}{}", "00".repeat(31))).expect("valid txid")
    }

    fn block_hash(byte: u8) -> BlockHash {
        BlockHash::from_str(&format!("{byte:02x}{}", "00".repeat(31))).expect("valid block hash")
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
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let mut tx_update = TxUpdate::default();
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let last_active_index = populate_with_spks(
            &client,
            0,
            &events,
            None,
            &cancel_token,
            &mut progress,
            "external",
            &mut inserted_txs,
            &mut tx_update,
            spks,
            None,
            2,
            1,
            false,
        )
        .expect("scan succeeds");

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(last_active_index, None);
        assert!(tx_update.is_empty());
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event, ScanEvent::Progress(_))));
    }

    #[test]
    fn successful_empty_scan_sends_complete_once_without_update() {
        let request = FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
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

        let events = receiver.try_iter().collect::<Vec<_>>();
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
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();
        let mut wallet = Wallet::create(external_descriptor, internal_descriptor)
            .network(Network::Signet)
            .create_wallet_no_persist()
            .expect("wallet is created");
        let _ = wallet.reveal_addresses_to(KeychainKind::External, 3).last();
        let last_revealed_indices = wallet.spk_index().last_revealed_indices();
        let spks = wallet
            .spk_index()
            .unbounded_spk_iter(KeychainKind::External)
            .expect("external keychain exists");
        let request =
            FullScanRequest::builder_at(0).spks_for_keychain(KeychainKind::External, spks).build();
        let (events, receiver) = flume::unbounded();
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
        let request = FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
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
        let request = FullScanRequest::builder_at(7)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
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

        let events = receiver.try_iter().collect::<Vec<_>>();
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
    fn duplicate_transaction_history_fetches_and_pushes_transaction_once() {
        let tx = test_transaction();
        let txid = tx.compute_txid();
        let fake = FakeElectrum::with_duplicate_mempool_transaction(tx);
        let client = BdkElectrumClient::new(fake.clone());
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let mut tx_update = TxUpdate::default();
        let spks = (0..2).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let last_active_index = populate_with_spks(
            &client,
            7,
            &events,
            None,
            &cancel_token,
            &mut progress,
            "external",
            &mut inserted_txs,
            &mut tx_update,
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

        assert_eq!(last_active_index, Some(1));
        assert_eq!(fake.fetched_txids(), vec![txid]);
        assert_eq!(update.tx_update.txs.len(), 1);
        assert_eq!(tx_update.txs.len(), 1);
        assert!(update.tx_update.seen_ats.contains(&(txid, 7)));
        assert!(tx_update.seen_ats.contains(&(txid, 7)));
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
        let (events, _receiver) = flume::unbounded();

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
        let request = FullScanRequest::builder_at(7)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, _receiver) = flume::unbounded();
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
        let request = FullScanRequest::builder_at(7)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
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
        let request = FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
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

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn provider_error_does_not_emit_complete() {
        let request = FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let (events, receiver) = flume::unbounded();
        let client = BdkElectrumClient::new(FakeElectrum::history_error());

        let result = ProgressiveScanner::builder()
            .request(request)
            .stop_gap(2)
            .events(events)
            .electrum(client)
            .expect("scanner builds")
            .batch_size(1)
            .run();

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(result, Err(Error::Electrum(_))));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }
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

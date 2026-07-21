use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
};

use bdk_esplora::esplora_client::{self, Sleeper};
use bdk_wallet::chain::{
    BlockId, CheckPoint, ConfirmationBlockTime, Indexed, TxUpdate,
    bitcoin::{Amount, BlockHash, OutPoint, Script, TxOut, Txid},
    spk_client::{FullScanRequest, FullScanResponse, SpkWithExpectedTxids},
};
use futures::{TryStreamExt as _, future::BoxFuture, stream::FuturesOrdered};

use crate::{
    Error, ProgressiveScanner, Result, ScanEvent,
    core::{
        KeychainScanResult, ScanAccumulator, SpkBatcher, StopGapTracker, confirmed_status,
        insert_evicted_ats_from_expected, insert_prevout_txouts, insert_tx_status,
        scan_update_for_keychain,
    },
    event::{
        clone_full_scan_response, send_complete_async_unless_cancelled, send_progress,
        send_update_async,
    },
    scanner::ProgressiveScannerParts,
};

type EsploraError = Box<esplora_client::Error>;

trait IndexedBlockSource {
    fn block_window(
        &self,
        start_height: u32,
    ) -> BoxFuture<'_, Result<BTreeMap<u32, BlockHash>, EsploraError>>;

    fn tip_height(&self) -> BoxFuture<'_, Result<u32, EsploraError>>;
}

impl<S> IndexedBlockSource for Arc<esplora_client::AsyncClient<S>>
where
    S: Sleeper + Clone + Send + Sync,
    S::Sleep: Send,
{
    fn block_window(
        &self,
        start_height: u32,
    ) -> BoxFuture<'_, Result<BTreeMap<u32, BlockHash>, EsploraError>> {
        Box::pin(async move {
            Ok(self
                .get_block_infos(Some(start_height))
                .await
                .map_err(Box::new)?
                .into_iter()
                .map(|block| (block.height, block.id))
                .collect())
        })
    }

    fn tip_height(&self) -> BoxFuture<'_, Result<u32, EsploraError>> {
        Box::pin(async move { self.get_height().await.map_err(Box::new) })
    }
}

trait EsploraScanClient: Clone {
    fn scripthash_txs<'a>(
        &'a self,
        spk: &'a Script,
        last_seen: Option<Txid>,
    ) -> BoxFuture<'a, Result<Vec<esplora_client::Tx>, EsploraError>>;

    fn chain_update<'a>(
        &'a self,
        latest_blocks: &'a BTreeMap<u32, BlockHash>,
        local_tip: &'a CheckPoint,
        anchors: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
    ) -> BoxFuture<'a, Result<CheckPoint, EsploraError>>;
}

impl<S> EsploraScanClient for Arc<esplora_client::AsyncClient<S>>
where
    S: Sleeper + Clone + Send + Sync,
    S::Sleep: Send,
{
    fn scripthash_txs<'a>(
        &'a self,
        spk: &'a Script,
        last_seen: Option<Txid>,
    ) -> BoxFuture<'a, Result<Vec<esplora_client::Tx>, EsploraError>> {
        Box::pin(
            async move { self.as_ref().scripthash_txs(spk, last_seen).await.map_err(Box::new) },
        )
    }

    fn chain_update<'a>(
        &'a self,
        latest_blocks: &'a BTreeMap<u32, BlockHash>,
        local_tip: &'a CheckPoint,
        anchors: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
    ) -> BoxFuture<'a, Result<CheckPoint, EsploraError>> {
        Box::pin(
            async move { chain_update(self.as_ref(), latest_blocks, local_tip, anchors).await },
        )
    }
}

pub struct ProgressiveEsploraScanner<K, S> {
    scanner: ProgressiveScanner<K>,
    client: Arc<esplora_client::AsyncClient<S>>,
    parallel_requests: usize,
}

impl<K, S> ProgressiveEsploraScanner<K, S>
where
    K: Ord + Clone + Send,
    S: Sleeper + Clone + Send + Sync,
    S::Sleep: Send,
{
    pub fn new(
        scanner: ProgressiveScanner<K>,
        client: impl Into<Arc<esplora_client::AsyncClient<S>>>,
    ) -> Self {
        Self { scanner, client: client.into(), parallel_requests: 4 }
    }

    pub fn parallel_requests(mut self, parallel_requests: usize) -> Self {
        self.parallel_requests = parallel_requests.max(1);
        self
    }

    pub async fn run(self) -> Result<FullScanResponse<K>> {
        let parts = self.scanner.into_parts();
        let latest_blocks = match parts.request.chain_tip() {
            Some(_) => Some(fetch_latest_blocks(&self.client).await?),
            None => None,
        };

        EsploraScanSession::run_parts(
            parts,
            self.client.clone(),
            latest_blocks,
            self.parallel_requests,
        )
        .await
    }
}

struct EsploraScanSession<K, C> {
    client: C,
    start_time: u64,
    stop_gap: usize,
    events: flume::Sender<ScanEvent<K>>,
    cancel_token: tokio_util::sync::CancellationToken,
    chain_tip: Option<CheckPoint>,
    latest_blocks: Option<BTreeMap<u32, BlockHash>>,
    parallel_requests: usize,
    scan: ScanAccumulator<K>,
}

impl<K, C> EsploraScanSession<K, C>
where
    K: Ord + Clone + Send,
    C: EsploraScanClient + Clone + Send + Sync,
{
    async fn run_parts(
        parts: ProgressiveScannerParts<K>,
        client: C,
        latest_blocks: Option<BTreeMap<u32, BlockHash>>,
        parallel_requests: usize,
    ) -> Result<FullScanResponse<K>> {
        let ProgressiveScannerParts {
            mut request,
            stop_gap,
            events,
            cancel_token,
            last_revealed_indices,
        } = parts;
        let start_time = request.start_time();
        let chain_tip = request.chain_tip();
        let scan = ScanAccumulator::new(stop_gap);
        let session = Self {
            client,
            start_time,
            stop_gap,
            events,
            cancel_token,
            chain_tip,
            latest_blocks,
            parallel_requests,
            scan,
        };

        session.run(&mut request, &last_revealed_indices).await
    }

    async fn run(
        mut self,
        request: &mut FullScanRequest<K>,
        last_revealed_indices: &BTreeMap<K, u32>,
    ) -> Result<FullScanResponse<K>> {
        for keychain in request.keychains() {
            if self.cancel_token.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let keychain_spks =
                request.iter_spks(keychain.clone()).map(|(spk_i, spk)| (spk_i, spk.into()));
            let keychain_result = self
                .scan_keychain(
                    keychain.clone(),
                    keychain_spks,
                    last_revealed_indices.get(&keychain).copied(),
                )
                .await?;
            self.scan.finish_keychain(keychain, keychain_result);
        }

        if self.cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let (tx_update, last_active_indices) = self.scan.into_response_parts();
        let chain_update = match (self.chain_tip, self.latest_blocks) {
            (Some(chain_tip), Some(latest_blocks)) => Some(
                self.client.chain_update(&latest_blocks, &chain_tip, &tx_update.anchors).await?,
            ),
            _ => None,
        };
        let response = FullScanResponse { chain_update, tx_update, last_active_indices };

        send_complete_async_unless_cancelled(
            &self.events,
            &self.cancel_token,
            clone_full_scan_response(&response),
        )
        .await?;

        Ok(response)
    }

    async fn scan_keychain<I>(
        &mut self,
        keychain: K,
        keychain_spks: I,
        last_revealed_index: Option<u32>,
    ) -> Result<KeychainScanResult>
    where
        I: Iterator<Item = Indexed<SpkWithExpectedTxids>> + Send,
    {
        type TxsOfSpkIndex = (u32, Vec<esplora_client::Tx>, HashSet<Txid>, HashSet<Txid>);

        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let mut stop_gap = StopGapTracker::new(self.stop_gap, last_revealed_index);
        let mut batcher = SpkBatcher::new(keychain_spks);

        loop {
            if self.cancel_token.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let spks = batcher.next_batch(self.parallel_requests);
            if spks.is_empty() {
                break;
            }

            let handles = spks
                .into_iter()
                .map(|(spk_index, spk)| {
                    let client = self.client.clone();
                    let expected_txids = spk.expected_txids;
                    let spk = spk.spk;
                    async move {
                        let mut last_seen = None;
                        let mut spk_txs = Vec::new();
                        loop {
                            let txs = client.scripthash_txs(&spk, last_seen).await?;
                            let tx_count = txs.len();
                            last_seen = txs.last().map(|tx| tx.txid);
                            spk_txs.extend(txs);
                            if tx_count < 25 {
                                break;
                            }
                        }
                        let got_txids = spk_txs.iter().map(|tx| tx.txid).collect::<HashSet<_>>();
                        Result::<TxsOfSpkIndex, EsploraError>::Ok((
                            spk_index,
                            spk_txs,
                            got_txids,
                            expected_txids,
                        ))
                    }
                })
                .collect::<FuturesOrdered<_>>();

            let mut partial_update = TxUpdate::<ConfirmationBlockTime>::default();
            for (index, txs, got_txids, expected_txids) in
                handles.try_collect::<Vec<TxsOfSpkIndex>>().await?
            {
                let used = !txs.is_empty();
                let scan_progress = self.scan.checked(keychain.clone(), used);
                send_progress(&self.events, scan_progress);
                stop_gap.record_spk(index, used);

                for tx in txs {
                    if !self.scan.insert_txid(tx.txid) {
                        continue;
                    }

                    partial_update.txs.push(tx.to_tx().into());
                    insert_anchor_or_seen_at_from_status(
                        &mut partial_update,
                        self.start_time,
                        tx.txid,
                        tx.status,
                    );
                    insert_prevouts(&mut partial_update, tx.vin);
                }
                insert_evicted_ats_from_expected(
                    &mut partial_update,
                    &expected_txids,
                    &got_txids,
                    self.start_time,
                );
            }

            if !partial_update.is_empty() {
                let partial_chain_update = match (&self.chain_tip, &self.latest_blocks) {
                    (Some(chain_tip), Some(latest_blocks)) => Some(
                        self.client
                            .chain_update(latest_blocks, chain_tip, &partial_update.anchors)
                            .await?,
                    ),
                    _ => None,
                };
                let scan_update = scan_update_for_keychain(
                    keychain.clone(),
                    &partial_update,
                    partial_chain_update,
                    stop_gap.last_active_index(),
                );

                send_update_async(&self.events, scan_update).await?;
                update.extend(partial_update);
            }

            if stop_gap.reached_stop_gap() {
                break;
            }
        }

        Ok(KeychainScanResult::new(update, stop_gap.last_active_index()))
    }
}

fn insert_anchor_or_seen_at_from_status(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    start_time: u64,
    txid: Txid,
    status: esplora_client::TxStatus,
) {
    if let esplora_client::TxStatus {
        confirmed: true,
        block_height: Some(height),
        block_hash: Some(hash),
        block_time: Some(time),
    } = status
    {
        insert_tx_status(update, start_time, txid, confirmed_status(height, hash, time));
    } else {
        insert_tx_status(update, start_time, txid, crate::core::TxStatusPlan::Seen);
    }
}

fn insert_prevouts(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    esplora_inputs: impl IntoIterator<Item = esplora_client::api::Vin>,
) {
    let prevouts =
        esplora_inputs.into_iter().filter_map(|vin| Some((vin.txid, vin.vout, vin.prevout?)));
    insert_prevout_txouts(
        update,
        prevouts.map(|(prev_txid, prev_vout, prev_txout)| {
            (
                OutPoint::new(prev_txid, prev_vout),
                TxOut {
                    script_pubkey: prev_txout.scriptpubkey,
                    value: Amount::from_sat(prev_txout.value),
                },
            )
        }),
    );
}

async fn fetch_latest_blocks<S>(
    client: &Arc<esplora_client::AsyncClient<S>>,
) -> Result<BTreeMap<u32, BlockHash>, EsploraError>
where
    S: Sleeper + Clone + Send + Sync,
    S::Sleep: Send,
{
    let tip_error = match client.get_block_infos(None).await {
        Ok(blocks) => {
            return Ok(blocks.into_iter().map(|block| (block.height, block.id)).collect());
        }
        Err(error) if is_not_found(&error) => Box::new(error),
        Err(error) => return Err(Box::new(error)),
    };

    fetch_latest_indexed_blocks(client).await?.ok_or(tip_error)
}

// binary-search the highest indexed block window when header tip races ahead
async fn fetch_latest_indexed_blocks(
    client: &impl IndexedBlockSource,
) -> Result<Option<BTreeMap<u32, BlockHash>>, EsploraError> {
    let header_tip = client.tip_height().await?;

    match client.block_window(header_tip).await {
        Ok(blocks) => return Ok(Some(blocks)),
        Err(error) if is_not_found(&error) => {}
        Err(error) => return Err(error),
    }

    if header_tip == 0 {
        return Ok(None);
    }

    // search [0, header_tip) for the highest height that returns a window
    let mut low = 0u32;
    let mut high = header_tip;
    let mut best = None;

    while low < high {
        let mid = low + (high - low) / 2;

        match client.block_window(mid).await {
            Ok(blocks) => {
                best = Some(blocks);
                low = mid.saturating_add(1);
            }
            Err(error) if is_not_found(&error) => {
                high = mid;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(best)
}

fn is_not_found(error: &esplora_client::Error) -> bool {
    matches!(error, esplora_client::Error::HttpResponse { status: 404, .. })
}

async fn fetch_block<S>(
    client: &esplora_client::AsyncClient<S>,
    latest_blocks: &BTreeMap<u32, BlockHash>,
    height: u32,
) -> Result<Option<BlockHash>, EsploraError>
where
    S: Sleeper,
{
    if let Some(&hash) = latest_blocks.get(&height) {
        return Ok(Some(hash));
    }

    match latest_blocks.keys().last().copied() {
        None => {
            debug_assert!(false, "`latest_blocks` should not be empty");
            return Ok(None);
        }
        Some(tip_height) => {
            if height > tip_height {
                return Ok(None);
            }
        }
    }

    Ok(Some(client.get_block_hash(height).await?))
}

async fn chain_update<S>(
    client: &esplora_client::AsyncClient<S>,
    latest_blocks: &BTreeMap<u32, BlockHash>,
    local_tip: &CheckPoint,
    anchors: &BTreeSet<(ConfirmationBlockTime, Txid)>,
) -> Result<CheckPoint, EsploraError>
where
    S: Sleeper,
{
    let mut point_of_agreement = None;
    let mut local_cp_hash = local_tip.hash();
    let mut conflicts = vec![];

    for local_cp in local_tip.iter() {
        let remote_hash = match fetch_block(client, latest_blocks, local_cp.height()).await? {
            Some(hash) => hash,
            None => continue,
        };
        if remote_hash == local_cp.hash() {
            point_of_agreement = Some(local_cp);
            break;
        }
        local_cp_hash = local_cp.hash();
        conflicts.push(BlockId { height: local_cp.height(), hash: remote_hash });
    }

    let mut tip = match point_of_agreement {
        Some(tip) => tip,
        None => {
            return Err(Box::new(esplora_client::Error::HeaderHashNotFound(local_cp_hash)));
        }
    };

    tip = tip.extend(conflicts.into_iter().rev()).expect("evicted are in order");

    for (anchor, _txid) in anchors {
        let height = anchor.block_id.height;
        if tip.get(height).is_none() {
            let hash = match fetch_block(client, latest_blocks, height).await? {
                Some(hash) => hash,
                None => continue,
            };
            tip = tip.insert(BlockId { height, hash });
        }
    }

    for (&height, &hash) in latest_blocks {
        tip = tip.insert(BlockId { height, hash });
    }

    Ok(tip)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bdk_esplora::esplora_client;
    use bdk_wallet::{
        KeychainKind,
        chain::{
            BlockId, CheckPoint, ConfirmationBlockTime, TxUpdate,
            bitcoin::{Amount, BlockHash, OutPoint, ScriptBuf, Txid},
            spk_client::{FullScanRequest, SpkWithExpectedTxids},
        },
    };
    use futures::future::BoxFuture;
    use parking_lot::Mutex;
    use tokio_util::sync::CancellationToken;

    use crate::{
        Error, ScanEvent,
        core::ScanAccumulator,
        esplora::{
            EsploraError, EsploraScanClient, EsploraScanSession, IndexedBlockSource,
            fetch_latest_indexed_blocks, insert_anchor_or_seen_at_from_status, insert_prevouts,
        },
        scanner::ProgressiveScannerParts,
        test_fixtures::{
            QueuedResponse, ResponseQueue, SharedCounter, block_hash, collect_events,
            confirmed_esplora_tx, empty_spks, esplora_input, esplora_tx, event_channel,
            external_request, revealed_external_request, txid,
        },
    };

    #[derive(Debug)]
    struct FakeBlockSource {
        tip_height: u32,
        // highest height with an indexed window; heights above this return 404
        latest_indexed: Option<u32>,
        // optional forced non-404 error for a specific height
        forced_error_at: Option<(u32, u16, String)>,
        requests: Mutex<Vec<u32>>,
    }

    impl FakeBlockSource {
        fn with_indexed_tip(tip_height: u32, latest_indexed: Option<u32>) -> Self {
            Self {
                tip_height,
                latest_indexed,
                forced_error_at: None,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn with_error_at(tip_height: u32, height: u32, status: u16, message: &str) -> Self {
            Self {
                tip_height,
                latest_indexed: None,
                forced_error_at: Some((height, status, message.to_string())),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<u32> {
            self.requests.lock().clone()
        }
    }

    impl IndexedBlockSource for FakeBlockSource {
        fn block_window(
            &self,
            start_height: u32,
        ) -> BoxFuture<'_, Result<BTreeMap<u32, BlockHash>, EsploraError>> {
            self.requests.lock().push(start_height);

            let response = if let Some((height, status, message)) = &self.forced_error_at
                && *height == start_height
            {
                Err(http_error(*status, message))
            } else {
                match self.latest_indexed {
                    Some(indexed) if start_height <= indexed => Ok(block_window(start_height)),
                    _ => Err(http_error(404, "header-only block not found")),
                }
            };

            Box::pin(async move { response })
        }

        fn tip_height(&self) -> BoxFuture<'_, Result<u32, EsploraError>> {
            Box::pin(async move { Ok(self.tip_height) })
        }
    }

    fn block_window(height: u32) -> BTreeMap<u32, BlockHash> {
        BTreeMap::from([(height, block_hash((height % 256) as u8))])
    }

    fn http_error(status: u16, message: &str) -> EsploraError {
        Box::new(esplora_client::Error::HttpResponse { status, message: message.to_string() })
    }

    #[derive(Clone, Debug)]
    struct FakeEsplora {
        responses: ResponseQueue<esplora_client::Tx>,
        chain_update_requests: SharedCounter,
    }

    impl FakeEsplora {
        fn with_responses(responses: impl IntoIterator<Item = Vec<esplora_client::Tx>>) -> Self {
            Self {
                responses: ResponseQueue::with_responses(responses),
                chain_update_requests: SharedCounter::default(),
            }
        }

        fn with_history_error() -> Self {
            Self {
                responses: ResponseQueue::with_error(),
                chain_update_requests: SharedCounter::default(),
            }
        }

        fn chain_update_count(&self) -> usize {
            self.chain_update_requests.get()
        }
    }

    impl EsploraScanClient for FakeEsplora {
        fn scripthash_txs<'a>(
            &'a self,
            _: &'a bdk_wallet::chain::bitcoin::Script,
            _: Option<Txid>,
        ) -> BoxFuture<'a, Result<Vec<esplora_client::Tx>, EsploraError>> {
            Box::pin(async move {
                match self.responses.pop() {
                    QueuedResponse::Response(txs) => Ok(txs),
                    QueuedResponse::Error => Err(Box::new(esplora_client::Error::HttpResponse {
                        status: 500,
                        message: "history failed".to_string(),
                    })),
                    QueuedResponse::Exhausted => Ok(Vec::new()),
                }
            })
        }

        fn chain_update<'a>(
            &'a self,
            _: &'a BTreeMap<u32, bdk_wallet::chain::bitcoin::BlockHash>,
            local_tip: &'a bdk_wallet::chain::CheckPoint,
            _: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
        ) -> BoxFuture<'a, Result<bdk_wallet::chain::CheckPoint, EsploraError>> {
            let chain_update_requests = self.chain_update_requests.clone();

            Box::pin(async move {
                chain_update_requests.increment();
                Ok(local_tip.clone())
            })
        }
    }

    fn keychain_session(
        client: FakeEsplora,
        events: flume::Sender<ScanEvent<&'static str>>,
    ) -> EsploraScanSession<&'static str, FakeEsplora> {
        EsploraScanSession {
            client,
            start_time: 7,
            stop_gap: 2,
            events,
            cancel_token: CancellationToken::new(),
            chain_tip: None,
            latest_blocks: None,
            parallel_requests: 1,
            scan: ScanAccumulator::new(2),
        }
    }

    fn scanner_parts<K>(
        request: FullScanRequest<K>,
        events: flume::Sender<ScanEvent<K>>,
        cancel_token: CancellationToken,
        last_revealed_indices: BTreeMap<K, u32>,
    ) -> ProgressiveScannerParts<K> {
        ProgressiveScannerParts {
            request,
            stop_gap: 2,
            events,
            cancel_token,
            last_revealed_indices,
        }
    }

    #[tokio::test]
    async fn latest_blocks_falls_back_to_latest_indexed_window() {
        let fake = FakeBlockSource::with_indexed_tip(42, Some(41));

        let blocks = fetch_latest_indexed_blocks(&fake)
            .await
            .expect("indexed-tip search succeeds")
            .expect("previous block window is indexed");

        assert_eq!(blocks, block_window(41));
        assert!(fake.requests().contains(&42));
        assert!(fake.requests().contains(&41));
        assert_eq!(*fake.requests().last().expect("at least one request"), 41);
    }

    #[tokio::test]
    async fn latest_blocks_recovers_from_long_indexing_lag() {
        // more than a fixed 10-block walk could cover
        let fake = FakeBlockSource::with_indexed_tip(100, Some(50));

        let blocks = fetch_latest_indexed_blocks(&fake)
            .await
            .expect("indexed-tip search succeeds")
            .expect("distant indexed window is found");

        assert_eq!(blocks, block_window(50));
        assert!(
            fake.requests().len() < 20,
            "binary search should avoid linear walk over the full lag: {:?}",
            fake.requests()
        );
    }

    #[tokio::test]
    async fn latest_blocks_propagates_non_not_found_error() {
        let fake = FakeBlockSource::with_error_at(42, 42, 500, "backend unavailable");

        let error =
            fetch_latest_indexed_blocks(&fake).await.expect_err("server error must be preserved");

        assert!(matches!(*error, esplora_client::Error::HttpResponse { status: 500, .. }));
        assert_eq!(fake.requests(), [42]);
    }

    #[tokio::test]
    async fn latest_blocks_returns_none_when_no_height_is_indexed() {
        let fake = FakeBlockSource::with_indexed_tip(42, None);

        let blocks =
            fetch_latest_indexed_blocks(&fake).await.expect("not-found errors are handled");

        assert!(blocks.is_none());
        assert!(fake.requests().contains(&42));
        assert!(fake.requests().contains(&0));
    }

    #[test]
    fn empty_histories_emit_progress_without_updates_until_stop_gap() {
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None))
            .expect("scan succeeds");

        let events = collect_events(receiver);
        assert!(result.update.is_empty());
        assert_eq!(result.last_active_index, None);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event, ScanEvent::Progress(_))));
    }

    #[test]
    fn mempool_history_emits_update_and_final_last_active_index() {
        let txid = txid(8);
        let fake = FakeEsplora::with_responses([vec![esplora_tx(txid)], Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None))
            .expect("scan succeeds");

        let events = collect_events(receiver);
        let updates = events
            .iter()
            .filter_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(result.last_active_index, Some(0));
        assert!(result.update.seen_ats.contains(&(txid, 7)));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].last_active_indices.get("external"), Some(&0));
        assert!(updates[0].tx_update.seen_ats.contains(&(txid, 7)));
    }

    #[test]
    fn out_of_order_active_indexes_report_max_last_active_index() {
        let higher_txid = txid(8);
        let lower_txid = txid(9);
        let fake = FakeEsplora::with_responses([
            vec![esplora_tx(higher_txid)],
            vec![esplora_tx(lower_txid)],
            Vec::new(),
            Vec::new(),
        ]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        session.parallel_requests = 2;
        let spks = [4, 1, 5, 6]
            .into_iter()
            .map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None))
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
    fn confirmed_history_emits_partial_chain_update() {
        let txid = txid(9);
        let tx = confirmed_esplora_tx(txid, 2, block_hash(2), 123);
        let fake = FakeEsplora::with_responses([vec![tx], Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        session.chain_tip = Some(CheckPoint::new(BlockId { height: 1, hash: block_hash(1) }));
        session.latest_blocks = Some(BTreeMap::new());
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None))
            .expect("scan succeeds");

        let updates = receiver
            .try_iter()
            .filter_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(result.last_active_index, Some(0));
        assert_eq!(updates.len(), 1);
        assert!(updates[0].chain_update.is_some());
        assert!(
            updates[0].tx_update.anchors.iter().any(|(_, anchored_txid)| *anchored_txid == txid)
        );
    }

    #[test]
    fn duplicate_confirmed_history_does_not_emit_duplicate_update() {
        let txid = txid(10);
        let make_tx = || confirmed_esplora_tx(txid, 2, block_hash(2), 123);
        let fake =
            FakeEsplora::with_responses([vec![make_tx()], vec![make_tx()], Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake.clone(), events);
        session.chain_tip = Some(CheckPoint::new(BlockId { height: 1, hash: block_hash(1) }));
        session.latest_blocks = Some(BTreeMap::new());
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None))
            .expect("scan succeeds");

        let updates = receiver
            .try_iter()
            .filter_map(|event| match event {
                ScanEvent::Update(update) => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(result.last_active_index, Some(1));
        assert_eq!(result.update.txs.len(), 1);
        assert_eq!(result.update.anchors.len(), 1);
        assert_eq!(updates.len(), 1);
        assert_eq!(fake.chain_update_count(), 1);
    }

    #[test]
    fn update_send_failure_returns_channel_closed() {
        let request = external_request(0, 5);
        let fake = FakeEsplora::with_responses([vec![esplora_tx(txid(8))], Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        drop(receiver);
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, BTreeMap::new()),
            fake,
            None,
            1,
        ));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn cancelled_scan_returns_cancelled_without_update() {
        let fake = FakeEsplora::with_responses([Vec::new()]);
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        session.cancel_token.cancel();
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None));

        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn provider_error_does_not_emit_update() {
        let fake = FakeEsplora::with_history_error();
        let (events, receiver) = event_channel();
        let mut session = keychain_session(fake, events);
        let spks = empty_spks(5);

        let result = futures::executor::block_on(session.scan_keychain("external", spks, None));

        assert!(matches!(result, Err(Error::Esplora(_))));
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn successful_empty_scan_sends_complete_once_without_update() {
        let request = external_request(0, 5);
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();

        let response = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, BTreeMap::new()),
            fake,
            None,
            1,
        ))
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
        let fake = FakeEsplora::with_responses(std::iter::repeat_with(Vec::new).take(16));
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();

        let response = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, last_revealed_indices),
            fake,
            None,
            1,
        ))
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
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        drop(receiver);
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, BTreeMap::new()),
            fake,
            None,
            1,
        ));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn cancelled_scan_returns_cancelled_without_complete() {
        let request = external_request(0, 5);
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let result = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, BTreeMap::new()),
            fake,
            None,
            1,
        ));

        let events = collect_events(receiver);
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn provider_error_does_not_emit_complete() {
        let request = external_request(0, 5);
        let fake = FakeEsplora::with_history_error();
        let (events, receiver) = event_channel();
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(EsploraScanSession::run_parts(
            scanner_parts(request, events, cancel_token, BTreeMap::new()),
            fake,
            None,
            1,
        ));

        let events = collect_events(receiver);
        assert!(matches!(result, Err(Error::Esplora(_))));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn confirmed_esplora_status_inserts_anchor() {
        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let txid = txid(1);
        let block_hash = block_hash(2);

        insert_anchor_or_seen_at_from_status(
            &mut update,
            100,
            txid,
            esplora_client::TxStatus {
                confirmed: true,
                block_height: Some(42),
                block_hash: Some(block_hash),
                block_time: Some(123_456),
            },
        );

        let expected_anchor = ConfirmationBlockTime {
            block_id: BlockId { height: 42, hash: block_hash },
            confirmation_time: 123_456,
        };
        assert!(update.anchors.contains(&(expected_anchor, txid)));
        assert!(update.seen_ats.is_empty());
    }

    #[test]
    fn unconfirmed_esplora_status_inserts_seen_at() {
        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let txid = txid(3);

        insert_anchor_or_seen_at_from_status(
            &mut update,
            100,
            txid,
            esplora_client::TxStatus {
                confirmed: false,
                block_height: None,
                block_hash: None,
                block_time: None,
            },
        );

        assert!(update.seen_ats.contains(&(txid, 100)));
        assert!(update.anchors.is_empty());
    }

    #[test]
    fn unconfirmed_esplora_status_with_block_fields_inserts_seen_at() {
        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let txid = txid(5);

        insert_anchor_or_seen_at_from_status(
            &mut update,
            100,
            txid,
            esplora_client::TxStatus {
                confirmed: false,
                block_height: Some(42),
                block_hash: Some(block_hash(6)),
                block_time: Some(123_456),
            },
        );

        assert!(update.seen_ats.contains(&(txid, 100)));
        assert!(update.anchors.is_empty());
    }

    #[test]
    fn incomplete_confirmed_esplora_status_inserts_seen_at() {
        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let txid = txid(4);

        insert_anchor_or_seen_at_from_status(
            &mut update,
            100,
            txid,
            esplora_client::TxStatus {
                confirmed: true,
                block_height: Some(42),
                block_hash: Some(block_hash(5)),
                block_time: None,
            },
        );

        assert!(update.seen_ats.contains(&(txid, 100)));
        assert!(update.anchors.is_empty());
    }

    #[test]
    fn insert_prevouts_inserts_available_prevout_txouts() {
        let mut update = TxUpdate::<ConfirmationBlockTime>::default();
        let prev_txid = txid(6);
        let script_pubkey = ScriptBuf::from_bytes(vec![0x51]);

        insert_prevouts(
            &mut update,
            [
                esplora_input(prev_txid, 1, Some((50_000, script_pubkey.clone()))),
                esplora_input(txid(7), 0, None),
            ],
        );

        assert_eq!(
            update.txouts.get(&OutPoint::new(prev_txid, 1)),
            Some(&bdk_wallet::chain::bitcoin::TxOut {
                value: Amount::from_sat(50_000),
                script_pubkey,
            })
        );
        assert_eq!(update.txouts.len(), 1);
    }
}

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
    Error, ProgressTracker, ProgressiveScanner, Result, ScanEvent, ScanUpdate,
    event::{
        clone_full_scan_response, send_complete_async_unless_cancelled, send_progress,
        send_update_async,
    },
};

type EsploraError = Box<esplora_client::Error>;

trait EsploraScanClient: Clone {
    fn scripthash_txs<'a>(
        &'a self,
        spk: &'a Script,
        last_seen: Option<Txid>,
    ) -> BoxFuture<'a, std::result::Result<Vec<esplora_client::Tx>, EsploraError>>;

    fn chain_update<'a>(
        &'a self,
        latest_blocks: &'a BTreeMap<u32, BlockHash>,
        local_tip: &'a CheckPoint,
        anchors: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
    ) -> BoxFuture<'a, std::result::Result<CheckPoint, EsploraError>>;
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
    ) -> BoxFuture<'a, std::result::Result<Vec<esplora_client::Tx>, EsploraError>> {
        Box::pin(
            async move { self.as_ref().scripthash_txs(spk, last_seen).await.map_err(Box::new) },
        )
    }

    fn chain_update<'a>(
        &'a self,
        latest_blocks: &'a BTreeMap<u32, BlockHash>,
        local_tip: &'a CheckPoint,
        anchors: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
    ) -> BoxFuture<'a, std::result::Result<CheckPoint, EsploraError>> {
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
        let request = parts.request;
        let chain_tip = request.chain_tip();
        let latest_blocks = match chain_tip {
            Some(_) => Some(fetch_latest_blocks(&self.client).await?),
            None => None,
        };

        run_with_esplora_client(
            request,
            parts.stop_gap,
            parts.events,
            parts.cancel_token,
            self.client.clone(),
            latest_blocks,
            parts.last_revealed_indices,
            self.parallel_requests,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_with_esplora_client<K, C>(
    mut request: FullScanRequest<K>,
    stop_gap: usize,
    events: flume::Sender<ScanEvent<K>>,
    cancel_token: tokio_util::sync::CancellationToken,
    client: C,
    latest_blocks: Option<BTreeMap<u32, BlockHash>>,
    last_revealed_indices: BTreeMap<K, u32>,
    parallel_requests: usize,
) -> Result<FullScanResponse<K>>
where
    K: Ord + Clone + Send,
    C: EsploraScanClient + Clone + Send + Sync,
{
    let start_time = request.start_time();
    let keychains = request.keychains();
    let chain_tip = request.chain_tip();
    let mut tx_update = TxUpdate::<ConfirmationBlockTime>::default();
    let mut inserted_txs = HashSet::<Txid>::new();
    let mut last_active_indices = BTreeMap::<K, u32>::new();
    let mut progress = ProgressTracker::new(stop_gap);

    for keychain in keychains {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let keychain_spks =
            request.iter_spks(keychain.clone()).map(|(spk_i, spk)| (spk_i, spk.into()));
        let keychain_result = fetch_txs_with_keychain_spks(
            client.clone(),
            start_time,
            &events,
            &chain_tip,
            latest_blocks.as_ref(),
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            keychain.clone(),
            keychain_spks,
            last_revealed_indices.get(&keychain).copied(),
            stop_gap,
            parallel_requests,
        )
        .await?;
        tx_update.extend(keychain_result.update);
        if let Some(last_active_index) = keychain_result.last_active_index {
            last_active_indices.insert(keychain, last_active_index);
        }
    }

    if cancel_token.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let chain_update = match (chain_tip, latest_blocks) {
        (Some(chain_tip), Some(latest_blocks)) => {
            Some(client.chain_update(&latest_blocks, &chain_tip, &tx_update.anchors).await?)
        }
        _ => None,
    };
    let response = FullScanResponse { chain_update, tx_update, last_active_indices };

    send_complete_async_unless_cancelled(
        &events,
        &cancel_token,
        clone_full_scan_response(&response),
    )
    .await?;

    Ok(response)
}

struct KeychainScanResult {
    update: TxUpdate<ConfirmationBlockTime>,
    last_active_index: Option<u32>,
}

#[allow(clippy::too_many_arguments)]
async fn fetch_txs_with_keychain_spks<K, I, C>(
    client: C,
    start_time: u64,
    events: &flume::Sender<ScanEvent<K>>,
    chain_tip: &Option<CheckPoint>,
    latest_blocks: Option<&BTreeMap<u32, BlockHash>>,
    cancel_token: &tokio_util::sync::CancellationToken,
    progress: &mut ProgressTracker<K>,
    inserted_txs: &mut HashSet<Txid>,
    keychain: K,
    mut keychain_spks: I,
    last_revealed_index: Option<u32>,
    stop_gap: usize,
    parallel_requests: usize,
) -> Result<KeychainScanResult>
where
    K: Ord + Clone + Send,
    I: Iterator<Item = Indexed<SpkWithExpectedTxids>> + Send,
    C: EsploraScanClient + Clone + Send + Sync,
{
    type TxsOfSpkIndex = (u32, Vec<esplora_client::Tx>, HashSet<Txid>);

    let mut update = TxUpdate::<ConfirmationBlockTime>::default();
    let mut last_active_index = Option::<u32>::None;
    let mut stop_gap_unused_count = 0usize;
    let gap_limit = stop_gap.max(1);

    loop {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let handles = keychain_spks
            .by_ref()
            .take(parallel_requests)
            .map(|(spk_index, spk)| {
                let client = client.clone();
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
                    let evicted_txids =
                        expected_txids.difference(&got_txids).copied().collect::<HashSet<_>>();
                    Result::<TxsOfSpkIndex, EsploraError>::Ok((spk_index, spk_txs, evicted_txids))
                }
            })
            .collect::<FuturesOrdered<_>>();

        if handles.is_empty() {
            break;
        }

        let mut partial_update = TxUpdate::<ConfirmationBlockTime>::default();
        for (index, txs, evicted) in handles.try_collect::<Vec<TxsOfSpkIndex>>().await? {
            let used = !txs.is_empty();
            let scan_progress = progress.checked(keychain.clone(), used);
            send_progress(events, scan_progress);

            if txs.is_empty() {
                if last_revealed_index.is_none_or(|last_revealed| index > last_revealed) {
                    stop_gap_unused_count = stop_gap_unused_count.saturating_add(1);
                }
            } else {
                stop_gap_unused_count = 0;
                last_active_index = Some(last_active_index.unwrap_or(index).max(index));
            }

            for tx in txs {
                if !inserted_txs.insert(tx.txid) {
                    continue;
                }

                partial_update.txs.push(tx.to_tx().into());
                insert_anchor_or_seen_at_from_status(
                    &mut partial_update,
                    start_time,
                    tx.txid,
                    tx.status,
                );
                insert_prevouts(&mut partial_update, tx.vin);
            }
            partial_update.evicted_ats.extend(evicted.into_iter().map(|txid| (txid, start_time)));
        }

        if !partial_update.is_empty() {
            let partial_chain_update = match (chain_tip, latest_blocks) {
                (Some(chain_tip), Some(latest_blocks)) => Some(
                    client.chain_update(latest_blocks, chain_tip, &partial_update.anchors).await?,
                ),
                _ => None,
            };
            let scan_update = ScanUpdate {
                chain_update: partial_chain_update,
                tx_update: partial_update.clone(),
                last_active_indices: last_active_index
                    .map(|index| BTreeMap::from([(keychain.clone(), index)]))
                    .unwrap_or_default(),
            };

            send_update_async(events, scan_update).await?;
            update.extend(partial_update);
        }

        if stop_gap_unused_count >= gap_limit {
            break;
        }
    }

    Ok(KeychainScanResult { update, last_active_index })
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
        let anchor =
            ConfirmationBlockTime { block_id: BlockId { height, hash }, confirmation_time: time };
        update.anchors.insert((anchor, txid));
    } else {
        update.seen_ats.insert((txid, start_time));
    }
}

fn insert_prevouts(
    update: &mut TxUpdate<ConfirmationBlockTime>,
    esplora_inputs: impl IntoIterator<Item = esplora_client::api::Vin>,
) {
    let prevouts =
        esplora_inputs.into_iter().filter_map(|vin| Some((vin.txid, vin.vout, vin.prevout?)));
    for (prev_txid, prev_vout, prev_txout) in prevouts {
        update.txouts.insert(
            OutPoint::new(prev_txid, prev_vout),
            TxOut {
                script_pubkey: prev_txout.scriptpubkey,
                value: Amount::from_sat(prev_txout.value),
            },
        );
    }
}

async fn fetch_latest_blocks<S>(
    client: &esplora_client::AsyncClient<S>,
) -> std::result::Result<BTreeMap<u32, BlockHash>, EsploraError>
where
    S: Sleeper,
{
    Ok(client
        .get_block_infos(None)
        .await?
        .into_iter()
        .map(|block| (block.height, block.id))
        .collect())
}

async fn fetch_block<S>(
    client: &esplora_client::AsyncClient<S>,
    latest_blocks: &BTreeMap<u32, BlockHash>,
    height: u32,
) -> std::result::Result<Option<BlockHash>, EsploraError>
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
) -> std::result::Result<CheckPoint, EsploraError>
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
    use std::{
        collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
        str::FromStr as _,
        sync::{Arc, Mutex},
    };

    use bdk_esplora::esplora_client;
    use bdk_wallet::bitcoin::Network;
    use bdk_wallet::{
        KeychainKind, Wallet,
        chain::{
            BlockId, CheckPoint, ConfirmationBlockTime, TxUpdate,
            bitcoin::{Amount, OutPoint, ScriptBuf, Txid},
            spk_client::SpkWithExpectedTxids,
        },
        test_utils::get_test_wpkh_and_change_desc,
    };
    use futures::future::BoxFuture;
    use tokio_util::sync::CancellationToken;

    use crate::{
        Error, ProgressTracker, ScanEvent,
        esplora::{
            EsploraError, EsploraScanClient, fetch_txs_with_keychain_spks,
            insert_anchor_or_seen_at_from_status, insert_prevouts, run_with_esplora_client,
        },
    };

    #[derive(Clone, Debug)]
    struct FakeEsplora {
        responses: Arc<Mutex<VecDeque<Option<Vec<esplora_client::Tx>>>>>,
        chain_update_requests: Arc<Mutex<usize>>,
    }

    impl FakeEsplora {
        fn with_responses(responses: impl IntoIterator<Item = Vec<esplora_client::Tx>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into_iter().map(Some).collect())),
                chain_update_requests: Arc::new(Mutex::new(0)),
            }
        }

        fn with_history_error() -> Self {
            Self {
                responses: Arc::new(Mutex::new(VecDeque::from([None]))),
                chain_update_requests: Arc::new(Mutex::new(0)),
            }
        }

        fn chain_update_count(&self) -> usize {
            *self.chain_update_requests.lock().expect("request count lock not poisoned")
        }
    }

    impl EsploraScanClient for FakeEsplora {
        fn scripthash_txs<'a>(
            &'a self,
            _: &'a bdk_wallet::chain::bitcoin::Script,
            _: Option<Txid>,
        ) -> BoxFuture<'a, std::result::Result<Vec<esplora_client::Tx>, EsploraError>> {
            Box::pin(async move {
                match self.responses.lock().expect("response lock not poisoned").pop_front() {
                    Some(Some(txs)) => Ok(txs),
                    Some(None) => Err(Box::new(esplora_client::Error::HttpResponse {
                        status: 500,
                        message: "history failed".to_string(),
                    })),
                    None => Ok(Vec::new()),
                }
            })
        }

        fn chain_update<'a>(
            &'a self,
            _: &'a BTreeMap<u32, bdk_wallet::chain::bitcoin::BlockHash>,
            local_tip: &'a bdk_wallet::chain::CheckPoint,
            _: &'a BTreeSet<(ConfirmationBlockTime, Txid)>,
        ) -> BoxFuture<'a, std::result::Result<bdk_wallet::chain::CheckPoint, EsploraError>>
        {
            let chain_update_requests = Arc::clone(&self.chain_update_requests);

            Box::pin(async move {
                *chain_update_requests.lock().expect("request count lock not poisoned") += 1;
                Ok(local_tip.clone())
            })
        }
    }

    #[test]
    fn empty_histories_emit_progress_without_updates_until_stop_gap() {
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = None;
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            None,
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ))
        .expect("scan succeeds");

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert!(result.update.is_empty());
        assert_eq!(result.last_active_index, None);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event, ScanEvent::Progress(_))));
    }

    #[test]
    fn mempool_history_emits_update_and_final_last_active_index() {
        let txid = txid(8);
        let fake = FakeEsplora::with_responses([vec![esplora_tx(txid)], Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = None;
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            None,
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ))
        .expect("scan succeeds");

        let events = receiver.try_iter().collect::<Vec<_>>();
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
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = None;
        let spks = [4, 1, 5, 6]
            .into_iter()
            .map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            None,
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            2,
        ))
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
        let mut tx = esplora_tx(txid);
        tx.status = esplora_client::TxStatus {
            confirmed: true,
            block_height: Some(2),
            block_hash: Some(block_hash(2)),
            block_time: Some(123),
        };
        let fake = FakeEsplora::with_responses([vec![tx], Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = Some(CheckPoint::new(BlockId { height: 1, hash: block_hash(1) }));
        let latest_blocks = BTreeMap::new();
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            Some(&latest_blocks),
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ))
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
        let make_tx = || {
            let mut tx = esplora_tx(txid);
            tx.status = esplora_client::TxStatus {
                confirmed: true,
                block_height: Some(2),
                block_hash: Some(block_hash(2)),
                block_time: Some(123),
            };
            tx
        };
        let fake =
            FakeEsplora::with_responses([vec![make_tx()], vec![make_tx()], Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = Some(CheckPoint::new(BlockId { height: 1, hash: block_hash(1) }));
        let latest_blocks = BTreeMap::new();
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake.clone(),
            7,
            &events,
            &chain_tip,
            Some(&latest_blocks),
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ))
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
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let fake = FakeEsplora::with_responses([vec![esplora_tx(txid(8))], Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        drop(receiver);
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            BTreeMap::new(),
            1,
        ));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn cancelled_scan_returns_cancelled_without_update() {
        let fake = FakeEsplora::with_responses([Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = None;
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            None,
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ));

        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn provider_error_does_not_emit_update() {
        let fake = FakeEsplora::with_history_error();
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        let mut progress = ProgressTracker::new(2);
        let mut inserted_txs = HashSet::new();
        let chain_tip = None;
        let spks = (0..5).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())));

        let result = futures::executor::block_on(fetch_txs_with_keychain_spks(
            fake,
            7,
            &events,
            &chain_tip,
            None,
            &cancel_token,
            &mut progress,
            &mut inserted_txs,
            "external",
            spks,
            None,
            2,
            1,
        ));

        assert!(matches!(result, Err(Error::Esplora(_))));
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn successful_empty_scan_sends_complete_once_without_update() {
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();

        let response = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            BTreeMap::new(),
            1,
        ))
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
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(KeychainKind::External, spks)
            .build();
        let fake = FakeEsplora::with_responses(std::iter::repeat_with(Vec::new).take(16));
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();

        let response = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            last_revealed_indices,
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
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        drop(receiver);
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            BTreeMap::new(),
            1,
        ));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn cancelled_scan_returns_cancelled_without_complete() {
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let fake = FakeEsplora::with_responses([Vec::new(), Vec::new(), Vec::new()]);
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let result = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            BTreeMap::new(),
            1,
        ));

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!events.iter().any(|event| matches!(event, ScanEvent::Complete(_))));
    }

    #[test]
    fn provider_error_does_not_emit_complete() {
        let request = bdk_wallet::chain::spk_client::FullScanRequest::builder_at(0)
            .spks_for_keychain(
                "external",
                (0..5).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
            )
            .build();
        let fake = FakeEsplora::with_history_error();
        let (events, receiver) = flume::unbounded();
        let cancel_token = CancellationToken::new();

        let result = futures::executor::block_on(run_with_esplora_client(
            request,
            2,
            events,
            cancel_token,
            fake,
            None,
            BTreeMap::new(),
            1,
        ));

        let events = receiver.try_iter().collect::<Vec<_>>();
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

    fn esplora_input(
        txid: Txid,
        vout: u32,
        prevout: Option<(u64, ScriptBuf)>,
    ) -> esplora_client::api::Vin {
        esplora_client::api::Vin {
            txid,
            vout,
            prevout: prevout
                .map(|(value, scriptpubkey)| esplora_client::api::PrevOut { value, scriptpubkey }),
            scriptsig: ScriptBuf::new(),
            witness: Vec::new(),
            sequence: 0,
            is_coinbase: false,
        }
    }

    fn txid(byte: u8) -> Txid {
        Txid::from_str(&format!("{byte:02x}{}", "00".repeat(31))).expect("valid txid")
    }

    fn block_hash(byte: u8) -> bdk_wallet::chain::bitcoin::BlockHash {
        bdk_wallet::chain::bitcoin::BlockHash::from_str(&format!("{byte:02x}{}", "00".repeat(31)))
            .expect("valid block hash")
    }

    fn esplora_tx(txid: Txid) -> esplora_client::Tx {
        esplora_client::Tx {
            txid,
            version: 2,
            locktime: 0,
            vin: Vec::new(),
            vout: Vec::new(),
            size: 0,
            weight: 0,
            status: esplora_client::TxStatus {
                confirmed: false,
                block_height: None,
                block_hash: None,
                block_time: None,
            },
            fee: 0,
        }
    }
}

use std::collections::BTreeMap;
use std::str::FromStr as _;

use bdk_electrum::{BdkElectrumClient, electrum_client::Client};
use bdk_esplora::{EsploraAsyncExt as _, esplora_client};
use bdk_wallet::KeychainKind;
use bdk_wallet::bitcoin::{Address, ScriptBuf};
use bdk_wallet::chain::spk_client::{FullScanRequest, FullScanResponse};
use cove_bdk_progressive_scan::{ProgressiveScanner, ScanEvent};
use tokio_util::sync::CancellationToken;

const USED_ADDRESS: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
const UNUSED_ADDRESS: &str = "bc1q0g0vn4yqyk0zjwxw0zv5pltyyczty004zc9g7r";

#[tokio::test]
#[ignore] // requires external network connection to blockstream's Esplora server
async fn live_esplora_matches_bdk_full_scan_for_used_address() {
    let baseline_client = esplora_client::Builder::new("https://blockstream.info/api")
        .build_async()
        .expect("client builds");
    let progressive_client = esplora_client::Builder::new("https://blockstream.info/api")
        .build_async()
        .expect("client builds");

    let baseline =
        baseline_client.full_scan(scan_request(), 1, 1).await.expect("bdk full scan succeeds");
    let progressive = run_progressive_esplora(progressive_client, scan_request()).await;

    assert_matching_wallet_facts(&baseline, &progressive.response);
    assert_update_before_complete(&progressive.events);
}

#[tokio::test]
#[ignore] // requires external network connection to blockstream's Esplora server
async fn live_esplora_matches_bdk_full_scan_for_unused_address() {
    let baseline_client = esplora_client::Builder::new("https://blockstream.info/api")
        .build_async()
        .expect("client builds");
    let progressive_client = esplora_client::Builder::new("https://blockstream.info/api")
        .build_async()
        .expect("client builds");

    let baseline = baseline_client
        .full_scan(unused_scan_request(), 1, 1)
        .await
        .expect("bdk full scan succeeds");
    let progressive = run_progressive_esplora(progressive_client, unused_scan_request()).await;

    assert_matching_wallet_facts(&baseline, &progressive.response);
    assert_empty_scan_completes_without_update(&progressive.events);
}

#[test]
#[ignore] // requires external network connection to blockstream's Electrum server
fn live_electrum_matches_bdk_full_scan_for_used_address() {
    let baseline_client = BdkElectrumClient::new(
        Client::new("ssl://electrum.blockstream.info:50002").expect("client connects"),
    );
    let progressive_client = BdkElectrumClient::new(
        Client::new("ssl://electrum.blockstream.info:50002").expect("client connects"),
    );

    let baseline =
        baseline_client.full_scan(scan_request(), 1, 1, false).expect("bdk full scan succeeds");
    let progressive = run_progressive_electrum(progressive_client, scan_request());

    assert_matching_wallet_facts(&baseline, &progressive.response);
    assert_update_before_complete(&progressive.events);
}

#[test]
#[ignore] // requires external network connection to blockstream's Electrum server
fn live_electrum_matches_bdk_full_scan_for_unused_address() {
    let baseline_client = BdkElectrumClient::new(
        Client::new("ssl://electrum.blockstream.info:50002").expect("client connects"),
    );
    let progressive_client = BdkElectrumClient::new(
        Client::new("ssl://electrum.blockstream.info:50002").expect("client connects"),
    );

    let baseline = baseline_client
        .full_scan(unused_scan_request(), 1, 1, false)
        .expect("bdk full scan succeeds");
    let progressive = run_progressive_electrum(progressive_client, unused_scan_request());

    assert_matching_wallet_facts(&baseline, &progressive.response);
    assert_empty_scan_completes_without_update(&progressive.events);
}

struct ProgressiveRun {
    response: FullScanResponse<KeychainKind>,
    events: Vec<ScanEvent<KeychainKind>>,
}

async fn run_progressive_esplora(
    client: esplora_client::r#async::AsyncClient,
    request: FullScanRequest<KeychainKind>,
) -> ProgressiveRun {
    let (events, receiver) = flume::unbounded();
    let response = ProgressiveScanner::builder()
        .request(request)
        .last_revealed_indices(BTreeMap::from([(KeychainKind::External, 0)]))
        .stop_gap(1)
        .events(events)
        .cancel_token(CancellationToken::new())
        .esplora(client)
        .expect("scanner builds")
        .parallel_requests(1)
        .run()
        .await
        .expect("progressive scan succeeds");

    ProgressiveRun { response, events: receiver.try_iter().collect() }
}

fn run_progressive_electrum(
    client: BdkElectrumClient<Client>,
    request: FullScanRequest<KeychainKind>,
) -> ProgressiveRun {
    let (events, receiver) = flume::unbounded();
    let response = ProgressiveScanner::builder()
        .request(request)
        .last_revealed_indices(BTreeMap::from([(KeychainKind::External, 0)]))
        .stop_gap(1)
        .events(events)
        .cancel_token(CancellationToken::new())
        .electrum(client)
        .expect("scanner builds")
        .batch_size(1)
        .fetch_prev_txouts(false)
        .run()
        .expect("progressive scan succeeds");

    ProgressiveRun { response, events: receiver.try_iter().collect() }
}

fn scan_request() -> FullScanRequest<KeychainKind> {
    FullScanRequest::builder_at(0)
        .spks_for_keychain(
            KeychainKind::External,
            [(0, address_script(USED_ADDRESS)), (1, address_script(UNUSED_ADDRESS))],
        )
        .build()
}

fn unused_scan_request() -> FullScanRequest<KeychainKind> {
    FullScanRequest::builder_at(0)
        .spks_for_keychain(KeychainKind::External, [(0, address_script(UNUSED_ADDRESS))])
        .build()
}

fn address_script(address: &str) -> ScriptBuf {
    Address::from_str(address).expect("address parses").assume_checked().script_pubkey()
}

fn assert_matching_wallet_facts(
    expected: &FullScanResponse<KeychainKind>,
    actual: &FullScanResponse<KeychainKind>,
) {
    assert_eq!(actual.last_active_indices, expected.last_active_indices);
    assert_eq!(txids(actual), txids(expected));
    assert_eq!(anchor_txids(actual), anchor_txids(expected));
    assert_eq!(actual.tx_update.seen_ats, expected.tx_update.seen_ats);
    assert_eq!(actual.tx_update.evicted_ats, expected.tx_update.evicted_ats);
    assert_eq!(
        actual.tx_update.txouts.keys().collect::<Vec<_>>(),
        expected.tx_update.txouts.keys().collect::<Vec<_>>()
    );
}

fn txids(response: &FullScanResponse<KeychainKind>) -> Vec<String> {
    let mut txids =
        response.tx_update.txs.iter().map(|tx| tx.compute_txid().to_string()).collect::<Vec<_>>();
    txids.sort();
    txids
}

fn anchor_txids(response: &FullScanResponse<KeychainKind>) -> Vec<String> {
    let mut txids =
        response.tx_update.anchors.iter().map(|(_, txid)| txid.to_string()).collect::<Vec<_>>();
    txids.sort();
    txids
}

fn assert_update_before_complete(events: &[ScanEvent<KeychainKind>]) {
    let update_index = events
        .iter()
        .position(|event| matches!(event, ScanEvent::Update(update) if !update.is_empty()));
    let complete_index = events.iter().position(|event| matches!(event, ScanEvent::Complete(_)));

    assert!(
        matches!((update_index, complete_index), (Some(update), Some(complete)) if update < complete)
    );
}

fn assert_empty_scan_completes_without_update(events: &[ScanEvent<KeychainKind>]) {
    assert_eq!(events.iter().filter(|event| matches!(event, ScanEvent::Complete(_))).count(), 1);
    assert!(!events.iter().any(|event| matches!(event, ScanEvent::Update(_))));
}

use std::{
    collections::{BTreeMap, VecDeque},
    str::FromStr as _,
    sync::Arc,
};

use bdk_esplora::esplora_client;
use bdk_wallet::{
    KeychainKind, Wallet,
    bitcoin::Network,
    chain::{
        bitcoin::{BlockHash, ScriptBuf, Transaction, Txid, absolute, transaction},
        spk_client::{FullScanRequest, SpkWithExpectedTxids},
    },
    test_utils::get_test_wpkh_and_change_desc,
};
use parking_lot::Mutex;

use crate::ScanEvent;

#[derive(Debug, Clone)]
pub(crate) struct ResponseQueue<T>(Arc<Mutex<VecDeque<Option<Vec<T>>>>>);

impl<T> ResponseQueue<T> {
    pub(crate) fn empty() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::new())))
    }

    pub(crate) fn with_responses(responses: impl IntoIterator<Item = Vec<T>>) -> Self {
        Self(Arc::new(Mutex::new(responses.into_iter().map(Some).collect())))
    }

    pub(crate) fn with_error() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::from([None]))))
    }

    pub(crate) fn pop(&self) -> QueuedResponse<T> {
        match self.0.lock().pop_front() {
            Some(Some(response)) => QueuedResponse::Response(response),
            Some(None) => QueuedResponse::Error,
            None => QueuedResponse::Exhausted,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum QueuedResponse<T> {
    Response(Vec<T>),
    Error,
    Exhausted,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SharedCounter(Arc<Mutex<usize>>);

impl SharedCounter {
    pub(crate) fn increment(&self) {
        *self.0.lock() += 1;
    }

    pub(crate) fn get(&self) -> usize {
        *self.0.lock()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SharedVec<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for SharedVec<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T> SharedVec<T> {
    pub(crate) fn push(&self, value: T) {
        self.0.lock().push(value);
    }
}

impl<T> SharedVec<T>
where
    T: Clone,
{
    pub(crate) fn snapshot(&self) -> Vec<T> {
        self.0.lock().clone()
    }
}

pub(crate) fn event_channel<K>() -> (flume::Sender<ScanEvent<K>>, flume::Receiver<ScanEvent<K>>) {
    flume::unbounded()
}

pub(crate) fn collect_events<K>(receiver: flume::Receiver<ScanEvent<K>>) -> Vec<ScanEvent<K>> {
    receiver.try_iter().collect()
}

pub(crate) fn empty_spks(count: u32) -> impl Iterator<Item = (u32, SpkWithExpectedTxids)> {
    (0..count).map(|index| (index, SpkWithExpectedTxids::from(ScriptBuf::new())))
}

pub(crate) fn external_request(start_time: u64, count: u32) -> FullScanRequest<&'static str> {
    FullScanRequest::builder_at(start_time)
        .spks_for_keychain(
            "external",
            (0..count).map(|index| (index, ScriptBuf::new())).collect::<Vec<_>>(),
        )
        .build()
}

pub(crate) fn revealed_external_request(
    last_revealed: u32,
) -> (FullScanRequest<KeychainKind>, BTreeMap<KeychainKind, u32>) {
    let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();
    let mut wallet = Wallet::create(external_descriptor, internal_descriptor)
        .network(Network::Signet)
        .create_wallet_no_persist()
        .expect("wallet is created");
    let _ = wallet.reveal_addresses_to(KeychainKind::External, last_revealed).last();
    let last_revealed_indices = wallet.spk_index().last_revealed_indices();
    let spks = wallet
        .spk_index()
        .unbounded_spk_iter(KeychainKind::External)
        .expect("external keychain exists");
    let request =
        FullScanRequest::builder_at(0).spks_for_keychain(KeychainKind::External, spks).build();

    (request, last_revealed_indices)
}

pub(crate) fn test_transaction() -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: Vec::new(),
        output: Vec::new(),
    }
}

pub(crate) fn txid(byte: u8) -> Txid {
    Txid::from_str(&format!("{byte:02x}{}", "00".repeat(31))).expect("valid txid")
}

pub(crate) fn block_hash(byte: u8) -> BlockHash {
    BlockHash::from_str(&format!("{byte:02x}{}", "00".repeat(31))).expect("valid block hash")
}

pub(crate) fn esplora_tx(txid: Txid) -> esplora_client::Tx {
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

pub(crate) fn confirmed_esplora_tx(
    txid: Txid,
    height: u32,
    hash: BlockHash,
    time: u64,
) -> esplora_client::Tx {
    let mut tx = esplora_tx(txid);
    tx.status = esplora_client::TxStatus {
        confirmed: true,
        block_height: Some(height),
        block_hash: Some(hash),
        block_time: Some(time),
    };
    tx
}

pub(crate) fn esplora_input(
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

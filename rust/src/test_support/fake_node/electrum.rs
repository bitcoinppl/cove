use std::{str::FromStr as _, sync::Arc};

use bitcoin::{
    Amount, Transaction, Txid,
    consensus::{deserialize, encode::serialize},
    hex::DisplayHex as _,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use super::{
    FakeChain, Shared,
    chain::{AlreadyConfirmed, HistoryEntry, TxLocation},
    control::{Protocol, Verdict},
};

/// The protocol version the fake negotiates, which selects the legacy
/// `blockchain.block.headers` response shape in electrum-client
const PROTOCOL_VERSION: &str = "1.4";
const MAX_HEADERS_PER_REQUEST: usize = 2016;

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const NOT_FOUND: i64 = 2;
/// What electrs answers when bitcoind rejects a broadcast
const DAEMON_ERROR: i64 = 2;

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn invalid_params() -> Self {
        Self::new(INVALID_PARAMS, "invalid params")
    }
}

/// Accepts connections until `shutdown` flips, then drops the listener
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                tokio::spawn(serve_connection(stream, shared.clone()));
            }

            _ = shutdown.wait_for(|stop| *stop) => return,
        }
    }
}

/// Answers newline-delimited JSON-RPC requests one at a time
///
/// electrum-client sends a batch as several request lines rather than a JSON
/// array, and matches responses to requests by id
async fn serve_connection(stream: TcpStream, shared: Arc<Shared>) {
    let mut reader = BufReader::new(stream);

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default().to_string();
        let params = request["params"].as_array().cloned().unwrap_or_default();

        let verdict = shared
            .control
            .intercept(Protocol::Electrum, &method, &Value::Array(params.clone()).to_string())
            .await;
        let outcome = match verdict {
            Verdict::Fail { status, message } => Err(RpcError::new(i64::from(status), message)),
            Verdict::Serve => handle(&shared.chain, &method, &params),
        };

        let reply = match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": error.code, "message": error.message },
            }),
        };

        let mut bytes = reply.to_string().into_bytes();
        bytes.push(b'\n');
        if reader.get_mut().write_all(&bytes).await.is_err() {
            return;
        }
    }
}

fn handle(chain: &FakeChain, method: &str, params: &[Value]) -> Result<Value, RpcError> {
    match method {
        "server.version" => Ok(json!(["fake-electrum 0.1", PROTOCOL_VERSION])),
        "server.banner" => Ok(json!("fake electrum server")),
        "server.donation_address" => Ok(Value::Null),
        "server.ping" => Ok(Value::Null),
        "server.features" => Ok(json!({
            "server_version": "fake-electrum 0.1",
            "genesis_hash": chain.header(0).map(|header| header.block_hash().to_string()),
            "protocol_min": PROTOCOL_VERSION,
            "protocol_max": PROTOCOL_VERSION,
            "hash_function": "sha256",
            "pruning": null,
        })),

        "blockchain.relayfee" => Ok(json!(0.00001)),
        "blockchain.estimatefee" => Ok(json!(0.00002)),
        "mempool.get_fee_histogram" => Ok(json!([])),

        "blockchain.headers.subscribe" => {
            let height = chain.tip_height();
            let header = chain.header(height).expect("tip exists");

            Ok(json!({ "height": height, "hex": serialize(&header).to_lower_hex_string() }))
        }

        "blockchain.block.header" => {
            let height = usize_param(params, 0)?;
            let header = chain
                .header(height as u32)
                .ok_or_else(|| RpcError::new(1, format!("height {height} out of range")))?;

            Ok(json!(serialize(&header).to_lower_hex_string()))
        }

        "blockchain.block.headers" => {
            let start = usize_param(params, 0)?;
            let count = usize_param(params, 1)?.min(MAX_HEADERS_PER_REQUEST);
            let raw = (start..start + count)
                .map_while(|height| chain.header(height as u32))
                .flat_map(|header| serialize(&header))
                .collect::<Vec<u8>>();

            Ok(json!({
                "count": raw.len() / 80,
                "hex": raw.to_lower_hex_string(),
                "max": MAX_HEADERS_PER_REQUEST,
            }))
        }

        "blockchain.scripthash.get_history" => {
            let history = history_for(chain, params)?;

            Ok(Value::Array(history.iter().map(history_json).collect()))
        }

        "blockchain.scripthash.subscribe" => {
            let history = history_for(chain, params)?;

            // the status is a hash over the history, null when there is none
            Ok(status_hash(&history).map_or(Value::Null, Value::String))
        }

        "blockchain.scripthash.get_balance" => {
            let Some(script) = script_param(chain, params)? else {
                return Ok(json!({ "confirmed": 0, "unconfirmed": 0 }));
            };

            // a pending spend of confirmed funds makes `unconfirmed` negative, as in electrs
            let (mut confirmed, mut unconfirmed) = (0i64, 0i64);
            for entry in chain.history(&script) {
                let tx = chain.tx(entry.txid).expect("history entries exist").tx;
                let received = tx
                    .output
                    .iter()
                    .filter(|output| output.script_pubkey == script)
                    .map(|output| output.value)
                    .sum::<Amount>();
                let spent = chain
                    .spent_from(&tx, &script)
                    .iter()
                    .map(|output| output.value)
                    .sum::<Amount>();
                let change = received.to_signed().expect("amount fits").to_sat()
                    - spent.to_signed().expect("amount fits").to_sat();

                match entry.location {
                    TxLocation::Mempool => unconfirmed += change,
                    TxLocation::Confirmed { .. } => confirmed += change,
                }
            }

            Ok(json!({ "confirmed": confirmed, "unconfirmed": unconfirmed }))
        }

        "blockchain.transaction.get" => {
            let txid = txid_param(params, 0)?;
            let verbose = params.get(1).and_then(Value::as_bool).unwrap_or(false);
            let entry = chain.tx(txid).ok_or_else(|| {
                RpcError::new(
                    NOT_FOUND,
                    "No such mempool or blockchain transaction. Use gettransaction for wallet transactions.",
                )
            })?;
            let raw = serialize(&entry.tx).to_lower_hex_string();

            if !verbose {
                return Ok(json!(raw));
            }

            // electrs forwards bitcoind's `getrawtransaction`, which leaves out
            // `confirmations` for a transaction that is not in a block
            let mut reply = json!({ "txid": txid, "hex": raw });
            if let TxLocation::Confirmed { height } = entry.location {
                reply["confirmations"] = json!(chain.tip_height() - height + 1);
            }

            Ok(reply)
        }

        "blockchain.transaction.get_merkle" => {
            let txid = txid_param(params, 0)?;
            let proof = chain
                .merkle_proof(txid)
                .ok_or_else(|| RpcError::new(NOT_FOUND, "transaction is not confirmed"))?;

            // electrum-client reverses each hash again when it validates the proof
            let merkle = proof
                .branch
                .iter()
                .map(|hash| {
                    let mut display = *hash;
                    display.reverse();

                    display.to_lower_hex_string()
                })
                .collect::<Vec<_>>();

            Ok(json!({ "block_height": proof.height, "pos": proof.position, "merkle": merkle }))
        }

        "blockchain.transaction.broadcast" => {
            let hex =
                params.first().and_then(Value::as_str).ok_or_else(RpcError::invalid_params)?;
            let raw = hex::decode(hex).map_err(|_| RpcError::invalid_params())?;
            let tx = deserialize::<Transaction>(&raw).map_err(|_| RpcError::invalid_params())?;

            let txid = chain.broadcast(tx).map_err(|AlreadyConfirmed| {
                RpcError::new(DAEMON_ERROR, "Transaction already in block chain")
            })?;

            Ok(json!(txid))
        }

        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("unknown method {other}"))),
    }
}

fn usize_param(params: &[Value], index: usize) -> Result<usize, RpcError> {
    let value = params.get(index).and_then(Value::as_u64).ok_or_else(RpcError::invalid_params)?;

    usize::try_from(value).map_err(|_| RpcError::invalid_params())
}

fn txid_param(params: &[Value], index: usize) -> Result<Txid, RpcError> {
    let text = params.get(index).and_then(Value::as_str).ok_or_else(RpcError::invalid_params)?;

    Txid::from_str(text).map_err(|_| RpcError::invalid_params())
}

/// The script behind the scripthash param, `None` when the chain has never seen it
///
/// The Electrum protocol sends the SHA-256 digest of the script reversed:
/// https://electrum-protocol.readthedocs.io/en/latest/protocol-basics.html#script-hashes
fn script_param(
    chain: &FakeChain,
    params: &[Value],
) -> Result<Option<bitcoin::ScriptBuf>, RpcError> {
    let text = params.first().and_then(Value::as_str).ok_or_else(RpcError::invalid_params)?;
    let mut digest = hex::decode(text)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(RpcError::invalid_params)?;
    digest.reverse();

    Ok(chain.script_for_digest(&digest))
}

fn history_for(chain: &FakeChain, params: &[Value]) -> Result<Vec<HistoryEntry>, RpcError> {
    let script = script_param(chain, params)?;

    Ok(script.map(|script| chain.history(&script)).unwrap_or_default())
}

fn history_json(entry: &HistoryEntry) -> Value {
    let height = match entry.location {
        TxLocation::Mempool => 0,
        TxLocation::Confirmed { height } => i64::from(height),
    };

    json!({ "height": height, "tx_hash": entry.txid })
}

fn status_hash(history: &[HistoryEntry]) -> Option<String> {
    use bitcoin::hashes::{Hash as _, sha256};

    if history.is_empty() {
        return None;
    }

    let preimage = history
        .iter()
        .map(|entry| {
            let height = match entry.location {
                TxLocation::Mempool => 0,
                TxLocation::Confirmed { height } => height,
            };

            format!("{}:{height}:", entry.txid)
        })
        .collect::<String>();

    Some(sha256::Hash::hash(preimage.as_bytes()).to_string())
}

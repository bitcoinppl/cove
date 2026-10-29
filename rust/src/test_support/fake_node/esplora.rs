use std::{str::FromStr as _, sync::Arc};

use bitcoin::{
    Address, Amount, Transaction, Txid,
    consensus::{deserialize, encode::serialize},
    hashes::Hash as _,
    hex::DisplayHex,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use super::{
    FakeChain, Shared,
    chain::{AlreadyConfirmed, ChainTx, HistoryEntry, TxLocation},
    control::{Protocol, Verdict},
};

/// Esplora pages confirmed history in chunks of this size
const CONFIRMED_PAGE_SIZE: usize = 25;
const BLOCKS_PAGE_SIZE: u32 = 10;

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: serde_json::to_vec(value).expect("json serializes"),
        }
    }

    fn text(text: impl Into<String>) -> Self {
        Self { status: 200, content_type: "text/plain", body: text.into().into_bytes() }
    }

    fn bytes(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "application/octet-stream", body }
    }

    fn error(status: u16, message: &str) -> Self {
        Self { status, content_type: "text/plain", body: message.as_bytes().to_vec() }
    }

    fn not_found(message: &str) -> Self {
        Self::error(404, message)
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

async fn serve_connection(stream: TcpStream, shared: Arc<Shared>) {
    let mut reader = BufReader::new(stream);

    // keep-alive: reqwest reuses the connection for many requests
    while let Some((method, path, body)) = read_request(&mut reader).await {
        let verdict = shared.control.intercept(Protocol::Esplora, &path, "").await;
        let response = match verdict {
            Verdict::Fail { status, message } => Response::error(status, &message),
            Verdict::Serve => route(&shared.chain, &method, &path, &body),
        };

        if write_response(reader.get_mut(), &response).await.is_err() {
            return;
        }
    }
}

async fn read_request(reader: &mut BufReader<TcpStream>) -> Option<(String, String, Vec<u8>)> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.ok()? == 0 {
        return None;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.ok()?;

        let header = header.trim();
        if header.is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().ok()?;
        }
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.ok()?;

    Some((method, path, body))
}

async fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
    );

    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.flush().await
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    }
}

fn route(chain: &FakeChain, method: &str, path: &str, body: &[u8]) -> Response {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();

    match (method, segments.as_slice()) {
        ("GET", ["blocks", "tip", "height"]) => Response::text(chain.tip_height().to_string()),
        ("GET", ["blocks", "tip", "hash"]) => Response::text(chain.tip_hash().to_string()),
        ("GET", ["blocks"]) => blocks(chain, None),
        ("GET", ["blocks", height]) => match height.parse() {
            Ok(height) => blocks(chain, Some(height)),
            Err(_) => Response::error(400, "Invalid block height"),
        },

        ("GET", ["block-height", height]) => {
            match height.parse().ok().and_then(|height| chain.header(height)) {
                Some(header) => Response::text(header.block_hash().to_string()),
                None => Response::not_found("Block not found"),
            }
        }

        ("GET", ["block", hash, "header"]) => match block_height(chain, hash) {
            Some(height) => {
                let header = chain.header(height).expect("height came from the chain");

                Response::text(serialize(&header).to_lower_hex_string())
            }
            None => Response::not_found("Block not found"),
        },

        ("GET", ["block", hash, "status"]) => match block_height(chain, hash) {
            Some(height) => {
                let next_best = chain.header(height + 1).map(|header| header.block_hash());

                Response::json(&json!({
                    "in_best_chain": true,
                    "height": height,
                    "next_best": next_best,
                }))
            }
            None => Response::not_found("Block not found"),
        },

        ("GET", ["tx", txid]) => {
            with_tx(chain, txid, |entry| Response::json(&tx_json(chain, entry)))
        }
        ("GET", ["tx", txid, "raw"]) => {
            with_tx(chain, txid, |entry| Response::bytes(serialize(&entry.tx)))
        }
        ("GET", ["tx", txid, "status"]) => {
            with_tx(chain, txid, |entry| Response::json(&status_json(chain, entry.location)))
        }
        ("GET", ["tx", txid, "merkle-proof"]) => with_proof(chain, txid),

        ("POST", ["tx"]) => broadcast(chain, body),

        ("GET", ["scripthash", hash, "txs"]) => scripthash_txs(chain, hash, Page::First),
        ("GET", ["scripthash", hash, "txs", "chain", last_seen]) => {
            scripthash_txs(chain, hash, Page::After(last_seen))
        }
        ("GET", ["scripthash", hash, "txs", "mempool"]) => {
            scripthash_txs(chain, hash, Page::Mempool)
        }

        ("GET", ["address", address]) => address_stats(chain, address),
        ("GET", ["fee-estimates"]) => Response::json(&json!({ "1": 2.0, "3": 2.0, "6": 1.0 })),

        _ => Response::not_found("Not found"),
    }
}

fn blocks(chain: &FakeChain, start: Option<u32>) -> Response {
    let infos = chain
        .headers_descending(start, BLOCKS_PAGE_SIZE as usize)
        .into_iter()
        .map(|(height, header)| {
            let tx_count = chain.block_txids(height).map_or(0, |txids| txids.len());
            let previous = (height > 0).then_some(header.prev_blockhash);

            json!({
                "id": header.block_hash(),
                "height": height,
                "version": header.version.to_consensus(),
                "timestamp": header.time,
                "tx_count": tx_count,
                "size": 285,
                "weight": 1140,
                "merkle_root": header.merkle_root,
                "previousblockhash": previous,
                "mediantime": header.time,
                "nonce": header.nonce,
                "bits": header.bits.to_consensus(),
                "difficulty": 1.0,
            })
        })
        .collect::<Vec<_>>();

    Response::json(&Value::Array(infos))
}

fn block_height(chain: &FakeChain, hash: &str) -> Option<u32> {
    chain.height_of_block(hash.parse().ok()?)
}

fn with_tx(chain: &FakeChain, txid: &str, respond: impl FnOnce(&ChainTx) -> Response) -> Response {
    let Ok(txid) = Txid::from_str(txid) else { return Response::error(400, "Invalid hex string") };

    match chain.tx(txid) {
        Some(entry) => respond(&entry),
        None => Response::not_found("Transaction not found"),
    }
}

fn with_proof(chain: &FakeChain, txid: &str) -> Response {
    let Ok(txid) = Txid::from_str(txid) else { return Response::error(400, "Invalid hex string") };

    match chain.merkle_proof(txid) {
        Some(proof) => Response::json(&json!({
            "block_height": proof.height,
            "merkle": proof.branch.into_iter().map(Txid::from_byte_array).collect::<Vec<_>>(),
            "pos": proof.position,
        })),
        None => Response::not_found("Transaction not found or is unconfirmed"),
    }
}

fn status_json(chain: &FakeChain, location: TxLocation) -> Value {
    match location {
        TxLocation::Mempool => json!({ "confirmed": false }),
        TxLocation::Confirmed { height } => {
            let header = chain.header(height).expect("confirmed height exists");

            json!({
                "confirmed": true,
                "block_height": height,
                "block_hash": header.block_hash(),
                "block_time": header.time,
            })
        }
    }
}

fn tx_json(chain: &FakeChain, entry: &ChainTx) -> Value {
    let tx = &entry.tx;
    let vin = tx.input.iter().map(|input| {
        let prevout = chain.prevout(&input.previous_output).map(|output| {
            json!({ "value": output.value.to_sat(), "scriptpubkey": output.script_pubkey.to_hex_string() })
        });

        json!({
            "txid": input.previous_output.txid,
            "vout": input.previous_output.vout,
            "prevout": prevout,
            "scriptsig": input.script_sig.to_hex_string(),
            "witness": input.witness.iter().map(DisplayHex::to_lower_hex_string).collect::<Vec<_>>(),
            "sequence": input.sequence.0,
            "is_coinbase": input.previous_output.is_null(),
        })
    });
    let vout = tx.output.iter().map(|output| {
        json!({ "value": output.value.to_sat(), "scriptpubkey": output.script_pubkey.to_hex_string() })
    });

    json!({
        "txid": tx.compute_txid(),
        "version": tx.version.0,
        "locktime": tx.lock_time.to_consensus_u32(),
        "vin": vin.collect::<Vec<_>>(),
        "vout": vout.collect::<Vec<_>>(),
        "size": tx.total_size(),
        "weight": tx.weight().to_wu(),
        "status": status_json(chain, entry.location),
        "fee": chain.fee(tx).map_or(0, Amount::to_sat),
    })
}

fn broadcast(chain: &FakeChain, body: &[u8]) -> Response {
    let Ok(text) = std::str::from_utf8(body) else { return Response::error(400, "Invalid body") };
    let Ok(raw) = hex::decode(text.trim()) else { return Response::error(400, "Invalid hex") };
    let Ok(tx) = deserialize::<Transaction>(&raw) else {
        return Response::error(400, "Invalid transaction");
    };

    match chain.broadcast(tx) {
        Ok(txid) => Response::text(txid.to_string()),
        Err(AlreadyConfirmed) => Response::error(400, "Transaction already in block chain"),
    }
}

enum Page<'a> {
    First,
    After(&'a str),
    Mempool,
}

fn scripthash_txs(chain: &FakeChain, hash: &str, page: Page<'_>) -> Response {
    let Some(script) = parse_hash(hash).and_then(|hash| chain.script_for_hash(&hash)) else {
        return Response::json(&json!([]));
    };

    let history = chain.history(&script);
    let mempool = history.iter().filter(|entry| entry.location == TxLocation::Mempool);
    let mut confirmed = history
        .iter()
        .filter(|entry| matches!(entry.location, TxLocation::Confirmed { .. }))
        .copied()
        .collect::<Vec<HistoryEntry>>();

    // newest first, matching what Esplora returns
    confirmed.reverse();
    confirmed.sort_by_key(|entry| match entry.location {
        TxLocation::Confirmed { height } => std::cmp::Reverse(height),
        TxLocation::Mempool => std::cmp::Reverse(0),
    });

    let entries = match page {
        Page::Mempool => mempool.copied().collect::<Vec<_>>(),
        Page::First => {
            mempool.copied().chain(confirmed.iter().take(CONFIRMED_PAGE_SIZE).copied()).collect()
        }
        Page::After(last_seen) => {
            let Ok(last_seen) = Txid::from_str(last_seen) else {
                return Response::error(400, "Invalid hex string");
            };
            let start = confirmed
                .iter()
                .position(|entry| entry.txid == last_seen)
                .map_or(confirmed.len(), |index| index + 1);

            confirmed.iter().skip(start).take(CONFIRMED_PAGE_SIZE).copied().collect()
        }
    };

    let txs = entries
        .iter()
        .filter_map(|entry| chain.tx(entry.txid))
        .map(|entry| tx_json(chain, &entry))
        .collect::<Vec<_>>();

    Response::json(&Value::Array(txs))
}

fn address_stats(chain: &FakeChain, address: &str) -> Response {
    let Ok(address) = Address::from_str(address) else {
        return Response::error(400, "Invalid address");
    };
    let address = address.assume_checked();
    let script = address.script_pubkey();

    let summarize = |confirmed: bool| {
        let entries = chain
            .history(&script)
            .into_iter()
            .filter(|entry| (entry.location != TxLocation::Mempool) == confirmed)
            .collect::<Vec<_>>();
        let txs = entries.iter().filter_map(|entry| chain.tx(entry.txid)).collect::<Vec<_>>();
        let funded = txs
            .iter()
            .flat_map(|entry| &entry.tx.output)
            .filter(|output| output.script_pubkey == script)
            .collect::<Vec<_>>();

        // esplora counts a spend under the status of the spending transaction
        let spent =
            txs.iter().flat_map(|entry| chain.spent_from(&entry.tx, &script)).collect::<Vec<_>>();

        json!({
            "funded_txo_count": funded.len(),
            "funded_txo_sum": funded.iter().map(|output| output.value.to_sat()).sum::<u64>(),
            "spent_txo_count": spent.len(),
            "spent_txo_sum": spent.iter().map(|output| output.value.to_sat()).sum::<u64>(),
            "tx_count": entries.len(),
        })
    };

    Response::json(&json!({
        "address": address.to_string(),
        "chain_stats": summarize(true),
        "mempool_stats": summarize(false),
    }))
}

fn parse_hash(hash: &str) -> Option<[u8; 32]> {
    hex::decode(hash).ok()?.try_into().ok()
}

//! Runs the `cove-fake-node` binary and drives it over real sockets, the way
//! the iOS and Android UI tests do

use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    net::TcpStream,
    process::{Child, Command, ExitStatus, Stdio},
    str::FromStr as _,
};

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    absolute::LockTime, consensus::encode::serialize_hex, transaction::Version,
};
use cove_fake_node::RESET_BLOCKS;
use serde_json::{Value, json};

const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// m/84'/1'/0'/0/0 of [`MNEMONIC`], the first receive address Cove shows on Signet
const FIRST_RECEIVE: &str = "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl";
const EXTERNAL: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

/// The binary on free ports, killed on drop if the test fails before stopping it
struct RunningNode {
    child: Child,
    control: String,
    esplora: String,
    electrum: String,
}

impl RunningNode {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cove-fake-node"))
            .args(["--control-port", "0", "--esplora-port", "0", "--electrum-port", "0"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("the fake node starts");

        // the binary prints its ready line only once every port listens
        let mut line = String::new();
        let stdout = child.stdout.take().expect("stdout is piped");
        BufReader::new(stdout).read_line(&mut line).expect("the ready line is readable");
        let ready = serde_json::from_str::<Value>(&line).expect("the ready line is JSON");
        let url =
            |key: &str| ready[key].as_str().expect("the ready line has every url").to_string();

        assert_eq!(ready["network"], "signet", "{ready}");

        Self { control: url("control"), esplora: url("esplora"), electrum: url("electrum"), child }
    }

    /// A control API call, with `Value::Null` sending no body
    fn control(&self, method: &str, path: &str, body: &Value) -> (u16, Value) {
        let body = if body.is_null() { String::new() } else { body.to_string() };
        let (status, body) = http(&self.control, method, path, &body);

        (status, serde_json::from_str(&body).expect("control responses are JSON"))
    }

    /// A control API call that must succeed
    fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, reply) = self.control(method, path, &body);
        assert_eq!(status, 200, "{method} {path}: {reply}");

        reply
    }

    fn esplora_json(&self, path: &str) -> Value {
        let (status, body) = http(&self.esplora, "GET", path, "");
        assert_eq!(status, 200, "GET {path}: {body}");

        serde_json::from_str(&body).expect("esplora answers JSON")
    }

    fn broadcast_on_esplora(&self, tx: &Transaction) -> String {
        let (status, body) = http(&self.esplora, "POST", "/tx", &serialize_hex(tx));
        assert_eq!(status, 200, "esplora broadcast: {body}");

        body
    }

    fn broadcast_on_electrum(&self, tx: &Transaction) -> Value {
        let address = self.electrum.strip_prefix("tcp://").expect("electrum url is tcp");
        let mut stream = TcpStream::connect(address).expect("electrum accepts connections");
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "blockchain.transaction.broadcast",
            "params": [serialize_hex(tx)],
        });
        writeln!(stream, "{request}").expect("the request is sent");

        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).expect("electrum replies");

        serde_json::from_str(&line).expect("electrum replies JSON")
    }

    /// Sends SIGTERM and waits for the process to exit
    fn stop(mut self) -> ExitStatus {
        let terminated = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("kill runs");
        assert!(terminated.success());

        self.child.wait().expect("the fake node exits")
    }
}

impl Drop for RunningNode {
    fn drop(&mut self) {
        // already reaped after `stop`, where this fails harmlessly
        let _ = self.child.kill();
    }
}

/// One HTTP/1.1 request on a fresh connection, returning the status and body
fn http(base: &str, method: &str, path: &str, body: &str) -> (u16, String) {
    let host = base.strip_prefix("http://").expect("an http url");
    let mut stream = TcpStream::connect(host).expect("the server accepts connections");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nhost: {host}\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
    .expect("the request is sent");

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).expect("a status line");
    let status = status_line.split_whitespace().nth(1).and_then(|code| code.parse().ok());

    // the servers keep the connection open, so read exactly content-length bytes
    let mut content_length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("a header line");

        let header = header.trim();
        if header.is_empty() {
            break;
        }

        if let Some(length) = header.strip_prefix("content-length:") {
            content_length = length.trim().parse().expect("a numeric content length");
        }
    }

    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).expect("the whole body");

    (status.expect("a status code"), String::from_utf8(body).expect("a utf-8 body"))
}

/// A transaction spending `outpoint` to the external address
fn spend(outpoint: OutPoint, sats: u64) -> Transaction {
    let external = Address::from_str(EXTERNAL).unwrap().assume_checked();

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: external.script_pubkey(),
        }],
    }
}

fn outpoint(output: &Value) -> OutPoint {
    let txid = Txid::from_str(output["txid"].as_str().expect("a txid")).expect("a valid txid");

    OutPoint { txid, vout: output["vout"].as_u64().expect("a vout") as u32 }
}

#[test]
fn ui_test_fixture_flow_runs_over_the_control_api() {
    let node = RunningNode::start();

    let health = node.ok("GET", "/health", Value::Null);
    assert_eq!(health["ok"], true);
    assert_eq!(
        (health["esplora"].as_str(), health["electrum"].as_str()),
        (Some(node.esplora.as_str()), Some(node.electrum.as_str()))
    );
    assert_eq!(
        (health["network"].as_str(), health["tip"].as_u64()),
        (Some("signet"), Some(u64::from(RESET_BLOCKS)))
    );

    // fund-wallet: one confirmed UTXO per output at the mnemonic's receive addresses
    let funded = node.ok(
        "POST",
        "/fund-wallet",
        json!({
            "mnemonic": MNEMONIC,
            "account_path": "m/84'/1'/0'",
            "outputs": [
                { "index": 0, "sats": 100_000 },
                { "index": 1, "sats": 60_000 },
                { "index": 2, "sats": 30_000 },
            ],
            "confirm": true,
        }),
    );
    let outputs = funded["outputs"].as_array().expect("funded outputs").clone();
    let [a, b, c] = [&outputs[0], &outputs[1], &outputs[2]];
    assert_eq!(outputs.len(), 3, "{funded}");
    assert_eq!((a["address"].as_str(), a["sats"].as_u64()), (Some(FIRST_RECEIVE), Some(100_000)));
    assert_ne!(a["txid"], b["txid"], "each amount is its own funding transaction");

    let stats = node.esplora_json(&format!("/address/{FIRST_RECEIVE}"));
    assert_eq!(stats["chain_stats"]["funded_txo_sum"], 100_000, "{stats}");

    let status = node.esplora_json(&format!("/tx/{}/status", c["txid"].as_str().unwrap()));
    assert_eq!(status["block_height"], RESET_BLOCKS + 1, "{status}");

    // fund without confirming, then mine: the first mined block confirms the mempool
    let pending =
        node.ok("POST", "/fund", json!({ "address": EXTERNAL, "sats": [5_000], "confirm": false }));
    let pending_txid = pending["outputs"][0]["txid"].as_str().unwrap().to_string();
    assert_eq!(node.esplora_json(&format!("/tx/{pending_txid}/status"))["confirmed"], false);

    let mined = node.ok("POST", "/mine", json!({ "blocks": 2 }));
    assert_eq!(mined["tip"], RESET_BLOCKS + 3);

    let status = node.esplora_json(&format!("/tx/{pending_txid}/status"));
    assert_eq!(
        (status["confirmed"].as_bool(), status["block_height"].as_u64()),
        (Some(true), Some(u64::from(RESET_BLOCKS + 2)))
    );

    // broadcasts on both protocols are logged once each, in order
    let spend_a = spend(outpoint(a), 99_000);
    let spend_b = spend(outpoint(b), 59_000);
    assert_eq!(node.broadcast_on_esplora(&spend_a), spend_a.compute_txid().to_string());
    assert_eq!(node.broadcast_on_electrum(&spend_b)["result"], spend_b.compute_txid().to_string());
    node.broadcast_on_esplora(&spend_a);

    let broadcasts = node.ok("GET", "/broadcasts", Value::Null);
    assert_eq!(
        broadcasts,
        json!([
            {
                "txid": spend_a.compute_txid().to_string(),
                "inputs": [outpoint(a).to_string()],
                "outputs": [{ "address": EXTERNAL, "sats": 99_000 }],
            },
            {
                "txid": spend_b.compute_txid().to_string(),
                "inputs": [outpoint(b).to_string()],
                "outputs": [{ "address": EXTERNAL, "sats": 59_000 }],
            },
        ])
    );

    // outspend tells spent from unspent outputs and rejects unknown ones
    let outspend = |output: &Value| {
        let outpoint = outpoint(output);
        node.ok("GET", &format!("/outspend/{}/{}", outpoint.txid, outpoint.vout), Value::Null)
    };
    assert_eq!(outspend(a), json!({ "spent": true, "spending_txid": spend_a.compute_txid() }));
    assert_eq!(outspend(c), json!({ "spent": false, "spending_txid": null }));

    let unknown = format!("/outspend/{}/1", outpoint(c).txid);
    assert_eq!(node.control("GET", &unknown, &Value::Null).0, 404);

    node.ok("POST", "/mine", Value::Null);
    let status = node.esplora_json(&format!("/tx/{}/status", spend_a.compute_txid()));
    assert_eq!(status["block_height"], RESET_BLOCKS + 4, "{status}");

    // bad input is a 400 with a JSON error rather than a dropped connection
    let mainnet = json!({ "address": "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "sats": [1] });
    let (status, error) = node.control("POST", "/fund", &mainnet);
    assert_eq!(status, 400, "{error}");
    assert!(error["error"].as_str().unwrap().contains("invalid address"), "{error}");

    let typo = json!({ "mnemonic": MNEMONIC, "outputs": [{ "index": 0, "amount": 1 }] });
    assert_eq!(node.control("POST", "/fund-wallet", &typo).0, 400);

    // reset starts over: fresh tip, empty broadcast log, funded outputs gone
    assert_eq!(node.ok("POST", "/reset", Value::Null)["tip"], RESET_BLOCKS);
    assert_eq!(node.ok("GET", "/broadcasts", Value::Null), json!([]));
    let funded_before_reset = format!("/outspend/{}/0", outpoint(c).txid);
    assert_eq!(node.control("GET", &funded_before_reset, &Value::Null).0, 404);

    let status = node.stop();
    assert!(status.success(), "SIGTERM is a clean exit: {status}");
}

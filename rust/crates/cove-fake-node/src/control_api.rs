//! JSON API that lets another process drive a fake node
//!
//! UI tests run the app in a simulator or emulator, so they set up the chain
//! and check what the app broadcast over HTTP instead of calling [`FakeChain`]
//! directly. Every response is JSON; errors are `{"error": "…"}` with a 400 or
//! 404 status

use std::{io, net::SocketAddr, str::FromStr as _, sync::Arc};

use bip39::Mnemonic;
use bitcoin::{
    Address, Amount, Network, OutPoint, Txid,
    bip32::{self, ChildNumber, DerivationPath, Xpriv, Xpub},
    hex::HexToArrayError,
    secp256k1::Secp256k1,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    sync::watch,
};

use crate::{
    Server, Shared,
    chain::FakeChain,
    http::{Request, Response, read_request, write_response},
};

/// How many empty blocks follow genesis after a reset, so the wallet's first
/// scan sees a chain with history rather than a lone genesis block
pub const RESET_BLOCKS: u32 = 10;

/// The control API server, which stops when dropped
pub struct ControlServer(Server);

impl ControlServer {
    pub(crate) fn start(
        address: SocketAddr,
        shared: Arc<Shared>,
        esplora_url: String,
        electrum_url: String,
    ) -> io::Result<Self> {
        let urls = Arc::new(NodeUrls { esplora: esplora_url, electrum: electrum_url });
        let server = Server::start(address, shared, move |listener, shared, shutdown| {
            serve(listener, shared, shutdown, urls)
        })?;

        Ok(Self(server))
    }

    /// The control API base URL, such as `http://127.0.0.1:39000`
    pub fn url(&self) -> String {
        format!("http://{}", self.0.address)
    }
}

struct NodeUrls {
    esplora: String,
    electrum: String,
}

#[derive(Debug)]
struct ApiError {
    status: u16,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self { status: 400, message: message.into() }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self { status: 404, message: message.into() }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::bad_request(format!("invalid request body: {error}"))
    }
}

impl From<bitcoin::address::ParseError> for ApiError {
    fn from(error: bitcoin::address::ParseError) -> Self {
        Self::bad_request(format!("invalid address: {error}"))
    }
}

impl From<bip39::Error> for ApiError {
    fn from(error: bip39::Error) -> Self {
        Self::bad_request(format!("invalid mnemonic: {error}"))
    }
}

impl From<bip32::Error> for ApiError {
    fn from(error: bip32::Error) -> Self {
        Self::bad_request(format!("invalid derivation: {error}"))
    }
}

impl From<HexToArrayError> for ApiError {
    fn from(error: HexToArrayError) -> Self {
        Self::bad_request(format!("invalid txid: {error}"))
    }
}

const fn confirm_by_default() -> bool {
    true
}

const fn one_block() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundRequest {
    address: String,
    sats: Vec<u64>,
    #[serde(default = "confirm_by_default")]
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundWalletRequest {
    mnemonic: String,
    account_path: Option<String>,
    outputs: Vec<WalletOutput>,
    #[serde(default = "confirm_by_default")]
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WalletOutput {
    index: u32,
    sats: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MineRequest {
    #[serde(default = "one_block")]
    blocks: u32,
}

/// Accepts connections until `shutdown` flips, then drops the listener
async fn serve(
    listener: TcpListener,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
    urls: Arc<NodeUrls>,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                tokio::spawn(serve_connection(stream, shared.clone(), urls.clone()));
            }

            _ = shutdown.wait_for(|stop| *stop) => return,
        }
    }
}

async fn serve_connection(stream: TcpStream, shared: Arc<Shared>, urls: Arc<NodeUrls>) {
    let mut reader = BufReader::new(stream);

    while let Some(request) = read_request(&mut reader).await {
        let response = match route(&shared, &urls, &request) {
            Ok(value) => Response::json(&value),
            Err(ApiError { status, message }) => {
                Response::json_with_status(status, &json!({ "error": message }))
            }
        };

        if write_response(reader.get_mut(), &response).await.is_err() {
            return;
        }
    }
}

fn route(shared: &Shared, urls: &NodeUrls, request: &Request) -> Result<Value, ApiError> {
    let chain = &shared.chain;
    let segments = request.path.trim_matches('/').split('/').collect::<Vec<_>>();

    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["health"]) => Ok(json!({
            "ok": true,
            "esplora": urls.esplora,
            "electrum": urls.electrum,
            "network": chain.network().to_string(),
            "tip": chain.tip_height(),
        })),

        ("POST", ["reset"]) => {
            shared.reset();

            Ok(json!({ "ok": true, "tip": chain.tip_height() }))
        }

        ("POST", ["fund"]) => fund(chain, body(&request.body)?),
        ("POST", ["fund-wallet"]) => fund_wallet(chain, body(&request.body)?),
        ("POST", ["mine"]) => mine(chain, body(&request.body)?),
        ("GET", ["broadcasts"]) => Ok(broadcasts(chain)),
        ("GET", ["outspend", txid, vout]) => outspend(chain, txid, vout),

        (method, _) => Err(ApiError::not_found(format!("no route for {method} {}", request.path))),
    }
}

/// Parses a JSON body, treating an empty one as `{}` so optional fields take
/// their defaults
fn body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    let body = if body.trim_ascii().is_empty() { b"{}".as_slice() } else { body };

    Ok(serde_json::from_slice(body)?)
}

fn fund(chain: &FakeChain, request: FundRequest) -> Result<Value, ApiError> {
    let address = Address::from_str(&request.address)?.require_network(chain.network())?;
    let outputs = request.sats.into_iter().map(|sats| (address.clone(), sats)).collect();

    pay_outputs(chain, outputs, request.confirm)
}

/// Funds BIP84 receive addresses of a mnemonic, so a test can fund the wallet
/// it imports without hardcoding its addresses
fn fund_wallet(chain: &FakeChain, request: FundWalletRequest) -> Result<Value, ApiError> {
    let network = chain.network();
    let mnemonic = Mnemonic::parse(&request.mnemonic)?;
    let account = match request.account_path {
        Some(path) => DerivationPath::from_str(&path)?,
        None => default_account_path(network),
    };

    let secp = Secp256k1::new();
    let master = Xpriv::new_master(network, &mnemonic.to_seed(""))?;
    let receive = master.derive_priv(&secp, &account.child(ChildNumber::Normal { index: 0 }))?;

    let outputs = request
        .outputs
        .into_iter()
        .map(|output| {
            let key = receive.derive_priv(&secp, &[ChildNumber::from_normal_idx(output.index)?])?;
            let address = Address::p2wpkh(&Xpub::from_priv(&secp, &key).to_pub(), network);

            Ok((address, output.sats))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    pay_outputs(chain, outputs, request.confirm)
}

/// The account BIP84 uses for the first native segwit account on `network`
fn default_account_path(network: Network) -> DerivationPath {
    let coin_type = if network == Network::Bitcoin { 0 } else { 1 };

    DerivationPath::from(vec![
        ChildNumber::Hardened { index: 84 },
        ChildNumber::Hardened { index: coin_type },
        ChildNumber::Hardened { index: 0 },
    ])
}

/// Pays each output from its own funding transaction, so each is its own UTXO,
/// and confirms them all in one block when `confirm` is set
fn pay_outputs(
    chain: &FakeChain,
    outputs: Vec<(Address, u64)>,
    confirm: bool,
) -> Result<Value, ApiError> {
    if outputs.is_empty() {
        return Err(ApiError::bad_request("at least one output is required"));
    }

    if outputs.iter().any(|(_, sats)| *sats == 0) {
        return Err(ApiError::bad_request("every output needs a positive amount"));
    }

    let paid = outputs
        .into_iter()
        .map(|(address, sats)| {
            (chain.pay(&address.script_pubkey(), Amount::from_sat(sats)), address, sats)
        })
        .collect::<Vec<_>>();

    if confirm {
        let txids = paid.iter().map(|(txid, _, _)| *txid).collect::<Vec<_>>();
        chain.confirm_together(&txids);
    }

    let outputs = paid
        .iter()
        .map(|(txid, address, sats)| {
            json!({ "txid": txid, "vout": 0, "sats": sats, "address": address.to_string() })
        })
        .collect::<Vec<_>>();

    Ok(json!({ "outputs": outputs }))
}

fn mine(chain: &FakeChain, request: MineRequest) -> Result<Value, ApiError> {
    if request.blocks == 0 {
        return Err(ApiError::bad_request("blocks must be at least 1"));
    }

    Ok(json!({ "tip": chain.mine_with_mempool(request.blocks) }))
}

fn broadcasts(chain: &FakeChain) -> Value {
    let network = chain.network();
    let txs = chain.broadcasts().into_iter().map(|tx| {
        let inputs = tx.input.iter().map(|input| input.previous_output.to_string());
        let outputs = tx.output.iter().map(|output| {
            let address = Address::from_script(&output.script_pubkey, network)
                .ok()
                .map(|address| address.to_string());

            json!({ "address": address, "sats": output.value.to_sat() })
        });

        json!({
            "txid": tx.compute_txid(),
            "inputs": inputs.collect::<Vec<_>>(),
            "outputs": outputs.collect::<Vec<_>>(),
        })
    });

    Value::Array(txs.collect())
}

fn outspend(chain: &FakeChain, txid: &str, vout: &str) -> Result<Value, ApiError> {
    let txid = Txid::from_str(txid)?;
    let vout = vout.parse().map_err(|_| ApiError::bad_request(format!("invalid vout {vout}")))?;
    let outpoint = OutPoint { txid, vout };

    // an unknown output is almost always a typo in the test, so say so instead of "unspent"
    if chain.prevout(&outpoint).is_none() {
        return Err(ApiError::not_found(format!("unknown output {outpoint}")));
    }

    let spending_txid = chain.spender_of(&outpoint);

    Ok(json!({ "spent": spending_txid.is_some(), "spending_txid": spending_txid }))
}

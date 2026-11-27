//! Checks that an open port is an Electrum server for the selected network before it is listed
//!
//! An open port only says something accepts connections there. The check asks the server which
//! chain it serves, so a server on another network, or a device that isn't an Electrum server at
//! all, is never offered. It sends only `server.version`, `server.features` and, for servers
//! that don't support features, block header 0, none of which tells the server anything about
//! the wallet

use std::{net::SocketAddr, time::Duration};

use bitcoin::{BlockHash, block::Header, consensus::deserialize};
use cove_tokio::FutureTimeoutExt as _;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpSocket, TcpStream},
};

use crate::network::Network;

/// How long one server's whole check may take, long enough for a busy home node and short
/// enough that a device that accepts but never answers doesn't hold up the search
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(4);

/// The longest response line read, far more than any of the checked responses needs, so a
/// device that streams without end can't exhaust memory
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// The protocol version the check asks for, which every maintained Electrum server supports
const PROTOCOL_VERSION: &str = "1.4";

/// What a verified server says about itself
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdentity {
    /// The server software's name, such as `Fulcrum` or `electrs`, when it reported one
    pub software: Option<String>,
}

/// Why a server wasn't listed
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("could not connect: {0}")]
    Connect(#[from] std::io::Error),

    #[error("the check took longer than {} seconds", VERIFY_TIMEOUT.as_secs())]
    TimedOut,

    #[error("not an Electrum server: {0}")]
    NotElectrum(String),

    #[error("serves another network, its genesis block is {0}")]
    WrongNetwork(BlockHash),
}

/// Checks the server at `address` over `socket`, which carries the search's network binding
pub async fn verify(
    socket: TcpSocket,
    address: SocketAddr,
    network: Network,
) -> Result<ServerIdentity, VerifyError> {
    let check = async {
        let stream = socket.connect(address).await?;
        let mut connection = Connection::new(stream);

        connection.check(network).await
    };

    check.with_timeout(VERIFY_TIMEOUT).await.map_err(|_| VerifyError::TimedOut)?
}

/// One newline-delimited JSON-RPC connection to a server being checked
struct Connection {
    stream: BufReader<TcpStream>,
    next_id: u64,
}

impl Connection {
    fn new(stream: TcpStream) -> Self {
        Self { stream: BufReader::new(stream), next_id: 0 }
    }

    async fn check(&mut self, network: Network) -> Result<ServerIdentity, VerifyError> {
        let expected = genesis_hash(network);

        // the version is only for the software name, the network proof comes after it
        let version = self.call("server.version", json!(["Cove", PROTOCOL_VERSION])).await?;
        let version_software =
            version.ok().and_then(|result| result.get(0)?.as_str().map(software_name));

        let features = self.call("server.features", json!([])).await?;
        let reported_genesis = features.as_ref().ok().and_then(|features| {
            let hash = features.get("genesis_hash")?.as_str()?;
            hash.parse::<BlockHash>().ok()
        });

        let genesis = match reported_genesis {
            Some(genesis) => genesis,
            None => self.genesis_from_header().await?,
        };

        if genesis != expected {
            return Err(VerifyError::WrongNetwork(genesis));
        }

        let features_software = features
            .ok()
            .and_then(|features| features.get("server_version")?.as_str().map(software_name));

        Ok(ServerIdentity { software: features_software.or(version_software) })
    }

    /// The hash of block header 0, for servers whose features don't name their genesis block
    async fn genesis_from_header(&mut self) -> Result<BlockHash, VerifyError> {
        let header = self
            .call("blockchain.block.header", json!([0]))
            .await?
            .map_err(|error| VerifyError::NotElectrum(format!("header 0 failed: {error}")))?;

        let bytes = header
            .as_str()
            .and_then(|hex| hex::decode(hex).ok())
            .ok_or_else(|| VerifyError::NotElectrum(format!("header 0 is not hex: {header}")))?;

        let header: Header = deserialize(&bytes)
            .map_err(|error| VerifyError::NotElectrum(format!("header 0 is invalid: {error}")))?;

        Ok(header.block_hash())
    }

    /// Sends one request and reads its response, returning the server's error as `Err`
    ///
    /// The outer error is for a connection that isn't speaking Electrum at all
    async fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, Value>, VerifyError> {
        let id = self.next_id;
        self.next_id += 1;

        let mut request =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        request.push('\n');
        self.stream.get_mut().write_all(request.as_bytes()).await?;

        let mut line = Vec::new();
        (&mut self.stream).take(MAX_RESPONSE_BYTES).read_until(b'\n', &mut line).await?;
        if !line.ends_with(b"\n") {
            return Err(VerifyError::NotElectrum(format!("no complete response to {method}")));
        }

        let response: Value = serde_json::from_slice(&line)
            .map_err(|error| VerifyError::NotElectrum(format!("{method} response: {error}")))?;

        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(VerifyError::NotElectrum(format!("{method} response has another id")));
        }

        if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
            return Ok(Err(error.clone()));
        }

        match response.get("result") {
            Some(result) => Ok(Ok(result.clone())),
            None => Err(VerifyError::NotElectrum(format!("{method} response has no result"))),
        }
    }
}

/// The selected network's genesis block hash, which the server's must match
fn genesis_hash(network: Network) -> BlockHash {
    bitcoin::constants::genesis_block(bitcoin::Network::from(network)).block_hash()
}

/// The software's name without its version, such as `Fulcrum` from `Fulcrum 1.9.8` or `electrs`
/// from `electrs/0.10.6`, since a saved node keeps its name after the server updates
fn software_name(version: &str) -> String {
    version.split([' ', '/']).find(|part| !part.is_empty()).unwrap_or(version).to_string()
}

#[cfg(test)]
pub(crate) mod tests;

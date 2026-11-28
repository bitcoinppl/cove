use std::net::Ipv4Addr;

use bitcoin::consensus::serialize;
use tokio::net::TcpListener;

use super::*;

/// How a test server answers the check's requests
#[derive(Debug, Clone, Copy)]
pub enum Behavior {
    /// An Electrum server on `network` that reports its genesis in `server.features`
    Electrum { network: Network },
    /// An Electrum server on `network` whose `server.features` fails, so only header 0 proves
    /// its chain
    WithoutFeatures { network: Network },
    /// Something other than Electrum, answering every line with an HTTP error
    Http,
    /// Accepts and never answers
    Silent,
}

/// A local server that answers the check the way `behavior` says, returning its address
pub async fn serve(behavior: Behavior) -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            tokio::spawn(answer(stream, behavior));
        }
    });

    address
}

async fn answer(stream: TcpStream, behavior: Behavior) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let request: Value = serde_json::from_str(&line).unwrap();
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default();

        let response = match behavior {
            Behavior::Silent => continue,
            Behavior::Http => "HTTP/1.1 400 Bad Request\r\n\r\n".to_string(),
            Behavior::Electrum { network } => electrum_response(id, method, network, true),
            Behavior::WithoutFeatures { network } => electrum_response(id, method, network, false),
        };

        if writer.write_all(format!("{response}\n").as_bytes()).await.is_err() {
            return;
        }
    }
}

fn electrum_response(id: Value, method: &str, network: Network, has_features: bool) -> String {
    let genesis = bitcoin::constants::genesis_block(bitcoin::Network::from(network));

    let result = match method {
        "server.version" => json!(["Fulcrum 1.9.8", "1.4"]),
        "server.features" if has_features => json!({
            "server_version": "Fulcrum 1.9.8",
            "genesis_hash": genesis.block_hash().to_string(),
            "protocol_min": "1.4",
            "protocol_max": "1.4.5",
            "hash_function": "sha256",
        }),
        "blockchain.block.header" => json!(hex::encode(serialize(&genesis.header))),
        _ => {
            let error = json!({ "code": -32601, "message": "unknown method" });
            return json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string();
        }
    };

    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

async fn check(address: SocketAddr, network: Network) -> Result<ServerIdentity, VerifyError> {
    verify(TcpSocket::new_v4().unwrap(), address, network).await
}

#[tokio::test]
async fn a_server_on_the_selected_network_is_verified_and_named() {
    let address = serve(Behavior::Electrum { network: Network::Signet }).await;

    let identity = check(address, Network::Signet).await.unwrap();

    assert_eq!(identity.software.as_deref(), Some("Fulcrum"));
}

#[tokio::test]
async fn a_server_on_another_network_is_rejected() {
    let address = serve(Behavior::Electrum { network: Network::Signet }).await;

    let error = check(address, Network::Bitcoin).await.unwrap_err();

    assert!(matches!(error, VerifyError::WrongNetwork(_)), "{error:?}");
}

// header 0 is the fallback proof, so it must hold servers without features to the same network
#[tokio::test]
async fn a_server_without_features_is_checked_by_its_first_header() {
    let address = serve(Behavior::WithoutFeatures { network: Network::Testnet }).await;

    let identity = check(address, Network::Testnet).await.unwrap();
    assert_eq!(identity.software.as_deref(), Some("Fulcrum"));

    let error = check(address, Network::Bitcoin).await.unwrap_err();
    assert!(matches!(error, VerifyError::WrongNetwork(_)), "{error:?}");
}

#[tokio::test]
async fn something_other_than_electrum_is_rejected() {
    let address = serve(Behavior::Http).await;

    let error = check(address, Network::Bitcoin).await.unwrap_err();

    assert!(matches!(error, VerifyError::NotElectrum(_)), "{error:?}");
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_times_out() {
    let address = serve(Behavior::Silent).await;

    let error = check(address, Network::Bitcoin).await.unwrap_err();

    assert!(matches!(error, VerifyError::TimedOut), "{error:?}");
}

#[test]
fn software_names_drop_their_version() {
    assert_eq!(software_name("Fulcrum 1.9.8"), "Fulcrum");
    assert_eq!(software_name("electrs/0.10.6"), "electrs");
    assert_eq!(software_name("ElectrumX 1.16.0"), "ElectrumX");
    assert_eq!(software_name("fake-electrum 0.1"), "fake-electrum");
}

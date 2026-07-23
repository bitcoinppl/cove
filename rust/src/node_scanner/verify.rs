use std::time::{Duration, Instant};

use bdk_electrum::electrum_client::{Client, ConfigBuilder, ElectrumApi as _, Param};
use bitcoin::hashes::Hash as _;
use cove_types::network::Network;
use cove_util::ResultExt as _;

use crate::node::client::electrum::transport::{self, Transport};
use crate::node::tls::{self, TlsTrust};
use crate::node_connect::NodeCertificate;

pub(crate) const ELECTRUM_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const VERIFICATION_OBSERVER_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct VerifiedMetadata {
    pub(crate) server_software: String,
    pub(crate) protocol_version: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VerifyError {
    #[error("the server certificate was rejected")]
    CertificateRejected,
    #[error("the server certificate changed")]
    CertificateChanged,
    #[error("the endpoint did not prove the selected Bitcoin network")]
    WrongNetwork,
    #[error("electrum verification failed: {0}")]
    Electrum(String),
    #[error("verification exceeded its deadline")]
    TimedOut,
}

impl From<transport::ConnectError> for VerifyError {
    fn from(error: transport::ConnectError) -> Self {
        match error {
            transport::ConnectError::CertificateRejected(_) => Self::CertificateChanged,
            error => Self::Electrum(error.to_string()),
        }
    }
}

pub(crate) fn verify(
    url: &str,
    network: Network,
    trust: Option<&TlsTrust>,
) -> Result<VerifiedMetadata, VerifyError> {
    let deadline = Instant::now() + VERIFICATION_OBSERVER_TIMEOUT;
    let transport = match trust {
        Some(trust) => Transport::connect_pinned_for_scan(url, trust, ELECTRUM_TIMEOUT)?,
        None => {
            let config = ConfigBuilder::new().timeout(Some(ELECTRUM_TIMEOUT)).retry(0).build();
            let client = Client::from_config(url, config).map_err(electrum_error)?;

            Transport::Default(Box::new(client))
        }
    };

    verify_transport(&transport, network, deadline)
}

fn verify_transport(
    transport: &Transport,
    network: Network,
    deadline: Instant,
) -> Result<VerifiedMetadata, VerifyError> {
    let negotiated = match transport {
        Transport::Default(_) => None,
        Transport::Pinned(_) => Some(best_effort_server_version(transport, deadline)),
    };

    ensure_before(deadline)?;
    if let Ok(features) = transport.server_features() {
        if features.genesis_hash != conventional_genesis_hash(network) {
            return Err(VerifyError::WrongNetwork);
        }

        return Ok(VerifiedMetadata {
            server_software: features.server_version,
            protocol_version: features.protocol_max,
        });
    }

    ensure_before(deadline)?;
    let header = transport.block_header(0).map_err_str(VerifyError::Electrum)?;
    let bitcoin_network = bitcoin::Network::from(network);
    let expected = bitcoin::constants::genesis_block(bitcoin_network).block_hash();
    if header.block_hash() != expected {
        return Err(VerifyError::WrongNetwork);
    }

    Ok(negotiated.unwrap_or_else(|| best_effort_server_version(transport, deadline)))
}

fn best_effort_server_version(transport: &Transport, deadline: Instant) -> VerifiedMetadata {
    if ensure_before(deadline).is_err() {
        return generic_metadata();
    }

    let response = transport.raw_call(
        "server.version",
        [
            Param::String("Cove".to_string()),
            Param::StringVec(vec!["1.4".to_string(), "1.6".to_string()]),
        ],
    );

    let Ok(serde_json::Value::Array(values)) = response else {
        return generic_metadata();
    };
    let mut values = values.into_iter();
    let Some(server_software) = values.next().and_then(|value| value.as_str().map(str::to_string))
    else {
        return generic_metadata();
    };
    let Some(protocol_version) = values.next().and_then(|value| value.as_str().map(str::to_string))
    else {
        return generic_metadata();
    };

    VerifiedMetadata { server_software, protocol_version }
}

fn ensure_before(deadline: Instant) -> Result<(), VerifyError> {
    if Instant::now() >= deadline {
        return Err(VerifyError::TimedOut);
    }

    Ok(())
}

fn generic_metadata() -> VerifiedMetadata {
    VerifiedMetadata {
        server_software: "Electrum server".to_string(),
        protocol_version: "Unknown".to_string(),
    }
}

pub(crate) fn capture_certificate(url: &str) -> Result<NodeCertificate, VerifyError> {
    let certificate = transport::peer_certificate_with_timeout(url, ELECTRUM_TIMEOUT)
        .map_err_str(VerifyError::Electrum)?;
    let sha256 = tls::fingerprint(&certificate);

    Ok(NodeCertificate { sha256: sha256.to_vec(), display: tls::display_fingerprint(&sha256) })
}

fn conventional_genesis_hash(network: Network) -> [u8; 32] {
    let bitcoin_network = bitcoin::Network::from(network);
    let mut bytes = bitcoin::constants::genesis_block(bitcoin_network).block_hash().to_byte_array();
    bytes.reverse();
    bytes
}

fn is_certificate_rejected(error: &bdk_electrum::electrum_client::Error) -> bool {
    use bdk_electrum::electrum_client::Error;

    let Error::IOError(error) = error else { return false };

    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|error| matches!(error, rustls::Error::InvalidCertificate(_)))
}

fn electrum_error(error: bdk_electrum::electrum_client::Error) -> VerifyError {
    if is_certificate_rejected(&error) {
        VerifyError::CertificateRejected
    } else {
        VerifyError::Electrum(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::client::electrum::test_server::{
        BITCOIN_GENESIS_HASH, TESTNET_GENESIS_HASH, TestServer, self_signed, setup,
    };

    #[test]
    fn electrum_genesis_hash_uses_conventional_byte_order() {
        assert_eq!(
            hex::encode(conventional_genesis_hash(Network::Bitcoin)),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        );
    }

    #[test]
    fn certificate_capture_sends_zero_electrum_requests() {
        setup();
        let server = TestServer::self_signed("fulcrum.local");
        let url = format!("ssl://127.0.0.1:{}", server.port);

        assert!(matches!(
            verify(&url, Network::Bitcoin, None),
            Err(VerifyError::CertificateRejected)
        ));
        assert_eq!(server.request_count(), 0);

        let certificate = capture_certificate(&url).unwrap();
        assert_eq!(
            certificate.sha256,
            match server.fingerprint_trust() {
                TlsTrust::PinnedFingerprint { sha256 } => sha256,
                TlsTrust::CustomCa { .. } => unreachable!(),
            }
        );
        assert_eq!(server.request_count(), 0);
    }

    #[test]
    fn confirmed_fingerprint_promotes_only_the_captured_certificate() {
        setup();
        let server = TestServer::self_signed("fulcrum.local");
        let url = format!("ssl://127.0.0.1:{}", server.port);
        let metadata = verify(&url, Network::Bitcoin, Some(&server.fingerprint_trust())).unwrap();

        assert_eq!(metadata.server_software, "Fulcrum test");
        assert_eq!(metadata.protocol_version, "1.4");

        let other = TlsTrust::PinnedFingerprint {
            sha256: tls::fingerprint(&self_signed("other.local").0).to_vec(),
        };
        assert!(matches!(
            verify(&url, Network::Bitcoin, Some(&other)),
            Err(VerifyError::CertificateChanged)
        ));
    }

    #[test]
    fn server_features_accepts_matching_and_rejects_wrong_genesis() {
        setup();
        let matching =
            TestServer::self_signed_with_genesis("localhost", BITCOIN_GENESIS_HASH, true);
        let matching_url = format!("ssl://127.0.0.1:{}", matching.port);
        assert!(
            verify(&matching_url, Network::Bitcoin, Some(&matching.fingerprint_trust())).is_ok()
        );

        let wrong = TestServer::self_signed_with_genesis("localhost", TESTNET_GENESIS_HASH, true);
        let wrong_url = format!("ssl://127.0.0.1:{}", wrong.port);
        assert!(matches!(
            verify(&wrong_url, Network::Bitcoin, Some(&wrong.fingerprint_trust())),
            Err(VerifyError::WrongNetwork)
        ));
    }

    #[test]
    fn header_zero_fallback_accepts_matching_and_rejects_wrong_network() {
        setup();
        let matching =
            TestServer::self_signed_with_genesis("localhost", BITCOIN_GENESIS_HASH, false);
        let matching_url = format!("ssl://127.0.0.1:{}", matching.port);
        let metadata =
            verify(&matching_url, Network::Bitcoin, Some(&matching.fingerprint_trust())).unwrap();
        assert_eq!(metadata.server_software, "Cove test server");

        let wrong = TestServer::self_signed_with_genesis("localhost", TESTNET_GENESIS_HASH, false);
        let wrong_url = format!("ssl://127.0.0.1:{}", wrong.port);
        assert!(matches!(
            verify(&wrong_url, Network::Testnet, Some(&wrong.fingerprint_trust())),
            Err(VerifyError::WrongNetwork)
        ));
    }

    #[test]
    fn trusted_non_electrum_endpoint_never_promotes() {
        setup();
        let server = TestServer::non_electrum("localhost");
        let url = format!("ssl://127.0.0.1:{}", server.port);

        assert!(matches!(
            verify(&url, Network::Bitcoin, Some(&server.fingerprint_trust())),
            Err(VerifyError::Electrum(_))
        ));
    }
}

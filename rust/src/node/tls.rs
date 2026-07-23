use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest as _, Sha256};

/// How a node's TLS certificate is verified.
///
/// A node with no [`TlsTrust`] is verified against the bundled webpki roots,
/// which is what every node did before this type existed.
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum, serde::Serialize, serde::Deserialize)]
pub enum TlsTrust {
    /// Verify the chain against a user supplied CA, still checking the hostname.
    /// The leaf may rotate without invalidating the setting, so this suits a
    /// self hosted certificate authority.
    CustomCa { cert: Vec<u8> },

    /// Accept exactly one leaf certificate, identified by the SHA-256 of its DER
    /// encoding. The hostname is not checked, so this also covers certificates
    /// issued without a matching SAN, which is common for servers reached by IP.
    ///
    /// Expiry is not checked either: the certificate is trusted because the user
    /// chose it, not because an authority vouched for it, so it stays valid until
    /// they replace it.
    PinnedFingerprint { sha256: Vec<u8> },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("certificate is not valid PEM or DER")]
    InvalidCertificate,

    #[error("certificate cannot be used as a trust anchor: {0}")]
    UntrustedCertificate(rustls::Error),

    #[error("certificate fingerprint must be 32 bytes, got {0}")]
    InvalidFingerprintLength(usize),

    #[error("failed to build TLS configuration: {0}")]
    Config(rustls::Error),
}

/// SHA-256 of a certificate's DER encoding, the value shown to the user when
/// they confirm a pin.
pub fn fingerprint(cert: &CertificateDer<'_>) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(cert.as_ref()));
    out
}

/// Accept whichever encoding the user's server exposes.
pub fn parse_certificate(bytes: &[u8]) -> Result<CertificateDer<'static>, Error> {
    if let Ok(cert) = CertificateDer::from_pem_slice(bytes) {
        return Ok(cert);
    }

    // Not PEM, so treat it as raw DER. A certificate is a SEQUENCE, and rejecting
    // anything else here keeps obviously wrong input from becoming a trust anchor.
    if bytes.first() == Some(&0x30) {
        return Ok(CertificateDer::from(bytes.to_vec()));
    }

    Err(Error::InvalidCertificate)
}

/// Build a rustls config that trusts exactly what `trust` describes and nothing else.
pub fn client_config(trust: &TlsTrust) -> Result<ClientConfig, Error> {
    // Own the provider rather than relying on the process-wide default, so the
    // config is correct regardless of bootstrap order.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(Error::Config)?;

    match trust {
        TlsTrust::CustomCa { cert } => {
            let mut roots = RootCertStore::empty();
            roots.add(parse_certificate(cert)?).map_err(Error::UntrustedCertificate)?;
            Ok(builder.with_root_certificates(roots).with_no_client_auth())
        }

        TlsTrust::PinnedFingerprint { sha256 } => {
            let expected: [u8; 32] = sha256
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidFingerprintLength(sha256.len()))?;

            Ok(builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedFingerprint {
                    expected,
                    provider,
                }))
                .with_no_client_auth())
        }
    }
}

/// Config for reading a server's certificate without judging it.
///
/// The certificate this accepts is only ever shown to the user so they can
/// compare it against their server; it must not be used to carry traffic. An
/// attacker in the path can present their own certificate here, which is why
/// the fingerprint has to be confirmed out of band before it is pinned.
pub(crate) fn capture_config(capture: Arc<CapturedCertificate>) -> Result<ClientConfig, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    capture.provider.get_or_init(|| provider.clone());

    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(Error::Config)?
        .dangerous()
        .with_custom_certificate_verifier(capture)
        .with_no_client_auth())
}

#[derive(Debug, Default)]
pub(crate) struct CapturedCertificate {
    certificate: std::sync::Mutex<Option<CertificateDer<'static>>>,
    provider: std::sync::OnceLock<Arc<CryptoProvider>>,
}

impl CapturedCertificate {
    fn algorithms(&self) -> &rustls::crypto::WebPkiSupportedAlgorithms {
        &self
            .provider
            .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
            .signature_verification_algorithms
    }
}

impl CapturedCertificate {
    pub(crate) fn take(&self) -> Option<CertificateDer<'static>> {
        self.certificate.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

impl ServerCertVerifier for CapturedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut captured =
            self.certificate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *captured = Some(end_entity.clone().into_owned());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, self.algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, self.algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms().supported_schemes()
    }
}

/// Colon separated uppercase hex, the form `openssl` and other wallets show.
pub fn display_fingerprint(sha256: &[u8]) -> String {
    sha256.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":")
}

/// Verifier that trusts a single leaf certificate by fingerprint.
///
/// Unlike disabling verification outright, the handshake signature is still
/// checked, so the peer must prove it holds the pinned certificate's key.
#[derive(Debug)]
struct PinnedFingerprint {
    expected: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedFingerprint {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.expected {
            return Ok(ServerCertVerified::assertion());
        }

        // Reported as a certificate rejection so the caller can tell a rejected
        // certificate apart from a connection that failed for another reason.
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::ApplicationVerificationFailure,
        ))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate() -> rcgen::CertifiedKey<rcgen::KeyPair> {
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap()
    }

    #[test]
    fn certificates_parse_from_pem_and_der() {
        let generated = certificate();
        let der = generated.cert.der().to_vec();

        assert_eq!(parse_certificate(generated.cert.pem().as_bytes()).unwrap().as_ref(), der);
        assert_eq!(parse_certificate(&der).unwrap().as_ref(), der);
    }

    #[test]
    fn text_that_is_not_a_certificate_is_rejected() {
        assert!(matches!(parse_certificate(b"hunter2"), Err(Error::InvalidCertificate)));
    }

    #[test]
    fn a_custom_ca_must_be_a_usable_trust_anchor() {
        // Passes the DER prefix check, but is not a certificate.
        let trust = TlsTrust::CustomCa { cert: vec![0x30, 0x03, 0x02, 0x01, 0x00] };

        assert!(matches!(client_config(&trust), Err(Error::UntrustedCertificate(_))));
    }

    #[test]
    fn fingerprints_display_as_colon_separated_hex() {
        assert_eq!(display_fingerprint(&[0xAB, 0x01, 0xFF]), "AB:01:FF");
    }

    #[test]
    fn a_fingerprint_must_be_a_sha256_digest() {
        let trust = TlsTrust::PinnedFingerprint { sha256: vec![0; 31] };

        assert!(matches!(client_config(&trust), Err(Error::InvalidFingerprintLength(31))));
    }

    #[test]
    fn a_valid_certificate_builds_both_configurations() {
        let generated = certificate();

        let ca = TlsTrust::CustomCa { cert: generated.cert.der().to_vec() };
        let pin = TlsTrust::PinnedFingerprint { sha256: vec![0; 32] };

        assert!(client_config(&ca).is_ok());
        assert!(client_config(&pin).is_ok());
    }
}

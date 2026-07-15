use std::{io::Write as _, str::FromStr as _, time::Duration};

use age::x25519;
use flate2::{Compression, write::GzEncoder};
use reqwest::{
    StatusCode,
    header::{CONTENT_TYPE, HeaderName},
};
use serde::Deserialize;

use super::DiagnosticsUploadReport;

const PRODUCTION_UPLOAD_URL: &str = "https://diagnostics.covebitcoinwallet.com/reports";
// this identifies public mobile clients and must not be trusted as an authorization credential
const PUBLIC_APP_TOKEN: &str = "v1.cove-mobile-2026-07";
const PRODUCTION_ENCRYPTION_KEY: DiagnosticsEncryptionKey = DiagnosticsEncryptionKey {
    id: "cove-diagnostics-2026-07",
    recipient: "age10mdkqr5jsuy0y98ezkxe9pyd3n42f7eusjx3fe5gmf38ssh8xyws4pe5lq",
};
const AGE_CONTENT_TYPE: &str = "application/age";
const ENCRYPTION_KEY_ID_HEADER: HeaderName = HeaderName::from_static("x-cove-diagnostics-key-id");
const MAX_REPORT_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_GZIPPED_REPORT_BYTES: usize = 10 * 1024 * 1024;
const MAX_SUCCESS_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_STATUS_BODY_BYTES: usize = 2 * 1024;
const MAX_UPLOAD_ATTEMPTS: usize = 2;
const UPLOAD_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);
const UPLOAD_RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct DiagnosticsEncryptionKey {
    id: &'static str,
    recipient: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UploadError {
    #[error("failed to create HTTP client: {0}")]
    Client(reqwest::Error),

    #[error("failed to encode report JSON: {0}")]
    EncodeJson(serde_json::Error),

    #[error("failed to gzip report JSON: {0}")]
    Gzip(std::io::Error),

    #[error("report JSON exceeded {limit} bytes")]
    JsonTooLarge { limit: usize },

    #[error("gzipped report exceeded {limit} bytes")]
    GzipTooLarge { limit: usize },

    #[error("configured diagnostics recipient is invalid: {0}")]
    InvalidRecipient(String),

    #[error("failed to encrypt diagnostics report: {0}")]
    Encrypt(age::EncryptError),

    #[error("collector request failed: {0}")]
    Request(reqwest::Error),

    #[error("failed to decode collector success response: {0}")]
    DecodeResponse(serde_json::Error),

    #[error("collector success response was invalid: {0}")]
    InvalidResponse(String),

    #[error("collector response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },

    #[error("collector returned status {status}: {body}")]
    Status { status: reqwest::StatusCode, body: String },
}

impl UploadError {
    pub(crate) fn user_message(&self) -> String {
        match self {
            Self::Client(_)
            | Self::EncodeJson(_)
            | Self::Gzip(_)
            | Self::InvalidRecipient(_)
            | Self::Encrypt(_) => {
                "Unable to prepare the diagnostics report. Please try again.".to_string()
            }
            Self::JsonTooLarge { .. } | Self::GzipTooLarge { .. } => {
                "The diagnostics report is too large to submit. You can still share it manually."
                    .to_string()
            }
            Self::Request(error) if error.is_connect() => {
                "Unable to reach the diagnostics collector. Check your connection and try again."
                    .to_string()
            }
            Self::Request(error) if error.is_timeout() => {
                "The diagnostics upload timed out after it may have reached the collector. Retrying could submit it again; you can share the report manually."
                    .to_string()
            }
            Self::Request(_) => {
                "The diagnostics upload failed after it may have reached the collector. Retrying could submit it again; you can share the report manually."
                    .to_string()
            }
            Self::DecodeResponse(_) | Self::InvalidResponse(_) | Self::ResponseTooLarge { .. } => {
                "The diagnostics collector returned an unexpected response after upload. The report may have been received; retrying could submit it again. You can share the report manually."
                    .to_string()
            }
            Self::Status { status, .. } if *status == StatusCode::PAYLOAD_TOO_LARGE => {
                "The diagnostics report is too large to submit. You can still share it manually."
                    .to_string()
            }
            Self::Status { status, .. } if status.is_server_error() => {
                format!(
                    "The diagnostics collector returned {status} after upload. The report may have been received; retrying could submit it again. You can share the report manually."
                )
            }
            Self::Status { status, .. } => {
                format!("The diagnostics collector rejected the report ({status}).")
            }
        }
    }

    pub(crate) fn log_message(&self) -> String {
        match self {
            Self::Status { status, .. } => format!("collector returned status {status}"),
            Self::ResponseTooLarge { limit } => {
                format!("collector response exceeded {limit} bytes")
            }
            Self::InvalidResponse(message) => {
                format!("collector success response was invalid: {message}")
            }
            Self::Client(error) => format!("failed to create HTTP client: {error}"),
            Self::EncodeJson(error) => format!("failed to encode report JSON: {error}"),
            Self::Gzip(error) => format!("failed to gzip report JSON: {error}"),
            Self::JsonTooLarge { limit } => format!("report JSON exceeded {limit} bytes"),
            Self::GzipTooLarge { limit } => format!("gzipped report exceeded {limit} bytes"),
            Self::InvalidRecipient(error) => {
                format!("configured diagnostics recipient is invalid: {error}")
            }
            Self::Encrypt(error) => format!("failed to encrypt diagnostics report: {error}"),
            Self::Request(error) => format!("collector request failed: {error}"),
            Self::DecodeResponse(error) => {
                format!("failed to decode collector success response: {error}")
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    id: String,
}

pub(crate) async fn submit_report(report: DiagnosticsUploadReport) -> Result<String, UploadError> {
    let body = cove_tokio::unblock::run_blocking(move || {
        let recipient = production_recipient(PRODUCTION_ENCRYPTION_KEY)?;

        encrypted_gzipped_json(&report, &recipient)
    })
    .await?;
    let client = cove_http::new_client_without_redirects().map_err(UploadError::Client)?;

    for attempt in 1..=MAX_UPLOAD_ATTEMPTS {
        match submit_report_once(&client, &body, PRODUCTION_ENCRYPTION_KEY).await {
            Ok(report_id) => return Ok(report_id),
            Err(error) if attempt < MAX_UPLOAD_ATTEMPTS && upload_error_is_retryable(&error) => {
                tracing::warn!(
                    "Diagnostics upload attempt {attempt} failed, retrying: {}",
                    error.log_message()
                );
                tokio::time::sleep(UPLOAD_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }

    unreachable!("upload attempts loop must return")
}

async fn submit_report_once(
    client: &reqwest::Client,
    body: &[u8],
    encryption_key: DiagnosticsEncryptionKey,
) -> Result<String, UploadError> {
    let mut response = client
        .post(upload_url())
        .timeout(UPLOAD_ATTEMPT_TIMEOUT)
        .bearer_auth(PUBLIC_APP_TOKEN)
        .header(CONTENT_TYPE, AGE_CONTENT_TYPE)
        .header(ENCRYPTION_KEY_ID_HEADER, encryption_key.id)
        .body(body.to_vec())
        .send()
        .await
        .map_err(UploadError::Request)?;
    let status = response.status();

    if !status.is_success() {
        let body = read_status_body_snippet(&mut response).await?;

        return Err(UploadError::Status { status, body });
    }

    let bytes = read_success_body(&mut response).await?;
    let response: UploadResponse =
        serde_json::from_slice(&bytes).map_err(UploadError::DecodeResponse)?;
    let report_id = response.id.trim().to_string();
    if report_id.is_empty() {
        return Err(UploadError::InvalidResponse("missing report id".to_string()));
    }

    Ok(report_id)
}

pub(crate) fn gzipped_json(report: &DiagnosticsUploadReport) -> Result<Vec<u8>, UploadError> {
    let json = serde_json::to_vec(report).map_err(UploadError::EncodeJson)?;
    if json.len() > MAX_REPORT_JSON_BYTES {
        return Err(UploadError::JsonTooLarge { limit: MAX_REPORT_JSON_BYTES });
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json).map_err(UploadError::Gzip)?;
    let gzipped = encoder.finish().map_err(UploadError::Gzip)?;
    if gzipped.len() > MAX_GZIPPED_REPORT_BYTES {
        return Err(UploadError::GzipTooLarge { limit: MAX_GZIPPED_REPORT_BYTES });
    }

    Ok(gzipped)
}

fn encrypted_gzipped_json(
    report: &DiagnosticsUploadReport,
    recipient: &x25519::Recipient,
) -> Result<Vec<u8>, UploadError> {
    let gzipped = gzipped_json(report)?;

    age::encrypt(recipient, &gzipped).map_err(UploadError::Encrypt)
}

fn production_recipient(
    encryption_key: DiagnosticsEncryptionKey,
) -> Result<x25519::Recipient, UploadError> {
    x25519::Recipient::from_str(encryption_key.recipient)
        .map_err(|error| UploadError::InvalidRecipient(error.to_string()))
}

fn upload_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("COVE_DIAGNOSTICS_URL")
        && !url.trim().is_empty()
    {
        return url;
    }

    PRODUCTION_UPLOAD_URL.to_string()
}

fn upload_error_is_retryable(error: &UploadError) -> bool {
    match error {
        // only retry pre-connection failures because POST outcomes are otherwise ambiguous
        UploadError::Request(error) => error.is_connect(),
        UploadError::Client(_)
        | UploadError::EncodeJson(_)
        | UploadError::Gzip(_)
        | UploadError::JsonTooLarge { .. }
        | UploadError::GzipTooLarge { .. }
        | UploadError::InvalidRecipient(_)
        | UploadError::Encrypt(_)
        | UploadError::Status { .. }
        | UploadError::DecodeResponse(_)
        | UploadError::InvalidResponse(_)
        | UploadError::ResponseTooLarge { .. } => false,
    }
}

async fn read_success_body(response: &mut reqwest::Response) -> Result<Vec<u8>, UploadError> {
    let mut body = Vec::new();

    while let Some(chunk) = response.chunk().await.map_err(UploadError::Request)? {
        if body.len() + chunk.len() > MAX_SUCCESS_RESPONSE_BYTES {
            return Err(UploadError::ResponseTooLarge { limit: MAX_SUCCESS_RESPONSE_BYTES });
        }

        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

async fn read_status_body_snippet(response: &mut reqwest::Response) -> Result<String, UploadError> {
    let mut body = Vec::new();
    let mut truncated = false;

    while let Some(chunk) = response.chunk().await.map_err(UploadError::Request)? {
        let remaining = MAX_STATUS_BODY_BYTES.saturating_sub(body.len());
        if remaining == 0 {
            truncated = true;
            break;
        }

        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }

        body.extend_from_slice(&chunk);
    }

    let mut snippet = String::from_utf8_lossy(&body).to_string();
    if truncated {
        snippet.push_str("... [truncated]");
    }

    Ok(snippet)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read as _,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use flate2::read::GzDecoder;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
    };

    use super::*;
    use crate::diagnostics::{DiagnosticsMetadata, DiagnosticsSection};

    struct EnvVarGuard(&'static str);

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            // tests hold global_state_test_lock while mutating process environment
            unsafe { std::env::set_var(key, value) };

            Self(key)
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // tests hold global_state_test_lock while mutating process environment
            unsafe { std::env::remove_var(self.0) };
        }
    }

    struct TestResponse {
        status: StatusCode,
        body: String,
    }

    fn report() -> DiagnosticsUploadReport {
        DiagnosticsUploadReport {
            schema_version: 1,
            generated_at: "2026-07-06T20:00:00Z".to_string(),
            metadata: DiagnosticsMetadata {
                platform: "ios".to_string(),
                app_version: "1.0".to_string(),
                build_number: "2".to_string(),
                os_version: "iOS 20".to_string(),
                device_model: "iPhone".to_string(),
                rust_git_hash: "abc".to_string(),
                rust_git_branch: "main".to_string(),
                rust_build_profile: "debug".to_string(),
            },
            user_description: Some("description".to_string()),
            sections: vec![DiagnosticsSection {
                title: "Rust logs".to_string(),
                body: "amount=5000 sats".to_string(),
            }],
        }
    }

    async fn upload_server(responses: Vec<TestResponse>) -> (String, Arc<AtomicUsize>) {
        crate::test_support::ensure_tokio_runtime();
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test server binds");
        let addr = listener.local_addr().expect("test server has local addr");
        let request_count = Arc::new(AtomicUsize::new(0));
        let server_request_count = request_count.clone();

        tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("test server accepts request");
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).await.expect("test server reads request");
                server_request_count.fetch_add(1, Ordering::Relaxed);

                let status = response.status;
                let reason = status.canonical_reason().unwrap_or("status");
                let body = response.body;
                let http_response = format!(
                    "HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    status.as_u16(),
                    body.len(),
                );

                socket
                    .write_all(http_response.as_bytes())
                    .await
                    .expect("test server writes response");
            }
        });

        (format!("http://{addr}/reports"), request_count)
    }

    async fn read_request_head(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();

        loop {
            let mut buffer = [0; 1024];
            let bytes_read = socket.read(&mut buffer).await.expect("test server reads request");
            assert_ne!(bytes_read, 0, "request ended before its headers");
            request.extend_from_slice(&buffer[..bytes_read]);
            assert!(request.len() <= MAX_SUCCESS_RESPONSE_BYTES, "request headers are too large");

            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                return request;
            }
        }
    }

    async fn redirect_server(
        location: &str,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        crate::test_support::ensure_tokio_runtime();
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test server binds");
        let addr = listener.local_addr().expect("test server has local addr");
        let request_count = Arc::new(AtomicUsize::new(0));
        let server_request_count = request_count.clone();
        let location = location.to_string();

        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.expect("test server accepts request");
                let request = read_request_head(&mut socket).await;
                server_request_count.fetch_add(1, Ordering::Relaxed);

                assert!(request.starts_with(b"POST /reports HTTP/1.1\r\n"));

                let body = r#"{"error":"redirect"}"#;
                let http_response = format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nlocation: {location}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len(),
                );

                socket
                    .write_all(http_response.as_bytes())
                    .await
                    .expect("test server writes response");
            }
        });

        (format!("http://{addr}/reports"), request_count, server)
    }

    #[test]
    fn gzipped_json_contains_report_and_description() {
        let report = report();

        let gzipped = gzipped_json(&report).unwrap();
        let mut decoder = GzDecoder::new(gzipped.as_slice());
        let mut json = String::new();
        decoder.read_to_string(&mut json).unwrap();

        assert!(json.contains("\"user_description\":\"description\""));
        assert!(json.contains("amount=5000 sats"));
    }

    #[test]
    fn production_recipient_is_valid() {
        assert_eq!(
            production_recipient(PRODUCTION_ENCRYPTION_KEY).unwrap().to_string(),
            PRODUCTION_ENCRYPTION_KEY.recipient
        );
    }

    #[test]
    fn report_is_gzipped_before_age_encryption() {
        let report = report();
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public();

        let encrypted = encrypted_gzipped_json(&report, &recipient).unwrap();
        let second_encrypted = encrypted_gzipped_json(&report, &recipient).unwrap();

        assert!(encrypted.starts_with(b"age-encryption.org/v1\n"));
        assert_ne!(encrypted, second_encrypted);
        assert!(!encrypted.windows(16).any(|window| window == b"amount=5000 sats"));
        assert!(!encrypted.starts_with(&[0x1f, 0x8b]));

        let compressed = age::decrypt(&identity, &encrypted).unwrap();
        assert!(compressed.starts_with(&[0x1f, 0x8b]));

        let mut decoder = GzDecoder::new(compressed.as_slice());
        let mut json = String::new();
        decoder.read_to_string(&mut json).unwrap();

        assert!(json.contains("\"user_description\":\"description\""));
        assert!(json.contains("amount=5000 sats"));
    }

    #[test]
    fn collector_responses_are_not_retried_without_idempotency_support() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            let error = UploadError::Status { status, body: String::new() };

            assert!(!upload_error_is_retryable(&error));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connection_establishment_failures_are_retryable() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let addr = {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("test server binds");

            listener.local_addr().expect("test server has local addr")
        };
        let client = cove_http::new_client_without_redirects().unwrap();

        let error = client.post(format!("http://{addr}/reports")).send().await.unwrap_err();

        assert!(error.is_connect());
        assert!(upload_error_is_retryable(&UploadError::Request(error)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timeout_after_connection_is_not_retryable() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test server binds");
        let addr = listener.local_addr().expect("test server has local addr");
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("test server accepts request");

            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let client = cove_http::new_client_without_redirects().unwrap();

        let error = client
            .post(format!("http://{addr}/reports"))
            .timeout(Duration::from_millis(20))
            .body("report")
            .send()
            .await
            .unwrap_err();

        server.abort();

        let error = UploadError::Request(error);

        assert!(matches!(&error, UploadError::Request(error) if error.is_timeout()));
        assert!(!upload_error_is_retryable(&error));
        assert!(error.user_message().contains("may have reached"));
    }

    #[test]
    fn status_user_message_does_not_include_collector_body() {
        let error = UploadError::Status {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: "{\"error\":\"internal collector error\"}".to_string(),
        };
        let message = error.user_message();

        assert!(message.contains("500 Internal Server Error"));
        assert!(!message.contains("internal collector error"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_does_not_retry_server_error_after_collector_received_request() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, request_count) = upload_server(vec![TestResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: r#"{"error":"temporary"}"#.to_string(),
        }])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        assert!(matches!(error, UploadError::Status { status, .. } if status.is_server_error()));
        assert!(error.user_message().contains("may have been received"));
        assert_eq!(request_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_does_not_follow_or_retry_post_redirect() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, request_count, server) =
            redirect_server("http://127.0.0.1:0/reports").await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        server.abort();

        assert!(matches!(
            error,
            UploadError::Status { status: StatusCode::TEMPORARY_REDIRECT, .. }
        ));
        assert_eq!(request_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_rejects_bad_success_json_without_retry() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, request_count) = upload_server(vec![TestResponse {
            status: StatusCode::OK,
            body: "not json".to_string(),
        }])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        assert!(matches!(error, UploadError::DecodeResponse(_)));
        assert!(error.user_message().contains("may have been received"));
        assert_eq!(request_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_rejects_blank_report_id() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, _request_count) = upload_server(vec![TestResponse {
            status: StatusCode::OK,
            body: r#"{"id":"   "}"#.to_string(),
        }])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        assert!(matches!(error, UploadError::InvalidResponse(_)));
        assert!(error.user_message().contains("may have been received"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_classifies_payload_too_large_without_retry() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, request_count) = upload_server(vec![TestResponse {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            body: r#"{"error":"too large"}"#.to_string(),
        }])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        assert!(matches!(error, UploadError::Status { status: StatusCode::PAYLOAD_TOO_LARGE, .. }));
        assert!(error.user_message().contains("too large to submit"));
        assert_eq!(request_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_rejects_oversized_success_response() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, _request_count) = upload_server(vec![TestResponse {
            status: StatusCode::OK,
            body: "x".repeat(MAX_SUCCESS_RESPONSE_BYTES + 1),
        }])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let error = submit_report(report()).await.unwrap_err();

        assert!(matches!(error, UploadError::ResponseTooLarge { .. }));
        assert!(error.user_message().contains("may have been received"));
    }
}

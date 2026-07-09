use std::{io::Write as _, time::Duration};

use flate2::{Compression, write::GzEncoder};
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;

use super::DiagnosticsUploadReport;

const PRODUCTION_UPLOAD_URL: &str = "https://diagnostics.covebitcoinwallet.com/reports";
const APP_TOKEN: &str = "v1.cove-mobile-2026-07";
const GZIP_CONTENT_TYPE: &str = "application/gzip";
const MAX_SUCCESS_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_STATUS_BODY_BYTES: usize = 2 * 1024;
const MAX_UPLOAD_ATTEMPTS: usize = 2;
const UPLOAD_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);
const UPLOAD_RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub(crate) enum UploadError {
    #[error("failed to create HTTP client: {0}")]
    Client(reqwest::Error),

    #[error("failed to encode report JSON: {0}")]
    EncodeJson(serde_json::Error),

    #[error("failed to gzip report JSON: {0}")]
    Gzip(std::io::Error),

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
            Self::Client(_) | Self::EncodeJson(_) | Self::Gzip(_) => {
                "Unable to prepare the diagnostics report. Please try again.".to_string()
            }
            Self::Request(error) if error.is_timeout() => {
                "The diagnostics upload timed out. Please try again.".to_string()
            }
            Self::Request(_) => {
                "Unable to reach the diagnostics collector. Check your connection and try again."
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
                format!("The diagnostics collector returned {status}. Please try again.")
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

pub(crate) async fn submit_report(report: &DiagnosticsUploadReport) -> Result<String, UploadError> {
    let client = cove_http::new_client().map_err(UploadError::Client)?;
    let body = gzipped_json(report)?;

    for attempt in 1..=MAX_UPLOAD_ATTEMPTS {
        match submit_report_once(&client, &body).await {
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

async fn submit_report_once(client: &reqwest::Client, body: &[u8]) -> Result<String, UploadError> {
    let mut response = client
        .post(upload_url())
        .timeout(UPLOAD_ATTEMPT_TIMEOUT)
        .bearer_auth(APP_TOKEN)
        .header(CONTENT_TYPE, GZIP_CONTENT_TYPE)
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
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json).map_err(UploadError::Gzip)?;
    encoder.finish().map_err(UploadError::Gzip)
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
        UploadError::Request(error) => error.is_timeout() || error.is_connect(),
        UploadError::Status { status, .. } => status_is_retryable(*status),
        UploadError::Client(_)
        | UploadError::EncodeJson(_)
        | UploadError::Gzip(_)
        | UploadError::DecodeResponse(_)
        | UploadError::InvalidResponse(_)
        | UploadError::ResponseTooLarge { .. } => false,
    }
}

fn status_is_retryable(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
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
        net::TcpListener,
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
            },
            user_description: Some("description".to_string()),
            sections: vec![DiagnosticsSection {
                title: "Rust logs".to_string(),
                body: "amount=5000 sats".to_string(),
            }],
        }
    }

    async fn upload_server(responses: Vec<TestResponse>) -> (String, Arc<AtomicUsize>) {
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
    fn status_retry_classification_only_retries_transient_responses() {
        assert!(status_is_retryable(StatusCode::REQUEST_TIMEOUT));
        assert!(status_is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(status_is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(status_is_retryable(StatusCode::BAD_GATEWAY));
        assert!(!status_is_retryable(StatusCode::BAD_REQUEST));
        assert!(!status_is_retryable(StatusCode::UNAUTHORIZED));
        assert!(!status_is_retryable(StatusCode::PAYLOAD_TOO_LARGE));
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
    async fn submit_retries_transient_status_once() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let (upload_url, request_count) = upload_server(vec![
            TestResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: r#"{"error":"temporary"}"#.to_string(),
            },
            TestResponse { status: StatusCode::OK, body: r#"{"id":"report_retry"}"#.to_string() },
        ])
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);

        let report_id = submit_report(&report()).await.unwrap();

        assert_eq!(report_id, "report_retry");
        assert_eq!(request_count.load(Ordering::Relaxed), 2);
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

        let error = submit_report(&report()).await.unwrap_err();

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

        let error = submit_report(&report()).await.unwrap_err();

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

        let error = submit_report(&report()).await.unwrap_err();

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

        let error = submit_report(&report()).await.unwrap_err();

        assert!(matches!(error, UploadError::ResponseTooLarge { .. }));
        assert!(error.user_message().contains("may have been received"));
    }
}

use std::{io::Write as _, time::Duration};

use flate2::{Compression, write::GzEncoder};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use super::DiagnosticsUploadReport;

const PRODUCTION_UPLOAD_URL: &str = "https://diagnostics.covebitcoinwallet.com/reports";
const APP_TOKEN: &str = "v1.cove-mobile-2026-07";
const GZIP_CONTENT_TYPE: &str = "application/gzip";
const MAX_SUCCESS_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_STATUS_BODY_BYTES: usize = 2 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum UploadError {
    #[error("failed to create HTTP client: {0}")]
    Client(reqwest::Error),

    #[error("failed to encode report JSON: {0}")]
    Json(serde_json::Error),

    #[error("failed to gzip report JSON: {0}")]
    Gzip(std::io::Error),

    #[error("collector request failed: {0}")]
    Request(reqwest::Error),

    #[error("collector response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },

    #[error("collector returned status {status}: {body}")]
    Status { status: reqwest::StatusCode, body: String },
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    id: String,
}

pub(crate) async fn submit_report(report: &DiagnosticsUploadReport) -> Result<String, UploadError> {
    let client = cove_http::new_client().map_err(UploadError::Client)?;
    let body = gzipped_json(report)?;
    let mut response = client
        .post(upload_url())
        .timeout(Duration::from_secs(60))
        .bearer_auth(APP_TOKEN)
        .header(CONTENT_TYPE, GZIP_CONTENT_TYPE)
        .body(body)
        .send()
        .await
        .map_err(UploadError::Request)?;
    let status = response.status();

    if !status.is_success() {
        let body = read_status_body_snippet(&mut response).await?;

        return Err(UploadError::Status { status, body });
    }

    let bytes = read_success_body(&mut response).await?;
    let response: UploadResponse = serde_json::from_slice(&bytes).map_err(UploadError::Json)?;
    Ok(response.id)
}

pub(crate) fn gzipped_json(report: &DiagnosticsUploadReport) -> Result<Vec<u8>, UploadError> {
    let json = serde_json::to_vec(report).map_err(UploadError::Json)?;
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
    use std::io::Read as _;

    use flate2::read::GzDecoder;

    use super::*;
    use crate::diagnostics::{DiagnosticsMetadata, DiagnosticsSection};

    #[test]
    fn gzipped_json_contains_report_and_description() {
        let report = DiagnosticsUploadReport {
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
        };

        let gzipped = gzipped_json(&report).unwrap();
        let mut decoder = GzDecoder::new(gzipped.as_slice());
        let mut json = String::new();
        decoder.read_to_string(&mut json).unwrap();

        assert!(json.contains("\"user_description\":\"description\""));
        assert!(json.contains("amount=5000 sats"));
    }
}

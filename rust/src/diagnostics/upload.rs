use std::{io::Write as _, time::Duration};

use flate2::{Compression, write::GzEncoder};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use super::DiagnosticsUploadReport;

const PRODUCTION_UPLOAD_URL: &str = "https://diagnostics.covebitcoinwallet.com/reports";
const APP_TOKEN: &str = "v1.cove-mobile-2026-07";
const GZIP_CONTENT_TYPE: &str = "application/gzip";

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
    let response = client
        .post(upload_url())
        .timeout(Duration::from_secs(60))
        .bearer_auth(APP_TOKEN)
        .header(CONTENT_TYPE, GZIP_CONTENT_TYPE)
        .body(body)
        .send()
        .await
        .map_err(UploadError::Request)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(UploadError::Request)?;

    if !status.is_success() {
        return Err(UploadError::Status {
            status,
            body: String::from_utf8_lossy(&bytes).to_string(),
        });
    }

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

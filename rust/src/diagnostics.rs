pub mod redact;
pub mod upload;

use std::{fmt, sync::Arc};

use cove_util::result_ext::ResultExt as _;
use serde::Serialize;

use crate::database::{Database, diagnostics_reports::DiagnosticsReportRecord};

const DIAGNOSTICS_REPORT_SIZE_UNIT_BYTES: u64 = 1_000;
const DIAGNOSTICS_REPORT_SIZE_UNIT_SCALE: f64 = DIAGNOSTICS_REPORT_SIZE_UNIT_BYTES as f64;
const DIAGNOSTICS_REPORT_SIZE_FRACTIONAL_LIMIT: f64 = 10.0;
const DIAGNOSTICS_REPORT_SIZE_UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];

#[derive(Debug, Clone, uniffi::Record)]
pub struct DiagnosticsPlatformInfo {
    pub platform: String,
    pub build_number: String,
    pub os_version: String,
    pub device_model: String,
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi(flat_error)]
pub enum DiagnosticsError {
    #[error("Failed to build diagnostics report: {0}")]
    Build(String),

    #[error("Failed to clear diagnostics logs: {0}")]
    ClearLogs(String),

    #[error("Failed to submit diagnostics report: {0}")]
    Submit(String),
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DiagnosticsSubmission {
    pub report_id: String,
    pub history_saved: bool,
    pub warning: Option<String>,
}

#[derive(uniffi::Object)]
pub struct DiagnosticsReport {
    generated_at: String,
    metadata: DiagnosticsMetadata,
    sections: Vec<DiagnosticsSection>,
    preview_text: String,
    redactor: redact::Redactor,
}

impl fmt::Debug for DiagnosticsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiagnosticsReport")
            .field("generated_at", &self.generated_at)
            .field("metadata", &self.metadata)
            .field("sections_count", &self.sections.len())
            .field("preview_text", &format_args!("<redacted len={}>", self.preview_text.len()))
            .field("redactor", &self.redactor)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiagnosticsUploadReport {
    schema_version: u16,
    generated_at: String,
    metadata: DiagnosticsMetadata,
    user_description: Option<String>,
    sections: Vec<DiagnosticsSection>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiagnosticsMetadata {
    platform: String,
    app_version: String,
    build_number: String,
    os_version: String,
    device_model: String,
    rust_git_hash: String,
    rust_git_branch: String,
    rust_build_profile: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiagnosticsSection {
    title: String,
    body: String,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn build_diagnostics_report(
    platform: DiagnosticsPlatformInfo,
    platform_logs: String,
) -> Result<Arc<DiagnosticsReport>, DiagnosticsError> {
    cove_tokio::unblock::run_blocking(move || {
        DiagnosticsReport::build(platform, platform_logs).map(Arc::new)
    })
    .await
}

#[uniffi::export]
pub fn clear_diagnostics_logs() -> Result<(), DiagnosticsError> {
    cove_common::logging::capture::clear().map_err_str(DiagnosticsError::ClearLogs)
}

#[uniffi::export(async_runtime = "tokio")]
impl DiagnosticsReport {
    pub fn preview_text(&self) -> String {
        self.preview_text.clone()
    }

    pub fn preview_text_for_description(&self, description: Option<String>) -> String {
        self.preview_text_with_description(description)
    }

    pub fn size_bytes(&self) -> u64 {
        self.preview_text.len() as u64
    }

    pub fn size_bytes_for_description(&self, description: Option<String>) -> u64 {
        self.preview_text_for_description(description).len() as u64
    }

    pub fn formatted_size(&self) -> String {
        format_diagnostics_report_size(self.size_bytes())
    }

    pub fn formatted_size_for_description(&self, description: Option<String>) -> String {
        format_diagnostics_report_size(self.size_bytes_for_description(description))
    }

    pub async fn submit(
        &self,
        description: Option<String>,
    ) -> Result<DiagnosticsSubmission, DiagnosticsError> {
        let description = self.redacted_user_description(description);
        let report = self.upload_report(description.clone());

        let report_id = upload::submit_report(&report).await.map_err(|error| {
            tracing::warn!("Failed to submit diagnostics report: {}", error.log_message());
            DiagnosticsError::Submit(error.user_message())
        })?;

        let record = DiagnosticsReportRecord::now(report_id.clone(), description);
        let save_result = cove_tokio::unblock::run_blocking(move || {
            Database::global().diagnostics_reports.add(record)
        })
        .await;
        if let Err(error) = save_result {
            tracing::warn!("Failed to save diagnostics report history: {error}");
            return Ok(DiagnosticsSubmission {
                report_id,
                history_saved: false,
                warning: Some(
                    "Diagnostics were uploaded, but Cove could not save the report ID on this device. Copy the report ID before closing this screen."
                        .to_string(),
                ),
            });
        }

        Ok(DiagnosticsSubmission { report_id, history_saved: true, warning: None })
    }
}

fn user_description_for_upload(description: Option<String>) -> Option<String> {
    description
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty())
}

fn format_diagnostics_report_size(size_bytes: u64) -> String {
    if size_bytes < DIAGNOSTICS_REPORT_SIZE_UNIT_BYTES {
        let unit = if size_bytes == 1 { "byte" } else { "bytes" };

        return format!("{size_bytes} {unit}");
    }

    let mut size = size_bytes as f64;
    let mut unit = DIAGNOSTICS_REPORT_SIZE_UNITS[0];
    for next_unit in DIAGNOSTICS_REPORT_SIZE_UNITS {
        size /= DIAGNOSTICS_REPORT_SIZE_UNIT_SCALE;
        unit = next_unit;
        if size < DIAGNOSTICS_REPORT_SIZE_UNIT_SCALE {
            break;
        }
    }

    format!("{} {unit}", format_diagnostics_report_size_value(size))
}

fn format_diagnostics_report_size_value(size: f64) -> String {
    let formatted = if size < DIAGNOSTICS_REPORT_SIZE_FRACTIONAL_LIMIT {
        format!("{size:.1}")
    } else {
        format!("{size:.0}")
    };

    formatted.strip_suffix(".0").unwrap_or(&formatted).to_string()
}

impl DiagnosticsReport {
    fn build(
        platform: DiagnosticsPlatformInfo,
        platform_logs: String,
    ) -> Result<Self, DiagnosticsError> {
        let startup = crate::bootstrap::startup_diagnostic_text_report();
        let rust_logs = cove_common::logging::capture::snapshot_text();

        Self::build_with_sources(platform, platform_logs, startup, rust_logs)
    }

    fn build_with_sources(
        platform: DiagnosticsPlatformInfo,
        platform_logs: String,
        startup: String,
        rust_logs: String,
    ) -> Result<Self, DiagnosticsError> {
        platform.validate()?;

        let generated_at = timestamp();
        let metadata = DiagnosticsMetadata {
            platform: platform.platform,
            app_version: crate::build::version(),
            build_number: platform.build_number,
            os_version: platform.os_version,
            device_model: platform.device_model,
            rust_git_hash: crate::build::git_short_hash(),
            rust_git_branch: crate::build::git_branch(),
            rust_build_profile: crate::build::profile(),
        };
        let mut redactor = redact::Redactor::with_default_paths();
        let sections = raw_sections_from(startup, platform_logs, rust_logs)
            .into_iter()
            .map(|section| section.redacted(&mut redactor))
            .collect::<Vec<_>>();
        let preview_text = render_preview(&generated_at, &metadata, &sections);

        Ok(Self { generated_at, metadata, sections, preview_text, redactor })
    }

    fn upload_report(&self, user_description: Option<String>) -> DiagnosticsUploadReport {
        DiagnosticsUploadReport {
            schema_version: 1,
            generated_at: self.generated_at.clone(),
            metadata: self.metadata.clone(),
            user_description,
            sections: self.sections.clone(),
        }
    }

    fn redacted_user_description(&self, description: Option<String>) -> Option<String> {
        let description = user_description_for_upload(description)?;
        let mut redactor = self.redactor.clone();

        Some(redactor.redact(&description))
    }

    fn preview_text_with_description(&self, description: Option<String>) -> String {
        let Some(description) = self.redacted_user_description(description) else {
            return self.preview_text.clone();
        };

        render_preview_with_user_description(&self.preview_text, &description)
    }
}

impl DiagnosticsSection {
    fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { title: title.into(), body: body.into() }
    }

    fn redacted(self, redactor: &mut redact::Redactor) -> Self {
        Self { title: self.title, body: redactor.redact(&self.body) }
    }
}

impl DiagnosticsPlatformInfo {
    fn validate(&self) -> Result<(), DiagnosticsError> {
        if self.platform.trim().is_empty() {
            return Err(DiagnosticsError::Build("platform is required".to_string()));
        }

        if self.build_number.trim().is_empty() {
            return Err(DiagnosticsError::Build("build number is required".to_string()));
        }

        if self.os_version.trim().is_empty() {
            return Err(DiagnosticsError::Build("OS version is required".to_string()));
        }

        if self.device_model.trim().is_empty() {
            return Err(DiagnosticsError::Build("device model is required".to_string()));
        }

        Ok(())
    }
}

fn raw_sections_from(
    startup: String,
    platform_logs: String,
    rust_logs: String,
) -> Vec<DiagnosticsSection> {
    let platform_logs = if platform_logs.trim().is_empty() {
        "no platform logs provided\n".to_string()
    } else {
        platform_logs
    };

    vec![
        DiagnosticsSection::new(
            "Privacy notice",
            "Cove redacts bitcoin addresses, extended public/private keys, WIF private keys, English BIP39 seed phrases, transaction IDs, and known local app data paths. Amounts remain visible. Review the report before submitting.",
        ),
        DiagnosticsSection::new("Startup diagnostics", startup),
        DiagnosticsSection::new("Platform logs", platform_logs),
        DiagnosticsSection::new("Rust logs", rust_logs),
    ]
}

fn render_preview(
    generated_at: &str,
    metadata: &DiagnosticsMetadata,
    sections: &[DiagnosticsSection],
) -> String {
    let mut text = String::new();
    text.push_str("Cove diagnostics report\n");
    text.push_str(&format!("Generated: {generated_at}\n\n"));
    text.push_str("App and build\n");
    text.push_str(&format!("Platform: {}\n", metadata.platform));
    text.push_str(&format!("App version: {}\n", metadata.app_version));
    text.push_str(&format!("Build number: {}\n", metadata.build_number));
    text.push_str(&format!("OS version: {}\n", metadata.os_version));
    text.push_str(&format!("Device model: {}\n", metadata.device_model));
    text.push_str(&format!("Rust git hash: {}\n", metadata.rust_git_hash));
    text.push_str(&format!("Rust git branch: {}\n", metadata.rust_git_branch));
    text.push_str(&format!("Rust build profile: {}\n", metadata.rust_build_profile));

    for section in sections {
        text.push_str("\n## ");
        text.push_str(&section.title);
        text.push('\n');
        text.push_str(&section.body);
        if !section.body.ends_with('\n') {
            text.push('\n');
        }
    }

    text
}

fn render_preview_with_user_description(preview_text: &str, user_description: &str) -> String {
    let mut text = preview_text.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }

    text.push_str("\n## User description\n");
    text.push_str(user_description);
    if !user_description.ends_with('\n') {
        text.push('\n');
    }

    text
}

fn timestamp() -> String {
    jiff::Timestamp::now().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    static DIAGNOSTICS_UPLOAD_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard(&'static str);

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            // tests hold DIAGNOSTICS_UPLOAD_ENV_LOCK while mutating process environment
            unsafe { std::env::set_var(key, value) };

            Self(key)
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // tests hold DIAGNOSTICS_UPLOAD_ENV_LOCK while mutating process environment
            unsafe { std::env::remove_var(self.0) };
        }
    }

    fn platform_info() -> DiagnosticsPlatformInfo {
        DiagnosticsPlatformInfo {
            platform: "ios".to_string(),
            build_number: "123".to_string(),
            os_version: "iOS 20.0".to_string(),
            device_model: "iPhone17,1".to_string(),
        }
    }

    async fn upload_server_url(report_id: &'static str) -> String {
        upload_server_response("200 OK", format!(r#"{{"id":"{report_id}"}}"#)).await
    }

    async fn upload_server_response(status: &'static str, body: String) -> String {
        use tokio::{
            io::{AsyncReadExt as _, AsyncWriteExt as _},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test server binds");
        let addr = listener.local_addr().expect("test server has local addr");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("test server accepts request");
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await.expect("test server reads request");

            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len(),
            );

            socket.write_all(response.as_bytes()).await.expect("test server writes response");
        });

        format!("http://{addr}/reports")
    }

    #[test]
    fn assembly_includes_sections_and_redacts_sensitive_values() {
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            "platform saw bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq amount=5000 sats".to_string(),
            "startup path /tmp/cove-test/.data/wallets".to_string(),
            "rust log txid 4d3c2b1a4d3c2b1a4d3c2b1a4d3c2b1a4d3c2b1a4d3c2b1a4d3c2b1a4d3c2b1a"
                .to_string(),
        )
        .unwrap();
        let preview = report.preview_text();

        assert!(preview.contains("App and build"));
        assert!(preview.contains("Startup diagnostics"));
        assert!(preview.contains("Platform logs"));
        assert!(preview.contains("Rust logs"));
        assert!(preview.contains("<redacted-bitcoin-address-"));
        assert!(preview.contains("amount=5000 sats"));
        assert!(!preview.contains("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
        assert_eq!(report.size_bytes(), preview.len() as u64);
    }

    #[test]
    fn upload_report_keeps_description_out_of_metadata() {
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let upload =
            report.upload_report(report.redacted_user_description(Some("description".to_string())));
        let json = serde_json::to_string(&upload).unwrap();

        assert!(json.contains("\"user_description\":\"description\""));
        assert!(!json.contains("\"description\":\"description\""));
    }

    #[test]
    fn upload_includes_build_metadata_shown_in_preview() {
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let upload = report.upload_report(None);
        let json = serde_json::to_value(upload).unwrap();

        assert_eq!(json["metadata"]["rust_git_branch"], report.metadata.rust_git_branch);
        assert_eq!(json["metadata"]["rust_build_profile"], report.metadata.rust_build_profile);
        assert!(
            report
                .preview_text
                .contains(&format!("Rust git branch: {}", report.metadata.rust_git_branch))
        );
        assert!(
            report
                .preview_text
                .contains(&format!("Rust build profile: {}", report.metadata.rust_build_profile))
        );
    }

    #[test]
    fn user_description_is_redacted_for_preview_and_upload() {
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            "platform saw bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".to_string(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let wif = "5JYkZjmN7PVMjJUfJWfRFwtuXTGB439XV6faajeHPAM9Z2PT2R3";
        let description = format!(
            "  same address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq seed {mnemonic} key {wif} under /tmp/cove-test/.data  "
        );

        let preview = report.preview_text_for_description(Some(description.clone()));
        let upload =
            report.upload_report(report.redacted_user_description(Some(description.clone())));
        let json = serde_json::to_string(&upload).unwrap();

        assert!(preview.contains("## User description"));
        assert!(preview.contains("<redacted-bitcoin-address-1>"));
        assert!(preview.contains("<redacted-seed-phrase-1>"));
        assert!(preview.contains("<redacted-wif-private-key-1>"));
        assert!(json.contains("<redacted-bitcoin-address-1>"));
        assert!(json.contains("<redacted-seed-phrase-1>"));
        assert!(json.contains("<redacted-wif-private-key-1>"));
        assert!(!preview.contains("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
        assert!(!preview.contains(mnemonic));
        assert!(!preview.contains(wif));
        assert!(!json.contains("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
        assert!(!json.contains(mnemonic));
        assert!(!json.contains(wif));
    }

    #[test]
    fn empty_user_description_is_omitted() {
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();

        let preview = report.preview_text_for_description(Some(" \n\t ".to_string()));
        let upload =
            report.upload_report(report.redacted_user_description(Some(" \n\t ".to_string())));

        assert_eq!(preview, report.preview_text());
        assert!(upload.user_description.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_persists_successful_report_history() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _env_guard = DIAGNOSTICS_UPLOAD_ENV_LOCK.lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::delete_database();
        Database::try_reinit().expect("database reinitializes");

        let upload_url = upload_server_url("report_123").await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();

        let submission = report.submit(Some("description".to_string())).await.unwrap();

        let records = Database::global().diagnostics_reports.all().unwrap();
        assert_eq!(submission.report_id, "report_123");
        assert!(submission.history_saved);
        assert!(submission.warning.is_none());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].report_id, "report_123");
        assert_eq!(records[0].description, Some("description".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_persists_redacted_history_description() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _env_guard = DIAGNOSTICS_UPLOAD_ENV_LOCK.lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::delete_database();
        Database::try_reinit().expect("database reinitializes");

        let upload_url = upload_server_url("report_secret").await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            "platform saw bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".to_string(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let description =
            format!("same address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq seed {mnemonic}");

        let submission = report.submit(Some(description)).await.unwrap();

        let records = Database::global().diagnostics_reports.all().unwrap();
        let history_description = records[0].description.as_ref().expect("description saved");
        assert_eq!(submission.report_id, "report_secret");
        assert!(history_description.contains("<redacted-bitcoin-address-1>"));
        assert!(history_description.contains("<redacted-seed-phrase-1>"));
        assert!(!history_description.contains("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
        assert!(!history_description.contains(mnemonic));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_upload_does_not_persist_history_and_returns_safe_message() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _env_guard = DIAGNOSTICS_UPLOAD_ENV_LOCK.lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::delete_database();
        Database::try_reinit().expect("database reinitializes");

        let upload_url = upload_server_response(
            "413 Payload Too Large",
            r#"{"error":"collector internal detail"}"#.to_string(),
        )
        .await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();

        let error = report.submit(Some("description".to_string())).await.unwrap_err();

        let records = Database::global().diagnostics_reports.all().unwrap();
        let DiagnosticsError::Submit(message) = error else {
            panic!("expected submit error");
        };
        assert!(records.is_empty());
        assert!(message.contains("too large to submit"));
        assert!(!message.contains("collector internal detail"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_success_with_history_save_failure_returns_warning() {
        let _global_guard = crate::test_support::global_state_test_lock().lock().await;
        let _env_guard = DIAGNOSTICS_UPLOAD_ENV_LOCK.lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::delete_database();
        Database::try_reinit().expect("database reinitializes");
        crate::database::diagnostics_reports::test_support::write_invalid_history(
            &Database::global().diagnostics_reports,
        );

        let upload_url = upload_server_url("report_history_failed").await;
        let _upload_url = EnvVarGuard::set("COVE_DIAGNOSTICS_URL", &upload_url);
        let report = DiagnosticsReport::build_with_sources(
            platform_info(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();

        let submission = report.submit(Some("description".to_string())).await.unwrap();

        assert_eq!(submission.report_id, "report_history_failed");
        assert!(!submission.history_saved);
        assert!(
            submission
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("could not save the report ID"))
        );
        assert!(Database::global().diagnostics_reports.all().is_err());
    }

    #[test]
    fn report_size_format_uses_bytes_below_kilobyte() {
        assert_eq!(format_diagnostics_report_size(0), "0 bytes");
        assert_eq!(format_diagnostics_report_size(1), "1 byte");
        assert_eq!(format_diagnostics_report_size(999), "999 bytes");
    }

    #[test]
    fn report_size_format_uses_kilobytes_at_kilobyte() {
        assert_eq!(format_diagnostics_report_size(1_000), "1 KB");
        assert_eq!(format_diagnostics_report_size(1_500), "1.5 KB");
        assert_eq!(format_diagnostics_report_size(12_000), "12 KB");
    }

    #[test]
    fn report_size_format_uses_larger_units() {
        assert_eq!(format_diagnostics_report_size(1_000_000), "1 MB");
        assert_eq!(format_diagnostics_report_size(1_500_000), "1.5 MB");
        assert_eq!(format_diagnostics_report_size(1_000_000_000), "1 GB");
    }
}

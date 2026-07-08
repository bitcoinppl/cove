pub mod redact;
pub mod upload;

use std::sync::Arc;

use serde::Serialize;

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

#[derive(Debug, uniffi::Object)]
pub struct DiagnosticsReport {
    generated_at: String,
    metadata: DiagnosticsMetadata,
    sections: Vec<DiagnosticsSection>,
    preview_text: String,
    redactor: redact::Redactor,
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
    DiagnosticsReport::build(platform, platform_logs).map(Arc::new)
}

#[uniffi::export]
pub fn clear_diagnostics_logs() -> Result<(), DiagnosticsError> {
    cove_common::logging::capture::clear()
        .map_err(|error| DiagnosticsError::ClearLogs(error.to_string()))
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

    pub async fn submit(&self, description: Option<String>) -> Result<String, DiagnosticsError> {
        let description = self.redacted_user_description(description);
        let report = self.upload_report(description);

        upload::submit_report(&report)
            .await
            .map_err(|error| DiagnosticsError::Submit(error.to_string()))
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
            "Cove redacts bitcoin addresses, extended public/private keys, WIF private keys, BIP39 seed phrases, transaction IDs, and known local app data paths. Amounts remain visible. Review the report before submitting.",
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
    text.push_str(&format!("Rust git branch: {}\n", crate::build::git_branch()));
    text.push_str(&format!("Rust build profile: {}\n", crate::build::profile()));

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

    fn platform_info() -> DiagnosticsPlatformInfo {
        DiagnosticsPlatformInfo {
            platform: "ios".to_string(),
            build_number: "123".to_string(),
            os_version: "iOS 20.0".to_string(),
            device_model: "iPhone17,1".to_string(),
        }
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

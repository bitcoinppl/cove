pub mod redact;
pub mod upload;

use std::sync::Arc;

use serde::Serialize;

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

    pub fn size_bytes(&self) -> u64 {
        self.preview_text.len() as u64
    }

    pub async fn submit(&self, description: Option<String>) -> Result<String, DiagnosticsError> {
        let description = user_description_for_upload(description);
        let report = self.upload_report(description);

        upload::submit_report(&report)
            .await
            .map_err(|error| DiagnosticsError::Submit(error.to_string()))
    }
}

fn user_description_for_upload(description: Option<String>) -> Option<String> {
    description.filter(|description| !description.trim().is_empty())
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

        Ok(Self { generated_at, metadata, sections, preview_text })
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
            "Cove redacts bitcoin addresses, extended public/private keys, transaction IDs, and known local app data paths. Amounts remain visible. Review the report before submitting.",
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
        let upload = report.upload_report(Some("description".to_string()));
        let json = serde_json::to_string(&upload).unwrap();

        assert!(json.contains("\"user_description\":\"description\""));
        assert!(!json.contains("\"description\":\"description\""));
    }

    #[test]
    fn user_description_is_not_automatically_redacted() {
        let description =
            "saw bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq under /tmp/cove-test/.data";

        let upload_description = user_description_for_upload(Some(description.to_string()))
            .expect("non-empty description");

        assert_eq!(upload_description, description);
    }
}

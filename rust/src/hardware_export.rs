use std::sync::Arc;

use derive_more::derive::{AsRef, Deref, From, Into};

#[derive(Debug, Clone, PartialEq, Eq, Deref, From, Into, AsRef, uniffi::Object)]
pub struct HardwareExport(Arc<pubport::Format>);

impl HardwareExport {
    pub fn new(format: pubport::Format) -> Self {
        Self(Arc::new(format))
    }

    pub fn format(&self) -> &pubport::Format {
        self.0.as_ref()
    }

    pub fn into_format(self) -> pubport::Format {
        Arc::unwrap_or_clone(self.0)
    }
}

impl From<pubport::Format> for HardwareExport {
    fn from(format: pubport::Format) -> Self {
        Self::new(format)
    }
}

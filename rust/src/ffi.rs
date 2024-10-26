use std::sync::Arc;

use derive_more::derive::{AsRef, Deref, From, Into};

#[derive(Debug, Clone, Deref, From, Into, AsRef, uniffi::Object)]
pub struct HardwareExport(Arc<pubport::Format>);

impl HardwareExport {
    pub fn new(format: pubport::Format) -> Self {
        Self(Arc::new(format))
    }
}

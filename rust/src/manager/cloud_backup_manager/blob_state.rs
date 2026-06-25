use crate::database::cloud_backup::PersistedCloudBlobState;

impl PersistedCloudBlobState {
    pub(crate) fn revision_hash(&self) -> Option<&str> {
        match self {
            Self::Uploading(state) => Some(&state.revision_hash),
            Self::UploadedPendingConfirmation(state) => Some(&state.revision_hash),
            Self::Confirmed(state) => Some(&state.revision_hash),
            Self::Failed(state) => state.revision_hash.as_deref(),
            Self::Dirty(_) => None,
        }
    }
}

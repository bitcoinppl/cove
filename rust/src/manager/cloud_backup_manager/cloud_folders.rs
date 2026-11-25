//! What each listed Cloud Backup folder holds, read once without asking for a passkey
//!
//! Onboarding's backup check, Enable, Restore and Recover Other Backups read a listing only
//! through [`CloudFolders`],
//! so whether a backup exists, and whether its files are still unavailable, is decided in one
//! place: [`ListedBackups::of`]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use cove_cspp::backup_data::{
    EncryptedMasterKeyBackup, MASTER_KEY_RECORD_ID, MasterKeyBackupVersion,
};
use cove_device::cloud_storage::{CloudBackupUploadStatus, CloudStorageClient, CloudStorageError};
use futures::stream::{self, TryStreamExt as _};
use tracing::{info, warn};

use super::timing::log_elapsed;
use super::{
    CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupExistingBackup,
    CloudBackupPasskeyHint, master_key_wrapper_revision_hash,
};

/// What one listed folder holds
pub(crate) enum CloudFolder {
    /// A master key wrapper in a format this app reads, which a passkey may open
    Readable(Box<ReadableFolder>),
    /// No master key wrapper is visible
    MasterKeyMissing,
    /// The master key wrapper's upload isn't confirmed yet
    UploadPending,
    /// The master key wrapper is in a format newer than this app reads
    NewerFormat,
    /// The master key wrapper is there but doesn't decode or names another namespace, or the
    /// provider failed to read it, with that failure
    Unreadable(Option<CloudStorageError>),
}

impl CloudFolder {
    /// Whether the folder holds a master key wrapper, in a format this app reads or not, so it
    /// is a backup before any passkey is asked for
    fn holds_wrapper(&self) -> bool {
        match self {
            Self::Readable(_) | Self::NewerFormat | Self::Unreadable(None) => true,
            Self::MasterKeyMissing | Self::UploadPending | Self::Unreadable(Some(_)) => false,
        }
    }
}

/// A listed folder that holds a master key wrapper
pub(crate) struct WrappedFolder {
    pub(crate) namespace_id: String,
    /// Where the passkey the wrapper names was created, when it names one
    pub(crate) passkey_hint: Option<CloudBackupPasskeyHint>,
}

/// One listed folder, with the namespace it was read from and the passkey hint its wrapper
/// carries
struct ListedFolder {
    namespace_id: String,
    folder: CloudFolder,
    passkey_hint: Option<CloudBackupPasskeyHint>,
}

/// A folder's master key wrapper a passkey may open
pub(crate) struct ReadableFolder {
    pub(crate) namespace_id: String,
    pub(crate) encrypted: EncryptedMasterKeyBackup,
    pub(crate) wrapper: DownloadedWrapper,
}

/// A master key wrapper exactly as the provider returned it, with the revision that names it
///
/// The revision is derived from the bytes on construction, so the two always describe the same
/// wrapper
#[derive(Clone)]
pub(crate) struct DownloadedWrapper {
    json: Vec<u8>,
    revision: String,
}

impl DownloadedWrapper {
    pub(crate) fn new(json: Vec<u8>) -> Self {
        let revision = master_key_wrapper_revision_hash(&json);
        Self { json, revision }
    }

    /// The bytes as downloaded
    pub(crate) fn json(&self) -> &[u8] {
        &self.json
    }

    /// Tells this revision of the wrapper from a later upload to the same namespace
    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }
}

/// Which master key wrappers a read downloads
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WrapperRead {
    /// Only a wrapper whose upload is confirmed; one whose upload isn't is
    /// [`CloudFolder::UploadPending`]
    Uploaded,
    /// Any wrapper this device can download, without reading its upload state
    ///
    /// Onboarding's silent check reads this way: on a new iPhone, a wrapper that synced may report
    /// its upload state as unknown, which reads as pending, and on Android a confirmed upload binds
    /// the Drive account before the user chose one. A folder the provider needs authorization
    /// for is only unreadable, since the check never asks for it and another folder's wrapper is
    /// still a backup to offer
    Visible,
}

/// What a listing shows before any passkey is asked for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListedBackups<'a> {
    /// No folder is listed
    None,
    /// A folder holds a master key wrapper, in a format this app reads or not
    Present,
    /// Every listed folder lacks a master key wrapper a passkey could open yet: it isn't
    /// visible, or its upload isn't confirmed
    ///
    /// The folders may still be syncing to this device, or be left by a setup that stopped before
    /// its upload; nothing here tells the two apart
    FilesUnavailable,
    /// No folder showed a wrapper, and the provider failed to read at least one of them, first
    /// with this failure
    Unknown(&'a CloudStorageError),
}

impl<'a> ListedBackups<'a> {
    /// The one rule for what `folders` show together
    pub(crate) fn of(folders: impl IntoIterator<Item = &'a CloudFolder>) -> Self {
        let mut listed = Self::None;
        for folder in folders {
            if folder.holds_wrapper() {
                return Self::Present;
            }

            // every folder left lacks a wrapper, so only a failed read keeps it from being files
            // still unavailable
            listed = match (listed, folder) {
                (Self::Unknown(failure), _) => Self::Unknown(failure),
                (_, CloudFolder::Unreadable(Some(failure))) => Self::Unknown(failure),
                _ => Self::FilesUnavailable,
            };
        }

        listed
    }
}

/// Every listed folder's classification, with the newest passkey hint their wrappers carry
pub(crate) struct CloudFolders {
    folders: Vec<ListedFolder>,
    passkey_hint: Option<CloudBackupPasskeyHint>,
}

impl CloudFolders {
    /// Reads each of `namespaces` once, downloading the wrappers `wrappers` names, stopping with
    /// [`CloudBackupError::Cancelled`] once `cancellation` is set
    ///
    /// With [`WrapperRead::Uploaded`], a provider that needs authorization fails the whole read,
    /// so the caller can ask for it; any other read failure classifies only its own folder
    pub(crate) async fn read(
        cloud: &CloudStorageClient,
        namespaces: &[String],
        wrappers: WrapperRead,
        cancellation: &AtomicBool,
    ) -> Result<Self, CloudBackupError> {
        let folders =
            read_each(cloud, namespaces, wrappers, cancellation, |listed| Ok(Some(listed))).await?;
        let passkey_hint = folders
            .iter()
            .filter_map(|folder| folder.passkey_hint.clone())
            .reduce(|newest, hint| if hint.is_newer_than(&newest) { hint } else { newest });

        Ok(Self { folders, passkey_hint })
    }

    /// Reads the folders of `namespaces` that hold a master key wrapper, in listing order, as
    /// [`CloudFolders::read`] does
    ///
    /// For a caller that lists backups one by one rather than asking what they show together. A
    /// folder the provider fails to read may hold a backup, so that failure fails the whole read
    /// as soon as it's known, without waiting for the folders still being read
    pub(crate) async fn read_wrapped(
        cloud: &CloudStorageClient,
        namespaces: &[String],
        wrappers: WrapperRead,
        cancellation: &AtomicBool,
    ) -> Result<Vec<WrappedFolder>, CloudBackupError> {
        read_each(cloud, namespaces, wrappers, cancellation, |listed| {
            let ListedFolder { namespace_id, folder, passkey_hint } = listed;
            match folder {
                CloudFolder::Unreadable(Some(failure)) => {
                    Err(CloudBackupError::cloud_storage_context(
                        "inspect cloud backup namespace",
                        failure,
                    ))
                }
                folder if folder.holds_wrapper() => {
                    Ok(Some(WrappedFolder { namespace_id, passkey_hint }))
                }
                _ => Ok(None),
            }
        })
        .await
    }

    pub(crate) fn listed(&self) -> ListedBackups<'_> {
        ListedBackups::of(self.folders.iter().map(|listed| &listed.folder))
    }

    /// Where the newest passkey named by any wrapper was created
    pub(crate) fn passkey_hint(&self) -> Option<&CloudBackupPasskeyHint> {
        self.passkey_hint.as_ref()
    }

    /// What a fresh Enable confirms before creating a new backup beside these folders
    ///
    /// A folder that couldn't be read still counts as a backup, so Enable keeps asking before
    /// it creates another one beside it
    pub(crate) fn existing_backup(&self) -> CloudBackupExistingBackup {
        match self.listed() {
            ListedBackups::FilesUnavailable => CloudBackupExistingBackup::FilesUnavailable,
            ListedBackups::None | ListedBackups::Present | ListedBackups::Unknown(_) => {
                CloudBackupExistingBackup::Readable { passkey_hint: self.passkey_hint.clone() }
            }
        }
    }

    pub(crate) fn into_folders(self) -> Vec<CloudFolder> {
        self.folders.into_iter().map(|listed| listed.folder).collect()
    }
}

/// Reads each of `namespaces` once, in listing order, keeping what `keep` makes of each folder
///
/// A folder `keep` fails ends the read at once, dropping the reads still running
async fn read_each<T>(
    cloud: &CloudStorageClient,
    namespaces: &[String],
    wrappers: WrapperRead,
    cancellation: &AtomicBool,
    keep: impl Fn(ListedFolder) -> Result<Option<T>, CloudBackupError>,
) -> Result<Vec<T>, CloudBackupError> {
    let keep = &keep;
    let mut seen_namespaces = HashSet::new();
    let unique_namespaces = namespaces
        .iter()
        .filter(|namespace| seen_namespaces.insert((*namespace).clone()))
        .cloned()
        .collect::<Vec<_>>();
    let mut kept =
        stream::iter(unique_namespaces.into_iter().enumerate().map(|(index, namespace_id)| {
            let cloud = cloud.clone();

            Ok(async move {
                let (folder, passkey_hint) =
                    read_folder(&cloud, namespace_id.clone(), wrappers, cancellation).await?;
                let kept = keep(ListedFolder { namespace_id, folder, passkey_hint })?;
                Ok::<_, CloudBackupError>((index, kept))
            })
        }))
        .try_buffer_unordered(CLOUD_BACKUP_IO_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    kept.sort_by_key(|(index, _)| *index);

    Ok(kept.into_iter().filter_map(|(_, kept)| kept).collect())
}

/// One folder's classification, with the passkey hint its wrapper carries
type ReadFolder = (CloudFolder, Option<CloudBackupPasskeyHint>);

async fn read_folder(
    cloud: &CloudStorageClient,
    namespace_id: String,
    wrappers: WrapperRead,
    cancellation: &AtomicBool,
) -> Result<ReadFolder, CloudBackupError> {
    if cancellation_requested(cancellation) {
        return Err(CloudBackupError::Cancelled);
    }

    if wrappers == WrapperRead::Uploaded
        && let Some(unconfirmed) = read_upload_state(cloud, &namespace_id, cancellation).await?
    {
        return Ok((unconfirmed, None));
    }

    let wrapper = log_elapsed(
        "Cloud folder wrapper read",
        cloud.download_master_key_backup(namespace_id.clone()),
    )
    .await;
    if cancellation_requested(cancellation) {
        return Err(CloudBackupError::Cancelled);
    }

    let master_json = match wrapper {
        Ok(master_json) => master_json,
        Err(CloudStorageError::NotFound(_)) => {
            info!("Cloud folder has no master key wrapper");
            return Ok((CloudFolder::MasterKeyMissing, None));
        }
        Err(error) if error.requires_account_action() && wrappers == WrapperRead::Uploaded => {
            return Err(error.into());
        }
        Err(error) => {
            warn!("Failed to download cloud backup master key: {error}");
            return Ok((CloudFolder::Unreadable(Some(error)), None));
        }
    };

    Ok(classify_wrapper(namespace_id, master_json))
}

/// The folder a wrapper whose upload isn't confirmed leaves, or `None` once it is
async fn read_upload_state(
    cloud: &CloudStorageClient,
    namespace_id: &str,
    cancellation: &AtomicBool,
) -> Result<Option<CloudFolder>, CloudBackupError> {
    let upload_state = log_elapsed(
        "Cloud folder upload-state read",
        cloud.is_backup_uploaded(namespace_id.to_string(), MASTER_KEY_RECORD_ID.to_string()),
    )
    .await;
    if cancellation_requested(cancellation) {
        return Err(CloudBackupError::Cancelled);
    }

    match upload_state {
        Ok(CloudBackupUploadStatus::Uploaded) => Ok(None),
        Ok(CloudBackupUploadStatus::Pending) => {
            info!("Cloud folder master key upload_state=pending");
            Ok(Some(CloudFolder::UploadPending))
        }
        Ok(CloudBackupUploadStatus::NotFound) => {
            info!("Cloud folder has no master key wrapper");
            Ok(Some(CloudFolder::MasterKeyMissing))
        }
        Err(error) if error.requires_account_action() => Err(error.into()),
        Err(error) => {
            warn!("Failed to read cloud folder upload state: {error}");
            Ok(Some(CloudFolder::Unreadable(Some(error))))
        }
    }
}

fn classify_wrapper(namespace_id: String, master_json: Vec<u8>) -> ReadFolder {
    let encrypted = match serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) {
        Ok(encrypted) => encrypted,
        Err(error) => {
            warn!("Failed to deserialize cloud backup master key: {error}");
            return (CloudFolder::Unreadable(None), None);
        }
    };

    // a wrapper in a newer format still names the passkey that created it
    let passkey_hint =
        CloudBackupPasskeyHint::from_master_key_wrapper(&encrypted, &namespace_id).ok().flatten();
    if !matches!(encrypted.backup_version(), Ok(MasterKeyBackupVersion::V1)) {
        return (CloudFolder::NewerFormat, passkey_hint);
    }
    if encrypted.remote_metadata.normalized_master_key(&namespace_id).is_err() {
        return (CloudFolder::Unreadable(None), passkey_hint);
    }

    let wrapper = DownloadedWrapper::new(master_json);
    let folder = ReadableFolder { namespace_id, encrypted, wrapper };
    (CloudFolder::Readable(Box::new(folder)), passkey_hint)
}

fn cancellation_requested(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_failed() -> CloudFolder {
        CloudFolder::Unreadable(Some(CloudStorageError::Offline("offline".into())))
    }

    /// A wrapper anywhere is a backup, a failed read beside only missing ones leaves it unknown,
    /// and only folders with no master key yet are files still unavailable
    #[test]
    fn listed_backups_reads_the_folders_together() {
        use CloudFolder as F;

        assert_eq!(ListedBackups::of(&[]), ListedBackups::None);
        assert_eq!(
            ListedBackups::of(&[F::MasterKeyMissing, F::UploadPending]),
            ListedBackups::FilesUnavailable
        );
        assert!(matches!(
            ListedBackups::of(&[F::MasterKeyMissing, read_failed(), F::UploadPending]),
            ListedBackups::Unknown(CloudStorageError::Offline(_))
        ));
        assert_eq!(ListedBackups::of(&[read_failed(), F::NewerFormat]), ListedBackups::Present);
        assert_eq!(
            ListedBackups::of(&[F::MasterKeyMissing, F::Unreadable(None)]),
            ListedBackups::Present
        );
    }
}

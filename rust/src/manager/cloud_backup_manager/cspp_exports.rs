use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use sha2::{Digest as _, Sha256};

#[uniffi::export]
pub fn cspp_master_key_record_id() -> String {
    MASTER_KEY_RECORD_ID.to_string()
}

pub(crate) fn master_key_wrapper_revision_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

#[uniffi::export]
pub fn cspp_master_key_filename() -> String {
    cove_cspp::backup_data::master_key_filename()
}

#[uniffi::export]
pub fn cspp_wallet_filename_from_record_id(record_id: String) -> String {
    cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)
}

#[uniffi::export]
pub fn cspp_master_key_directory() -> String {
    cove_cspp::backup_data::remote_layout::MASTER_KEY_DIRECTORY.to_string()
}

#[uniffi::export]
pub fn cspp_wallets_directory() -> String {
    cove_cspp::backup_data::remote_layout::WALLETS_DIRECTORY.to_string()
}

#[uniffi::export]
pub fn cspp_wallet_file_prefix() -> String {
    cove_cspp::backup_data::WALLET_FILE_PREFIX.to_string()
}

#[uniffi::export]
pub fn cspp_namespaces_subdirectory() -> String {
    cove_cspp::backup_data::NAMESPACES_SUBDIRECTORY.to_string()
}

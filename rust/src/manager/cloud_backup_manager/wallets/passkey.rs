mod authorization_retry;
mod material;
mod namespace_matcher;

use crate::manager::cloud_backup_manager::CloudBackupError;

pub(crate) use authorization_retry::PlatformAuthorizationRetrier;
pub(crate) use material::{
    PasskeyMaterialAcquirer, PasskeyMaterialOutcome, delay_before_new_passkey_auth,
    map_wrapper_repair_passkey_error,
};
pub(crate) use namespace_matcher::{
    NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher,
};

fn prf_output_to_key(prf_output: Vec<u8>) -> Result<[u8; 32], CloudBackupError> {
    prf_output
        .try_into()
        .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))
}

use rand::RngExt as _;
use zeroize::Zeroizing;

mod crypto;
mod error;
mod export;
pub(crate) mod import;
pub(crate) mod model;
mod verify;

pub use error::BackupError;
pub use model::{BackupImportReport, BackupResult, BackupVerifyReport};

#[derive(Debug, Clone, uniffi::Object)]
pub struct BackupManager;

#[uniffi::export]
impl BackupManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }

    pub async fn export(&self, password: String) -> Result<BackupResult, BackupError> {
        export::export_all(password).await
    }

    #[uniffi::method(name = "importBackup")]
    pub async fn import(
        &self,
        data: Vec<u8>,
        password: String,
    ) -> Result<BackupImportReport, BackupError> {
        import::import_all(data, password).await
    }

    #[uniffi::method(name = "verifyBackup")]
    pub async fn verify(
        &self,
        data: Vec<u8>,
        password: String,
    ) -> Result<BackupVerifyReport, BackupError> {
        verify::verify_backup(data, password).await
    }

    /// Validate the file format without decrypting
    pub fn validate_format(&self, data: &[u8]) -> Result<(), BackupError> {
        crypto::validate_header(data)
    }

    /// Generate a 12-word BIP39 mnemonic to use as the backup password
    ///
    /// NOTE: the returned String is not zeroized — bip39::Mnemonic::to_string()
    /// allocates a plain String, and the value crosses FFI to Swift/Kotlin where
    /// we can't control deallocation. The entropy source is zeroized, limiting
    /// the exposure window
    pub fn generate_password(&self) -> String {
        let entropy = Zeroizing::new(rand::rng().random::<[u8; 16]>());
        let mnemonic = bip39::Mnemonic::from_entropy(&*entropy)
            .expect("16 bytes is valid entropy for 12 words");
        mnemonic.to_string()
    }

    /// Check whether a password meets backup requirements
    pub fn is_password_valid(&self, password: &str) -> bool {
        crypto::clean_password(password).is_ok()
    }

    /// Account name for saving backup passwords to the system credential store
    pub fn backup_account_name(&self) -> String {
        let now = jiff::Zoned::now();
        format!("Cove Backup - {}", now.strftime("%Y-%m-%d"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_password() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandonabilityableaboutaboveabsentabsorb"));
    }

    #[test]
    fn valid_password_with_spaces() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandon ability able about above absent absorb"));
    }

    #[test]
    fn valid_password_with_tabs_and_newlines() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandon\tability\nable\tabout\tabove\tabsent\tabsorb"));
    }

    #[test]
    fn too_short_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("tooshort"));
    }

    #[test]
    fn empty_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid(""));
    }

    #[test]
    fn all_whitespace_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("                                    "));
    }

    #[test]
    fn exactly_32_chars_after_stripping() {
        let mgr = BackupManager::new();
        let password = "a".repeat(32);
        assert!(mgr.is_password_valid(&password));
    }

    #[test]
    fn short_after_stripping_whitespace() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("abc def ghi jkl mno"));
    }
}

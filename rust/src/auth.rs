use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use derive_more::{Display, FromStr};
use serde::{Deserialize, Serialize};

use crate::database;

use self::database::Database;

#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    Eq,
    PartialEq,
    uniffi::Enum,
    Serialize,
    Deserialize,
    Default,
    Display,
    FromStr,
)]
pub enum AuthType {
    Pin,
    Biometric,
    Both,

    #[default]
    None,
}

type Result<T, E = AuthError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum AuthError {
    #[error("Unable to save pin to database {0:?}")]
    DatabaseSaveError(database::Error),

    #[error("Unable to get pin from database {0:?}")]
    DatabaseGetError(database::Error),

    #[error("Unable to hash pin {0}")]
    HashError(String),

    #[error("Unable to parse hashed pin {0}")]
    ParseHashedPinError(String),

    #[error("Verification failed {0}")]
    VerificationFailed(String),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct AuthPin;

impl AuthPin {
    pub fn set(&self, pin: String) -> Result<()> {
        let hashed = self.hash(pin)?;

        Database::global()
            .global_config
            .set_hashed_pin_code(hashed)
            .map_err(AuthError::DatabaseSaveError)
    }

    pub fn hash(&self, pin: String) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let pin_hash = argon2
            .hash_password(pin.as_bytes(), &salt)
            .map_err(|error| AuthError::HashError(format!("unable to hash pin: {error}")))?
            .to_string();

        Ok(pin_hash)
    }

    pub fn delete(&self) -> Result<()> {
        Database::global()
            .global_config
            .delete_hashed_pin_code()
            .map_err(AuthError::DatabaseSaveError)
    }

    pub fn check(&self, pin: &str) -> bool {
        let hashed_pin = Database::global().global_config.hashed_pin_code().unwrap_or_default();

        self.verify(pin, &hashed_pin).is_ok()
    }

    pub fn verify(&self, pin: &str, hashed_pin: &str) -> Result<()> {
        let argon2 = Argon2::default();

        let parsed_hash = PasswordHash::new(hashed_pin).map_err(|error| {
            AuthError::ParseHashedPinError(format!("unable to parse hashed pin: {error}"))
        })?;

        argon2
            .verify_password(pin.as_bytes(), &parsed_hash)
            .map_err(|error| AuthError::VerificationFailed(format!("{error:?}")))
    }
}

#[uniffi::export]
impl AuthPin {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {}
    }

    #[uniffi::method(name = "check")]
    pub fn _check(&self, pin: String) -> bool {
        self.check(&pin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_pin() {
        let auth = AuthPin::new();
        let hashed = auth.hash("123456".to_string()).unwrap();
        assert!(hashed.starts_with("$argon2id"));
    }

    #[test]
    fn test_verify_pin() {
        let auth = AuthPin::new();
        let hashed = auth.hash("123456".to_string()).unwrap();
        let result = auth.verify("123456", &hashed);
        assert!(result.is_ok());
    }
}

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
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

#[derive(
    Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error, derive_more::Display,
)]
pub enum AuthError {
    /// Unable to save pin to database {0:?}
    DatabaseSaveError(database::Error),

    /// Unable to get pin from database {0:?}
    DatabaseGetError(database::Error),

    /// Unable to hash pin {0}
    HashError(String),

    /// Unable to parse hashed pin {0}
    ParseHashedPinError(String),

    /// Verification failed {0}
    VerificationFailed(String),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct AuthPin {}

#[uniffi::export]
impl AuthPin {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {}
    }

    #[uniffi::method]
    pub fn check(&self, pin: String) -> bool {
        let hashed_pin = Database::global()
            .global_config
            .hashed_pin_code()
            .unwrap_or_default();

        self.verify(pin, hashed_pin).is_ok()
    }

    #[uniffi::method]
    pub fn set(&self, pin: String) -> Result<()> {
        let hashed = self.hash(pin)?;
        Database::global()
            .global_config
            .set_hashed_pin_code(hashed)
            .map_err(|error| AuthError::DatabaseSaveError(error))
    }

    #[uniffi::method]
    pub fn hash(&self, pin: String) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let pin_hash = argon2
            .hash_password(pin.as_bytes(), &salt)
            .map_err(|error| AuthError::HashError(format!("unable to hash pin: {error}")))?
            .to_string();

        Ok(pin_hash)
    }

    #[uniffi::method]
    pub fn verify(&self, pin: String, hashed_pin: String) -> Result<()> {
        let argon2 = Argon2::default();

        let parsed_hash = PasswordHash::new(&hashed_pin).map_err(|error| {
            AuthError::ParseHashedPinError(format!("unable to parse hashed pin: {error}"))
        })?;

        argon2
            .verify_password(pin.as_bytes(), &parsed_hash)
            .map_err(|error| AuthError::VerificationFailed(format!("{error:?}")))
    }
}

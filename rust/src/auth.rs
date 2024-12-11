use derive_more::{Display, FromStr};
use serde::{Deserialize, Serialize};

use crate::database;

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
        todo!("CHECK PIN {pin}");
    }

    #[uniffi::method]
    pub fn set(&self, pin: String) -> Result<()> {
        todo!("SET PIN {pin}");
    }
}

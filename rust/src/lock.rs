use derive_more::{Display, FromStr};
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
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
pub enum LockType {
    Pin,
    Biometric,
    Both,

    #[default]
    None,
}

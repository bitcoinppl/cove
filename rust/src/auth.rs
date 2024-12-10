use derive_more::{Display, FromStr};
use serde::{Deserialize, Serialize};

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

pub mod client;

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum, strum::Display, Serialize, Deserialize,
)]
pub enum FiatCurrency {
    Usd,
    Eur,
    Gbp,
    Cad,
    Chf,
    Aud,
    Jpy,
}

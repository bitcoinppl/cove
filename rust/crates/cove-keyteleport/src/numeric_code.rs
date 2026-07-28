use std::{fmt, str::FromStr};

use crate::{Error, Result};

/// An eight-digit receiver code used to establish a KeyTeleport session
#[derive(Clone, PartialEq, Eq)]
pub struct NumericCode(String);

impl NumericCode {
    pub(crate) fn from_u32(value: u32) -> Self {
        Self(format!("{value:08}"))
    }

    /// Returns the receiver code without display grouping
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the receiver code grouped into pairs for display
    pub fn grouped(&self) -> String {
        self.0
            .as_bytes()
            .chunks(2)
            .map(|chunk| std::str::from_utf8(chunk).expect("numeric code is ASCII"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Debug for NumericCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NumericCode(****)")
    }
}

impl fmt::Display for NumericCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for NumericCode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim().replace(' ', "");

        if normalized.len() != 8 || !normalized.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidNumericCode);
        }

        Ok(Self(normalized))
    }
}

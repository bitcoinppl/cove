use std::{fmt, str::FromStr};

use data_encoding::BASE32_NOPAD;
use rand::RngExt as _;
use zeroize::{Zeroize as _, Zeroizing};

use crate::{Error, Result};

#[derive(Clone, PartialEq, Eq)]
pub struct TeleportPassword {
    bytes: [u8; 5],
}

impl TeleportPassword {
    pub fn generate() -> Self {
        let bytes = rand::rng().random::<[u8; 5]>();

        Self { bytes }
    }

    pub fn from_bytes(bytes: [u8; 5]) -> Self {
        Self { bytes }
    }

    pub fn expose_bytes(&self) -> &[u8; 5] {
        &self.bytes
    }

    pub fn as_display_text(&self) -> String {
        BASE32_NOPAD.encode(&self.bytes)
    }

    pub fn grouped(&self) -> String {
        self.as_display_text()
            .as_bytes()
            .chunks(2)
            .map(|chunk| std::str::from_utf8(chunk).expect("password text is ASCII"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Debug for TeleportPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TeleportPassword(****)")
    }
}

impl fmt::Display for TeleportPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_display_text())
    }
}

impl Drop for TeleportPassword {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl FromStr for TeleportPassword {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = normalize_password(value)?;
        let decoded = Zeroizing::new(BASE32_NOPAD.decode(normalized.as_bytes())?);
        let bytes: [u8; 5] =
            decoded.as_slice().try_into().map_err(|_| Error::InvalidTeleportPassword)?;

        Ok(Self { bytes })
    }
}

fn normalize_password(value: &str) -> Result<Zeroizing<String>> {
    let mut normalized = Zeroizing::new(String::with_capacity(8));

    for ch in value.chars() {
        if ch.is_ascii_whitespace() || ch == '-' {
            continue;
        }

        let ch = ch.to_ascii_uppercase();
        let ch = match ch {
            '0' => 'O',
            '1' => 'L',
            '8' => 'B',
            _ => ch,
        };

        if !ch.is_ascii_alphanumeric() {
            return Err(Error::InvalidTeleportPassword);
        }

        normalized.push(ch);
    }

    if normalized.len() != 8 {
        return Err(Error::InvalidTeleportPassword);
    }

    Ok(normalized)
}

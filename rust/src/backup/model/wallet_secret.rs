use serde::de::{Error as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secret material for a wallet, depends on wallet type
#[derive(Serialize, Zeroize, ZeroizeOnDrop)]
pub enum WalletSecret {
    /// Hot wallet BIP-39 mnemonic
    Mnemonic(String),
    /// Hot wallet BIP-32 extended private key
    Xprv(String),
    /// TapSigner encrypted backup bytes
    TapSignerBackup(Vec<u8>),
    /// No secret material (xpub-only / watch-only)
    None,
    /// Unrecognized variant from a newer backup version
    #[serde(skip_serializing)]
    Unknown,
}

impl<'de> Deserialize<'de> for WalletSecret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(WalletSecretVisitor)
    }
}

struct WalletSecretVisitor;

impl<'de> Visitor<'de> for WalletSecretVisitor {
    type Value = WalletSecret;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a wallet secret variant")
    }

    fn visit_str<E>(self, variant: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match variant {
            "None" => Ok(WalletSecret::None),
            "Mnemonic" | "Xprv" | "TapSignerBackup" => {
                Err(E::custom(format!("{variant} requires a value")))
            }
            _ => Ok(WalletSecret::Unknown),
        }
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let Some(variant) = map.next_key::<WalletSecretVariant>()? else {
            return Err(A::Error::custom("wallet secret object cannot be empty"));
        };

        let secret = match variant {
            WalletSecretVariant::Mnemonic => WalletSecret::Mnemonic(map.next_value()?),
            WalletSecretVariant::Xprv => WalletSecret::Xprv(map.next_value()?),
            WalletSecretVariant::TapSignerBackup => {
                WalletSecret::TapSignerBackup(map.next_value()?)
            }
            WalletSecretVariant::None => {
                map.next_value::<IgnoredAny>()?;
                return Err(A::Error::custom("None must use its unit variant representation"));
            }
            WalletSecretVariant::Unknown => {
                map.next_value::<IgnoredAny>()?;
                WalletSecret::Unknown
            }
        };

        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(A::Error::custom("wallet secret object must contain exactly one variant"));
        }

        Ok(secret)
    }
}

enum WalletSecretVariant {
    Mnemonic,
    Xprv,
    TapSignerBackup,
    None,
    Unknown,
}

impl<'de> Deserialize<'de> for WalletSecretVariant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(WalletSecretVariantVisitor)
    }
}

struct WalletSecretVariantVisitor;

impl Visitor<'_> for WalletSecretVariantVisitor {
    type Value = WalletSecretVariant;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a wallet secret variant name")
    }

    fn visit_str<E>(self, variant: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(match variant {
            "Mnemonic" => WalletSecretVariant::Mnemonic,
            "Xprv" => WalletSecretVariant::Xprv,
            "TapSignerBackup" => WalletSecretVariant::TapSignerBackup,
            "None" => WalletSecretVariant::None,
            _ => WalletSecretVariant::Unknown,
        })
    }
}

impl std::fmt::Debug for WalletSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mnemonic(_) => write!(f, "Mnemonic(****)"),
            Self::Xprv(_) => write!(f, "Xprv(****)"),
            Self::TapSignerBackup(_) => write!(f, "TapSignerBackup(****)"),
            Self::None => write!(f, "None"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_secret_variants_are_ignored_without_materializing_their_value() {
        let secret: WalletSecret =
            serde_json::from_str(r#"{"FutureSecret":{"nested":["private",{"material":[1,2,3]}]}}"#)
                .unwrap();

        assert!(matches!(secret, WalletSecret::Unknown));
    }

    #[test]
    fn unknown_unit_secret_variant_is_accepted() {
        let secret: WalletSecret = serde_json::from_str(r#""FutureSecret""#).unwrap();

        assert!(matches!(secret, WalletSecret::Unknown));
    }

    #[test]
    fn malformed_known_secret_variant_is_rejected() {
        assert!(serde_json::from_str::<WalletSecret>(r#"{"Mnemonic":42}"#).is_err());
    }

    #[test]
    fn wallet_secret_object_rejects_multiple_variants() {
        assert!(
            serde_json::from_str::<WalletSecret>(
                r#"{"FutureSecret":"opaque","Mnemonic":"abandon"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_secret_cannot_be_serialized() {
        assert!(serde_json::to_vec(&WalletSecret::Unknown).is_err());
    }

    #[test]
    fn none_secret_requires_unit_representation() {
        let secret: WalletSecret = serde_json::from_str(r#""None""#).unwrap();

        assert!(matches!(secret, WalletSecret::None));
        assert!(serde_json::from_str::<WalletSecret>(r#"{"None":null}"#).is_err());
    }
}

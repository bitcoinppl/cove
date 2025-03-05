#![allow(dead_code)]
use std::str::FromStr as _;

use bdk_chain::miniscript::descriptor::DescriptorType;
use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint};
use bdk_wallet::descriptor::ExtendedDescriptor;
use bdk_wallet::keys::bip39::Mnemonic;
use bdk_wallet::keys::{
    DerivableKey as _, DescriptorSecretKey as BdkDescriptorSecretKey, ExtendedKey,
};
use bdk_wallet::{CreateParams, KeychainKind};
use bdk_wallet::{
    keys::{DescriptorPublicKey as BdkDescriptorPublicKey, KeyMap},
    miniscript::descriptor::{DescriptorXKey, Wildcard},
    template::{Bip44, Bip49, Bip84, Bip84Public, DescriptorTemplate as _},
};
use bitcoin::secp256k1;

use crate::network::Network;

pub type Seed = [u8; 64];

#[derive(Debug, Clone, derive_more::Display, derive_more::From, derive_more::FromStr)]
pub struct DescriptorSecretKey(pub(crate) BdkDescriptorSecretKey);

pub type Error = DescriptorKeyParseError;

#[derive(Debug, thiserror::Error)]
pub enum DescriptorKeyParseError {
    #[error("invalid descriptor: {0:?}")]
    InvalidDescriptor(#[from] bdk_wallet::miniscript::Error),

    #[error("unsupported descriptor: {0}")]
    UnsupportedDescriptor(String),

    #[error("unsupported descriptor type: {0:?}")]
    UnsupportedDescriptorType(DescriptorType),

    #[error("no origin found")]
    NoOrigin,
}

#[derive(Debug, Clone)]
pub struct Descriptors {
    /// The external descriptor, main account
    pub external: Descriptor,
    /// The change descriptor
    pub internal: Descriptor,
}

#[derive(Debug, Clone)]
pub struct Descriptor {
    pub extended_descriptor: ExtendedDescriptor,
    pub key_map: KeyMap,
}

impl Descriptors {
    pub fn new_from_public(external: ExtendedDescriptor, internal: ExtendedDescriptor) -> Self {
        Self {
            external: Descriptor::new_from_public(external),
            internal: Descriptor::new_from_public(internal),
        }
    }

    pub fn into_create_params(self) -> CreateParams {
        bdk_wallet::Wallet::create(self.external.into_tuple(), self.internal.into_tuple())
    }

    pub fn origin(&self) -> Result<String, Error> {
        self.external.origin()
    }

    pub fn fingerprint(&self) -> Option<Fingerprint> {
        let pub_key = self.external.descriptor_public_key().ok()?;
        let fingerprint = pub_key.master_fingerprint();

        if fingerprint == Fingerprint::default() {
            return None;
        }

        Some(fingerprint)
    }
}

impl Descriptor {
    pub fn new_from_public(extended_descriptor: ExtendedDescriptor) -> Self {
        Self {
            extended_descriptor,
            key_map: KeyMap::new(),
        }
    }

    /// Parse a descriptor string into a `Descriptor` struct.
    pub fn parse_public_descriptor(descriptor: &str) -> Result<Self, Error> {
        let secp = &secp256k1::Secp256k1::signing_only();
        let (descriptor, key_map) =
            bdk_wallet::miniscript::Descriptor::<BdkDescriptorPublicKey>::parse_descriptor(
                secp, descriptor,
            )?;

        Ok(Self {
            extended_descriptor: descriptor,
            key_map,
        })
    }

    pub fn descriptor_public_key(&self) -> Result<&BdkDescriptorPublicKey, Error> {
        use bdk_wallet::miniscript::Descriptor as D;
        use bdk_wallet::miniscript::descriptor::ShInner;

        let key = match &self.extended_descriptor {
            D::Pkh(pk) => pk.as_inner(),
            D::Wpkh(pk) => pk.as_inner(),
            D::Tr(pk) => pk.internal_key(),
            D::Sh(pk) => match pk.as_inner() {
                ShInner::Wpkh(pk) => pk.as_inner(),
                _ => {
                    return Err(Error::UnsupportedDescriptor(
                        "unsupported wallet bare descriptor not wpkh".to_string(),
                    ));
                }
            },

            // not sure
            D::Bare(_pk) => {
                return Err(Error::UnsupportedDescriptor(
                    "unsupported wallet bare descriptor not wpkh".to_string(),
                ));
            }

            // multi-sig
            D::Wsh(_pk) => {
                return Err(Error::UnsupportedDescriptor(
                    "unsupported wallet, multisig".to_string(),
                ));
            }
        };

        Ok(key)
    }

    pub fn origin(&self) -> Result<String, Error> {
        let public_key = self.descriptor_public_key()?;

        let origin = match &public_key {
            BdkDescriptorPublicKey::Single(pk) => &pk.origin,
            BdkDescriptorPublicKey::XPub(pk) => &pk.origin,
            BdkDescriptorPublicKey::MultiXPub(pk) => &pk.origin,
        };

        let desc_type = self.extended_descriptor.desc_type();
        let desc_type_str = match desc_type {
            DescriptorType::Pkh => "pkh",
            DescriptorType::Wpkh => "wpkh",
            DescriptorType::Tr => "tr",
            DescriptorType::Sh => "sh",
            other => Err(Error::UnsupportedDescriptorType(other))?,
        };

        let (fingerprint, path) = origin.as_ref().ok_or(Error::NoOrigin)?;
        let origin = format!("{}([{}/{}])", desc_type_str, fingerprint, path);
        Ok(origin)
    }

    /// BIP84 for P2WPKH (Segwit)
    pub(crate) fn new_bip84(
        secret_key: &DescriptorSecretKey,
        keychain_kind: KeychainKind,
        network: Network,
    ) -> Self {
        let derivable_key = &secret_key.0;

        match derivable_key {
            BdkDescriptorSecretKey::XPrv(descriptor_x_key) => {
                let derivable_key = descriptor_x_key.xkey;
                let (extended_descriptor, key_map, _) = Bip84(derivable_key, keychain_kind)
                    .build(network.into())
                    .unwrap();

                Self {
                    extended_descriptor,
                    key_map,
                }
            }

            BdkDescriptorSecretKey::MultiXPrv(_) => {
                unreachable!()
            }

            BdkDescriptorSecretKey::Single(_) => {
                unreachable!()
            }
        }
    }

    /// BIP84 for P2WPKH (Segwit)
    #[allow(dead_code)]
    pub(crate) fn new_bip84_public(
        public_key: &BdkDescriptorPublicKey,
        fingerprint: String,
        keychain_kind: KeychainKind,
        network: Network,
    ) -> Self {
        let fingerprint = Fingerprint::from_str(fingerprint.as_str()).unwrap();
        let derivable_key = public_key;

        match derivable_key {
            BdkDescriptorPublicKey::XPub(descriptor_x_key) => {
                let derivable_key = descriptor_x_key.xkey;
                let (extended_descriptor, key_map, _) =
                    Bip84Public(derivable_key, fingerprint, keychain_kind)
                        .build(network.into())
                        .unwrap();

                Self {
                    extended_descriptor,
                    key_map,
                }
            }
            BdkDescriptorPublicKey::MultiXPub(_) => {
                unreachable!()
            }

            BdkDescriptorPublicKey::Single(_) => {
                unreachable!()
            }
        }
    }

    /// BIP49 for P2WPKH-nested-in-P2SH (Wrapped Segwit)
    pub(crate) fn new_bip49(
        secret_key: &DescriptorSecretKey,
        keychain_kind: KeychainKind,
        network: Network,
    ) -> Self {
        let derivable_key = &secret_key.0;

        match derivable_key {
            BdkDescriptorSecretKey::Single(_) => {
                unreachable!()
            }
            BdkDescriptorSecretKey::XPrv(descriptor_x_key) => {
                let derivable_key = descriptor_x_key.xkey;
                let (extended_descriptor, key_map, _) = Bip49(derivable_key, keychain_kind)
                    .build(network.into())
                    .unwrap();
                Self {
                    extended_descriptor,
                    key_map,
                }
            }
            BdkDescriptorSecretKey::MultiXPrv(_) => {
                unreachable!()
            }
        }
    }

    /// BIP44 for P2PKH (Legacy)
    pub(crate) fn new_bip44(
        secret_key: &DescriptorSecretKey,
        keychain_kind: KeychainKind,
        network: Network,
    ) -> Self {
        let derivable_key = &secret_key.0;

        match derivable_key {
            BdkDescriptorSecretKey::Single(_) => {
                unreachable!()
            }
            BdkDescriptorSecretKey::XPrv(descriptor_x_key) => {
                let derivable_key = descriptor_x_key.xkey;
                let (extended_descriptor, key_map, _) = Bip44(derivable_key, keychain_kind)
                    .build(network.into())
                    .unwrap();
                Self {
                    extended_descriptor,
                    key_map,
                }
            }
            BdkDescriptorSecretKey::MultiXPrv(_) => {
                unreachable!()
            }
        }
    }

    pub fn into_tuple(self) -> (ExtendedDescriptor, KeyMap) {
        (self.extended_descriptor, self.key_map)
    }
}

impl DescriptorSecretKey {
    pub(crate) fn new(network: Network, mnemonic: Mnemonic, passphrase: Option<String>) -> Self {
        let seed: Seed = mnemonic.to_seed(passphrase.as_deref().unwrap_or(""));
        let xkey: ExtendedKey = seed.into_extended_key().unwrap();

        let descriptor_secret_key = BdkDescriptorSecretKey::XPrv(DescriptorXKey {
            origin: None,
            xkey: xkey.into_xprv(network.into()).unwrap(),
            derivation_path: DerivationPath::master(),
            wildcard: Wildcard::Unhardened,
        });

        Self(descriptor_secret_key)
    }
}

impl From<ExtendedDescriptor> for Descriptor {
    fn from(descriptor: ExtendedDescriptor) -> Self {
        Self {
            extended_descriptor: descriptor,
            key_map: KeyMap::new(),
        }
    }
}

impl From<pubport::descriptor::Descriptors> for Descriptors {
    fn from(descriptors: pubport::descriptor::Descriptors) -> Self {
        let external = descriptors.external.into();
        let internal = descriptors.internal.into();

        Self { external, internal }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_descriptor_parse() {
        let desc = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7";
        let descriptor = Descriptor::parse_public_descriptor(desc);
        assert!(descriptor.is_ok());
    }

    #[test]
    fn test_descriptor_into_descriptor_public_key() {
        let desc = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7";
        let descriptor = Descriptor::parse_public_descriptor(desc);
        assert!(descriptor.is_ok());
        let descriptor = descriptor.unwrap();

        let public_key = descriptor.descriptor_public_key();
        assert!(public_key.is_ok());
    }

    #[test]
    fn test_descriptor_into_origin() {
        let desc = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7";
        let descriptor = Descriptor::parse_public_descriptor(desc);
        assert!(descriptor.is_ok());
        let descriptor = descriptor.unwrap();

        let origin = descriptor.origin();
        assert!(origin.is_ok());

        let origin = origin.unwrap();
        assert_eq!(origin, "wpkh([817e7be0/84'/0'/0'])");
    }
}

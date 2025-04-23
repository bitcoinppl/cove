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
use bitcoin::bip32::Xpub;
use bitcoin::secp256k1;

use crate::tap_card::tap_signer_reader::DeriveInfo;
use cove_types::Network;

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

    #[error("invalid public key")]
    InvalidPublicKey,

    #[error("invalid chain code")]
    InvalidChainCode,

    #[error("invalid bip84 path: {0:?}")]
    InvalidBip84Path(Vec<u32>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Descriptors {
    /// The external descriptor, main account
    pub external: Descriptor,
    /// The change descriptor
    pub internal: Descriptor,
}

#[derive(Debug, Clone, Eq, PartialEq)]
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

    pub fn new_from_tap_signer(derive: &DeriveInfo) -> Result<Self, Error> {
        use bitcoin::{
            NetworkKind,
            bip32::{ChainCode, ChildNumber, Xpub},
            secp256k1::PublicKey,
        };

        // dept is always 3 and always the first (0) child, derives the standard derivation path
        let depth = 3;
        let child_number = ChildNumber::Hardened { index: 0 };

        // using the master fingerprint as the parent fingerprint )
        let master_fingerprint = derive.master_fingerprint();
        let public_key =
            PublicKey::from_slice(&derive.pubkey).map_err(|_| Error::InvalidPublicKey)?;

        let chain_code_bytes: [u8; 32] = derive
            .chain_code
            .clone()
            .try_into()
            .map_err(|_| Error::InvalidChainCode)?;

        let chain_code = ChainCode::from(chain_code_bytes);

        let xpub = Xpub {
            network: NetworkKind::from(derive.network),
            depth,
            parent_fingerprint: master_fingerprint,
            child_number,
            public_key,
            chain_code,
        };

        let path = match derive.path.as_slice() {
            [84, 0, 0] => [84, 0, 0],
            [84, 1, 0] => [84, 1, 0],
            path => return Err(Error::InvalidBip84Path(path.to_vec())),
        };

        Self::try_new_bip84(xpub, path, master_fingerprint)
    }

    pub fn try_new_bip84(
        xpub: Xpub,
        path: [u32; 3],
        master_fingerprint: Fingerprint,
    ) -> Result<Self, Error> {
        let derivation_path = match path {
            [84, 0, 0] => "84h/0h/0h",
            [84, 1, 0] => "84h/1h/0h",
            path => return Err(Error::InvalidBip84Path(path.to_vec())),
        };

        let desc_string = format!("wpkh([{master_fingerprint}/{derivation_path}]{xpub}/<0;1>/*)");
        let desc = pubport::descriptor::Descriptors::try_from_line(&desc_string)
            .expect("valid descriptor, because xpub is valid");

        Ok(Self::from(desc))
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

    pub fn xpub(&self) -> Option<Xpub> {
        match self.descriptor_public_key() {
            Ok(BdkDescriptorPublicKey::XPub(xpub)) => Some(xpub.xkey),
            _ => None,
        }
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
    use pretty_assertions::assert_eq;

    fn desc() -> &'static str {
        "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7"
    }

    fn derive_info() -> DeriveInfo {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let original_xpub = bitcoin::bip32::Xpub::from_str(xpub).unwrap();

        let master_xpub = "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb";
        let master_xpub = bitcoin::bip32::Xpub::from_str(master_xpub).unwrap();

        let master_xpub_bytes = master_xpub.public_key.serialize();
        let xpub_bytes = original_xpub.public_key.serialize();

        DeriveInfo {
            network: Network::Bitcoin,
            master_pubkey: master_xpub_bytes.to_vec(),
            pubkey: xpub_bytes.to_vec(),
            chain_code: original_xpub.chain_code.to_bytes().to_vec(),
            path: vec![84, 0, 0],
        }
    }

    #[test]
    fn test_descriptor_parse() {
        let descriptor = Descriptor::parse_public_descriptor(desc());
        assert!(descriptor.is_ok());
    }

    #[test]
    fn test_descriptor_into_descriptor_public_key() {
        let descriptor = Descriptor::parse_public_descriptor(desc());
        assert!(descriptor.is_ok());
        let descriptor = descriptor.unwrap();

        let public_key = descriptor.descriptor_public_key();
        assert!(public_key.is_ok());
    }

    #[test]
    fn test_descriptor_into_origin() {
        let descriptor = Descriptor::parse_public_descriptor(desc());
        assert!(descriptor.is_ok());
        let descriptor = descriptor.unwrap();

        let origin = descriptor.origin();
        assert!(origin.is_ok());

        let origin = origin.unwrap();
        assert_eq!(origin, "wpkh([817e7be0/84'/0'/0'])");
    }

    #[test]
    fn test_from_tap_signer_create_descriptor() {
        let derive_info = derive_info();
        let parsed_descriptors = Descriptors::new_from_tap_signer(&derive_info);
        assert!(parsed_descriptors.is_ok());
    }

    #[test]
    fn test_from_tap_signer_creates_same_address() {
        let original_descriptor: Descriptors =
            pubport::descriptor::Descriptors::try_from_line(desc())
                .unwrap()
                .into();

        let parsed_descriptors = Descriptors::new_from_tap_signer(&derive_info()).unwrap();

        let mut original_wallet = original_descriptor
            .into_create_params()
            .create_wallet_no_persist()
            .unwrap();

        let mut parsed_wallet = parsed_descriptors
            .into_create_params()
            .create_wallet_no_persist()
            .unwrap();

        // verify  external addresses are same
        let original_address = original_wallet.next_unused_address(KeychainKind::External);
        let parsed_address = parsed_wallet.next_unused_address(KeychainKind::External);
        assert_eq!(original_address, parsed_address);

        // verify internal addresses are same
        let original_address = original_wallet.next_unused_address(KeychainKind::Internal);
        let parsed_address = parsed_wallet.next_unused_address(KeychainKind::Internal);
        assert_eq!(original_address, parsed_address);
    }

    #[test]
    fn test_xpub_from_tap_signer() {
        let derive_info = derive_info();
        let parsed_descriptors = Descriptors::new_from_tap_signer(&derive_info).unwrap();
        assert!(parsed_descriptors.external.xpub().is_some());
    }
}

use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::descriptor::ExtendedDescriptor;
use bdk_wallet::keys::bip39::Mnemonic;
use bdk_wallet::keys::{
    DerivableKey as _, DescriptorSecretKey as BdkDescriptorSecretKey, ExtendedKey,
};
use bdk_wallet::keys::{DescriptorPublicKey as BdkDescriptorPublicKey, KeyMap};
use bdk_wallet::miniscript::descriptor::{DescriptorXKey, Wildcard};
use bdk_wallet::template::{Bip84, Bip84Public, DescriptorTemplate as _};
use bdk_wallet::KeychainKind;

pub type Seed = [u8; 64];

#[derive(Debug)]
pub struct DescriptorSecretKey(pub(crate) BdkDescriptorSecretKey);

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum Bip32Error {
    #[error("cannot derive from a hardened key")]
    CannotDeriveFromHardenedKey,

    #[error("secp256k1 error: {0}")]
    Secp256k1(String),

    #[error("invalid child number: {0}")]
    InvalidChildNumber(u32),

    #[error("invalid format for child number")]
    InvalidChildNumberFormat,

    #[error("invalid derivation path format")]
    InvalidDerivationPathFormat,

    #[error("unknown version: {0}")]
    UnknownVersion(String),

    #[error("wrong extended key length: {0}")]
    WrongExtendedKeyLength(u32),

    #[error("base58 error: {0}")]
    Base58(String),

    #[error("hexadecimal conversion error: {0}")]
    Hex(String),

    #[error("invalid public key hex length: {0}")]
    InvalidPublicKeyHexLength(u32),

    #[error("unknown error: {0}")]
    UnknownError(String),
}
#[derive(Debug)]
pub struct Descriptor {
    pub extended_descriptor: ExtendedDescriptor,
    pub key_map: KeyMap,
}

impl Descriptor {
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
                let (extended_descriptor, key_map, _) =
                    Bip84(derivable_key, keychain_kind).build(network).unwrap();
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
                        .build(network)
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
}

impl DescriptorSecretKey {
    pub(crate) fn new(network: Network, mnemonic: Mnemonic, passphrase: Option<String>) -> Self {
        let seed: Seed = mnemonic.to_seed(passphrase.as_deref().unwrap_or(""));
        let xkey: ExtendedKey = seed.into_extended_key().unwrap();

        let descriptor_secret_key = BdkDescriptorSecretKey::XPrv(DescriptorXKey {
            origin: None,
            xkey: xkey.into_xprv(network).unwrap(),
            derivation_path: DerivationPath::master(),
            wildcard: Wildcard::Unhardened,
        });

        Self(descriptor_secret_key)
    }
}

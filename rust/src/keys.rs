use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint};
use bdk_wallet::descriptor::ExtendedDescriptor;
use bdk_wallet::keys::bip39::Mnemonic;
use bdk_wallet::keys::{
    DerivableKey as _, DescriptorSecretKey as BdkDescriptorSecretKey, ExtendedKey,
};
use bdk_wallet::keys::{DescriptorPublicKey as BdkDescriptorPublicKey, KeyMap};
use bdk_wallet::miniscript::descriptor::{DescriptorXKey, Wildcard};
use bdk_wallet::template::{Bip84, Bip84Public, DescriptorTemplate as _};
use bdk_wallet::KeychainKind;

use crate::wallet::Network;

pub type Seed = [u8; 64];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to parse descriptor secret key: {0}")]
    ParseDescriptorSecretKey(String),
}

#[derive(Debug, Clone, derive_more::Display, derive_more::From, derive_more::FromStr)]
pub struct DescriptorSecretKey(pub(crate) BdkDescriptorSecretKey);

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

    pub fn to_tuple(self) -> (ExtendedDescriptor, KeyMap) {
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

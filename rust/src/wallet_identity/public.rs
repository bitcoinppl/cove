use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{Fingerprint as BdkFingerprint, Xpub};
use bdk_wallet::descriptor::ExtendedDescriptor;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_device::keychain::{Keychain, KeychainError};
use cove_util::{result_ext::ResultExt as _, sha256_hash};

use crate::keys::{self, Descriptors};
use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::{
    WalletAddressType,
    metadata::{WalletMetadata, WalletType},
};
use crate::xpub::XpubExt as _;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) enum PublicWalletIdentity {
    Descriptor(PublicDescriptorIdentity),
    Xpub(PublicXpubIdentity),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct PublicDescriptorIdentity {
    external: CanonicalDescriptor,
    internal: CanonicalDescriptor,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct CanonicalDescriptor(String);

impl CanonicalDescriptor {
    fn from_descriptor(descriptor: &ExtendedDescriptor) -> Self {
        Self(descriptor.to_normalized_string())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct PublicXpubIdentity(String);

#[derive(Debug, thiserror::Error)]
pub(crate) enum PublicWalletIdentityError {
    #[error("failed to read keychain public wallet material: {0}")]
    Keychain(#[from] KeychainError),

    #[error("failed to synthesize descriptor identity from xpub: {0}")]
    Descriptor(#[from] keys::Error),

    #[error("failed to parse fingerprint: {0}")]
    Fingerprint(String),

    #[error("failed to parse xpub: {0}")]
    Xpub(String),

    #[error("failed to parse descriptor: {0}")]
    DescriptorParse(String),
}

impl PublicWalletIdentity {
    pub(crate) fn from_descriptors(descriptors: &Descriptors) -> Self {
        Self::from_descriptor_pair(
            &descriptors.external.extended_descriptor,
            &descriptors.internal.extended_descriptor,
        )
    }

    pub(crate) fn from_descriptor_pair(
        external: &ExtendedDescriptor,
        internal: &ExtendedDescriptor,
    ) -> Self {
        Self::Descriptor(PublicDescriptorIdentity {
            external: CanonicalDescriptor::from_descriptor(external),
            internal: CanonicalDescriptor::from_descriptor(internal),
        })
    }

    pub(crate) fn from_descriptor_strs(
        external: &str,
        internal: &str,
    ) -> Result<Self, PublicWalletIdentityError> {
        let external = ExtendedDescriptor::from_str(external)
            .map_err_str(PublicWalletIdentityError::DescriptorParse)?;
        let internal = ExtendedDescriptor::from_str(internal)
            .map_err_str(PublicWalletIdentityError::DescriptorParse)?;

        Ok(Self::from_descriptor_pair(&external, &internal))
    }

    pub(crate) fn from_xpub(xpub: Xpub) -> Self {
        Self::Xpub(PublicXpubIdentity(xpub.to_string()))
    }

    pub(crate) fn from_xpub_str_default_address_type(
        xpub: &str,
        fingerprint: Option<Fingerprint>,
        network: Network,
        address_type: WalletAddressType,
    ) -> Result<Self, PublicWalletIdentityError> {
        let xpub = Xpub::from_str(xpub).map_err_str(PublicWalletIdentityError::Xpub)?;

        let Some(fingerprint) = fingerprint else {
            return Ok(Self::from_xpub(xpub));
        };

        Self::from_xpub_default_address_type(xpub, fingerprint, network, address_type)
    }

    pub(crate) fn from_xpub_default_address_type(
        xpub: Xpub,
        fingerprint: Fingerprint,
        network: Network,
        address_type: WalletAddressType,
    ) -> Result<Self, PublicWalletIdentityError> {
        let coin_type = match network {
            Network::Bitcoin => 0,
            Network::Testnet | Network::Testnet4 | Network::Signet => 1,
        };

        let fingerprint = BdkFingerprint::from_str(&fingerprint.as_lowercase())
            .map_err_str(PublicWalletIdentityError::Fingerprint)?;

        let account = xpub.account_index();
        let descriptors = match address_type {
            WalletAddressType::NativeSegwit => {
                Descriptors::try_new_bip84(xpub, [84, coin_type, account], fingerprint)?
            }
            WalletAddressType::WrappedSegwit => {
                Descriptors::try_new_bip49(xpub, [49, coin_type, account], fingerprint)?
            }
            WalletAddressType::Legacy => {
                Descriptors::try_new_bip44(xpub, [44, coin_type, account], fingerprint)?
            }
        };

        Ok(Self::from_descriptors(&descriptors))
    }

    pub(crate) fn from_existing_wallet(
        metadata: &WalletMetadata,
        keychain: &Keychain,
    ) -> Result<Option<Self>, PublicWalletIdentityError> {
        if let Some((external, internal)) = keychain.get_public_descriptor(&metadata.id)? {
            return Ok(Some(Self::from_descriptor_pair(&external, &internal)));
        }

        let Some(xpub) = keychain.get_wallet_xpub(&metadata.id)? else {
            return Ok(None);
        };

        if let Some(fingerprint) = metadata.master_fingerprint.as_deref()
            && matches!(
                metadata.wallet_type,
                WalletType::Cold | WalletType::XpubOnly | WalletType::WatchOnly
            )
        {
            return Self::from_xpub_default_address_type(
                xpub,
                *fingerprint,
                metadata.network,
                metadata.address_type,
            )
            .map(Some);
        }

        Ok(Some(Self::from_xpub(xpub)))
    }

    pub(crate) fn redacted_hash(&self) -> String {
        let identity = match self {
            Self::Descriptor(identity) => {
                format!("descriptor:{}\n{}", identity.external.0, identity.internal.0)
            }
            Self::Xpub(identity) => format!("xpub:{}", identity.0),
        };
        let hash = sha256_hash(identity.as_bytes()).to_string();

        hash[..16].to_string()
    }
}

use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{Fingerprint as BdkFingerprint, Xpub};
use bdk_wallet::descriptor::ExtendedDescriptor;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_util::sha256_hash;

use super::{
    fingerprint::Fingerprint,
    metadata::{WalletMetadata, WalletType},
};
use crate::{
    keychain::{Keychain, KeychainError},
    keys::{self, Descriptors},
    network::Network,
};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum PublicWalletIdentity {
    Descriptor(PublicDescriptorIdentity),
    Xpub(PublicXpubIdentity),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PublicDescriptorIdentity {
    external: String,
    internal: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PublicXpubIdentity(String);

#[derive(Debug, thiserror::Error)]
pub enum PublicWalletIdentityError {
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
    pub fn from_descriptors(descriptors: &Descriptors) -> Self {
        Self::from_descriptor_pair(
            &descriptors.external.extended_descriptor,
            &descriptors.internal.extended_descriptor,
        )
    }

    pub fn from_descriptor_pair(
        external: &ExtendedDescriptor,
        internal: &ExtendedDescriptor,
    ) -> Self {
        Self::Descriptor(PublicDescriptorIdentity {
            external: external.to_normalized_string(),
            internal: internal.to_normalized_string(),
        })
    }

    pub fn from_descriptor_strs(
        external: &str,
        internal: &str,
    ) -> Result<Self, PublicWalletIdentityError> {
        let external = ExtendedDescriptor::from_str(external)
            .map_err(|error| PublicWalletIdentityError::DescriptorParse(error.to_string()))?;
        let internal = ExtendedDescriptor::from_str(internal)
            .map_err(|error| PublicWalletIdentityError::DescriptorParse(error.to_string()))?;

        Ok(Self::from_descriptor_pair(&external, &internal))
    }

    pub fn from_xpub(xpub: Xpub) -> Self {
        Self::Xpub(PublicXpubIdentity(xpub.to_string()))
    }

    pub fn from_xpub_str_default_bip84(
        xpub: &str,
        fingerprint: Option<Fingerprint>,
        network: Network,
    ) -> Result<Self, PublicWalletIdentityError> {
        let xpub = Xpub::from_str(xpub)
            .map_err(|error| PublicWalletIdentityError::Xpub(error.to_string()))?;

        let Some(fingerprint) = fingerprint else {
            return Ok(Self::from_xpub(xpub));
        };

        Self::from_xpub_default_bip84(xpub, fingerprint, network)
    }

    pub fn from_xpub_default_bip84(
        xpub: Xpub,
        fingerprint: Fingerprint,
        network: Network,
    ) -> Result<Self, PublicWalletIdentityError> {
        let coin_type = match network {
            Network::Bitcoin => 0,
            Network::Testnet | Network::Testnet4 | Network::Signet => 1,
        };
        let fingerprint = BdkFingerprint::from_str(&fingerprint.as_lowercase())
            .map_err(|error| PublicWalletIdentityError::Fingerprint(error.to_string()))?;
        let descriptors = Descriptors::try_new_bip84(xpub, [84, coin_type, 0], fingerprint)?;

        Ok(Self::from_descriptors(&descriptors))
    }

    pub fn from_existing_wallet(
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
            && matches!(metadata.wallet_type, WalletType::XpubOnly | WalletType::WatchOnly)
        {
            return Self::from_xpub_default_bip84(xpub, *fingerprint, metadata.network).map(Some);
        }

        Ok(Some(Self::from_xpub(xpub)))
    }

    pub fn redacted_hash(&self) -> String {
        let identity = match self {
            Self::Descriptor(identity) => {
                format!("descriptor:{}\n{}", identity.external, identity.internal)
            }
            Self::Xpub(identity) => format!("xpub:{}", identity.0),
        };
        let hash = sha256_hash(identity.as_bytes()).to_string();

        hash[..16].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor_pair(account: u32) -> Descriptors {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let descriptor = format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/<0;1>/*)");

        pubport::descriptor::Descriptors::try_from_line(&descriptor).unwrap().into()
    }

    #[test]
    fn descriptor_identity_normalizes_equivalent_origin_notation() {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let h_descriptor = format!("wpkh([817e7be0/84h/0h/0h]{xpub}/<0;1>/*)");
        let apostrophe_descriptor = format!("wpkh([817e7be0/84'/0'/0']{xpub}/<0;1>/*)");

        let h_descriptors: Descriptors =
            pubport::descriptor::Descriptors::try_from_line(&h_descriptor).unwrap().into();
        let apostrophe_descriptors: Descriptors =
            pubport::descriptor::Descriptors::try_from_line(&apostrophe_descriptor).unwrap().into();

        assert_eq!(
            PublicWalletIdentity::from_descriptors(&h_descriptors),
            PublicWalletIdentity::from_descriptors(&apostrophe_descriptors)
        );
    }

    #[test]
    fn descriptor_identity_distinguishes_same_fingerprint_different_accounts() {
        assert_ne!(
            PublicWalletIdentity::from_descriptors(&descriptor_pair(0)),
            PublicWalletIdentity::from_descriptors(&descriptor_pair(1))
        );
    }

    #[test]
    fn descriptor_identity_wins_over_xpub_identity() {
        let descriptors = descriptor_pair(0);
        let xpub = descriptors.external.xpub().unwrap();

        assert_ne!(
            PublicWalletIdentity::from_descriptors(&descriptors),
            PublicWalletIdentity::from_xpub(xpub)
        );
    }

    #[test]
    fn xpub_default_bip84_synthesizes_descriptor_identity() {
        let descriptors = descriptor_pair(0);
        let xpub = descriptors.external.xpub().unwrap();
        let fingerprint = Fingerprint::from(BdkFingerprint::from_str("817e7be0").unwrap());

        let identity =
            PublicWalletIdentity::from_xpub_default_bip84(xpub, fingerprint, Network::Bitcoin)
                .unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }
}

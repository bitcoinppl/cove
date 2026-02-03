use bdk_wallet::{
    bitcoin::bip32::{DerivationPath, Fingerprint, Xpub},
    keys::DescriptorPublicKey,
    miniscript::{Descriptor, descriptor::DescriptorType},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no origin found")]
    NoOrigin,

    #[error("unsupported descriptor: {0}")]
    UnsupportedDescriptor(String),

    #[error("unsupported descriptor type: {0:?}")]
    UnsupportedDescriptorType(DescriptorType),
}

type Result<T, E = Error> = std::result::Result<T, E>;

pub trait DescriptorExt {
    /// Returns the descriptor public key from the descriptor
    ///
    /// # Errors
    /// Returns an error if the descriptor type is unsupported (bare, multisig)
    fn descriptor_public_key(&self) -> Result<&DescriptorPublicKey, Error>;

    /// Returns the full origin string including descriptor type and fingerprint/path
    ///
    /// # Errors
    /// Returns an error if the descriptor type is unsupported or has no origin
    fn full_origin(&self) -> Result<String>;

    /// Returns the origin tuple of fingerprint and derivation path
    ///
    /// # Errors
    /// Returns an error if the descriptor has no origin or is unsupported
    fn origin(&self) -> Result<&(Fingerprint, DerivationPath)> {
        let public_key = self.descriptor_public_key()?;

        let origin = match &public_key {
            DescriptorPublicKey::Single(pk) => &pk.origin,
            DescriptorPublicKey::XPub(pk) => &pk.origin,
            DescriptorPublicKey::MultiXPub(pk) => &pk.origin,
        };

        origin.as_ref().ok_or(Error::NoOrigin)
    }

    /// Returns the derivation path from the origin
    ///
    /// # Errors
    /// Returns an error if the descriptor has no origin or is unsupported
    fn derivation_path(&self) -> Result<DerivationPath> {
        let origin = self.origin()?;
        Ok(origin.1.clone())
    }

    fn xpub(&self) -> Option<Xpub> {
        match self.descriptor_public_key() {
            Ok(DescriptorPublicKey::XPub(xpub)) => Some(xpub.xkey),
            _ => None,
        }
    }
}

impl DescriptorExt for Descriptor<DescriptorPublicKey> {
    #[allow(clippy::use_self)] // using D alias for readability
    fn descriptor_public_key(&self) -> Result<&DescriptorPublicKey, Error> {
        use bdk_wallet::miniscript::Descriptor as D;
        use bdk_wallet::miniscript::descriptor::ShInner;

        let key = match &self {
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

    fn full_origin(&self) -> Result<String> {
        let desc_type = self.desc_type();
        let desc_type_str = match desc_type {
            DescriptorType::Pkh => "pkh",
            DescriptorType::Wpkh => "wpkh",
            DescriptorType::Tr => "tr",
            DescriptorType::Sh => "sh",
            other => Err(Error::UnsupportedDescriptorType(other))?,
        };

        let origin = self.origin()?;
        let (fingerprint, path) = origin;
        let origin = format!("{desc_type_str}([{fingerprint}/{path}])");
        Ok(origin)
    }
}

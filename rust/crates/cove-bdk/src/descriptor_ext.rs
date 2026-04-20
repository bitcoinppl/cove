use bdk_wallet::{
    bitcoin::bip32::{DerivationPath, Fingerprint, Xpub},
    keys::DescriptorPublicKey,
    miniscript::{Descriptor, descriptor::DescriptorType, descriptor::checksum::desc_checksum},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no origin found")]
    NoOrigin,

    #[error("unsupported descriptor: {0}")]
    UnsupportedDescriptor(String),

    #[error("unsupported descriptor type: {0:?}")]
    UnsupportedDescriptorType(DescriptorType),

    #[error("descriptors are not a matching external/internal pair")]
    NotMatchingPair,

    #[error("multisig descriptors are not yet supported")]
    MultisigNotSupported,
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

    /// Format descriptor with `h` hardened notation and a matching checksum
    ///
    /// miniscript uses `'` (apostrophe) for hardened steps, but many wallets
    /// (Sparrow, Electrum) normalize to `h` before validating the checksum,
    /// which causes a mismatch. This method produces the `h`-notation string
    /// with a correctly recomputed checksum so the descriptor round-trips
    /// through any BIP-380-compliant parser
    fn to_normalized_string(&self) -> String;

    /// Combine external (`/0/*`) and internal (`/1/*`) descriptors into a single
    /// BIP-389 multipath descriptor using `<0;1>` notation
    ///
    /// Takes the external descriptor, replaces `/0/*)` with `/<0;1>/*)`,
    /// normalizes to `h` notation, and recomputes the checksum
    ///
    /// Returns `Err(NotMatchingPair)` if the descriptors differ beyond the keychain index
    fn to_multipath_string(external: &Self, internal: &Self) -> Result<String>
    where
        Self: Sized;

    /// Export external/internal descriptors for wallet import
    ///
    /// Tries single-line BIP-389 multipath `<0;1>` format first,
    /// falls back to two normalized descriptors separated by a newline
    fn to_export_string(external: &Self, internal: &Self) -> String
    where
        Self: Sized;
}

impl DescriptorExt for Descriptor<DescriptorPublicKey> {
    fn to_normalized_string(&self) -> String {
        // {:#} produces the descriptor body without a checksum
        let body = format!("{self:#}").replace('\'', "h");
        let checksum = desc_checksum(&body).expect("valid descriptor body");
        format!("{body}#{checksum}")
    }

    fn to_multipath_string(external: &Self, internal: &Self) -> Result<String> {
        let external_body = format!("{external:#}").replace('\'', "h");
        let internal_body = format!("{internal:#}").replace('\'', "h");

        // both descriptors must be identical except for the keychain index (0 vs 1)
        let expected_internal = external_body.replace("/0/*)", "/1/*)");
        if internal_body != expected_internal {
            return Err(Error::NotMatchingPair);
        }

        let body = external_body.replace("/0/*)", "/<0;1>/*)");
        let checksum = desc_checksum(&body).expect("valid descriptor body");
        Ok(format!("{body}#{checksum}"))
    }

    fn to_export_string(external: &Self, internal: &Self) -> String {
        match Self::to_multipath_string(external, internal) {
            Ok(multipath) => multipath,
            Err(_) => {
                format!("{}\n{}", external.to_normalized_string(), internal.to_normalized_string())
            }
        }
    }

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
                ShInner::Wsh(_) => {
                    return Err(Error::MultisigNotSupported);
                }
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
                return Err(Error::MultisigNotSupported);
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
            DescriptorType::Wsh
            | DescriptorType::ShWsh
            | DescriptorType::WshSortedMulti
            | DescriptorType::ShWshSortedMulti => return Err(Error::MultisigNotSupported),
            other => Err(Error::UnsupportedDescriptorType(other))?,
        };

        let origin = self.origin()?;
        let (fingerprint, path) = origin;
        let origin = format!("{desc_type_str}([{fingerprint}/{path}])");
        Ok(origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type ExtendedDescriptor = Descriptor<DescriptorPublicKey>;

    /// Verify that {{:#}} produces the descriptor body without a checksum.
    /// This is documented miniscript behavior since v8.0.0, but we test it
    /// here so any upstream change breaks our build immediately
    #[test]
    fn alternate_display_omits_checksum() {
        let desc: ExtendedDescriptor = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/0/*)#sqx4cjta"
            .parse()
            .unwrap();

        let normal = format!("{desc}");
        let alternate = format!("{desc:#}");

        assert!(normal.contains('#'), "normal Display should include checksum");
        assert!(!alternate.contains('#'), "alternate Display should omit checksum");

        // the body from alternate plus a recomputed checksum should equal the normal output
        let checksum = desc_checksum(&alternate).unwrap();
        assert_eq!(normal, format!("{alternate}#{checksum}"));
    }

    fn matching_pair() -> (ExtendedDescriptor, ExtendedDescriptor) {
        let ext: ExtendedDescriptor = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/0/*)#sqx4cjta"
            .parse()
            .unwrap();
        let int: ExtendedDescriptor = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/1/*)#p5r598m9"
            .parse()
            .unwrap();
        (ext, int)
    }

    fn mismatched_pair() -> (ExtendedDescriptor, ExtendedDescriptor) {
        let ext: ExtendedDescriptor = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/0/*)#sqx4cjta"
            .parse()
            .unwrap();
        // different fingerprint
        let int: ExtendedDescriptor = "wpkh([aaaaaaaa/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/1/*)#eeutfex9"
            .parse()
            .unwrap();
        (ext, int)
    }

    #[test]
    fn multipath_rejects_mismatched_descriptors() {
        let (ext, int) = mismatched_pair();
        let result = DescriptorExt::to_multipath_string(&ext, &int);
        assert!(matches!(result, Err(Error::NotMatchingPair)));
    }

    #[test]
    fn multipath_accepts_matching_descriptors() {
        let (ext, int) = matching_pair();
        let result = DescriptorExt::to_multipath_string(&ext, &int).unwrap();
        assert!(result.contains("/<0;1>/*"));
    }

    #[test]
    fn export_string_uses_multipath_for_matching_pair() {
        let (ext, int) = matching_pair();
        let export = DescriptorExt::to_export_string(&ext, &int);
        assert!(export.contains("/<0;1>/*"));
        assert!(!export.contains('\n'), "matching pair should be a single line");
    }

    fn wsh_sortedmulti_desc() -> ExtendedDescriptor {
        // 2-of-2 wsh(sortedmulti) using bare compressed public keys (no xpub derivation)
        "wsh(sortedmulti(2,02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5,03774ae7f858a9411e5ef4246b70c65aac5649980be5c17891bbec17895da008cb))"
            .parse()
            .expect("valid wsh(sortedmulti) descriptor")
    }

    #[test]
    fn wsh_sortedmulti_descriptor_public_key_returns_multisig_not_supported() {
        let desc = wsh_sortedmulti_desc();
        let result = DescriptorExt::descriptor_public_key(&desc);
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }

    #[test]
    fn wsh_sortedmulti_full_origin_returns_multisig_not_supported() {
        let desc = wsh_sortedmulti_desc();
        let result = desc.full_origin();
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }

    #[test]
    fn wsh_sortedmulti_origin_returns_multisig_not_supported() {
        let desc = wsh_sortedmulti_desc();
        let result = desc.origin();
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }

    #[test]
    fn export_string_falls_back_to_two_lines() {
        let (ext, int) = mismatched_pair();
        let export = DescriptorExt::to_export_string(&ext, &int);
        let lines: Vec<&str> = export.lines().collect();
        assert_eq!(lines.len(), 2, "mismatched pair should produce two lines");
        assert_eq!(lines[0], ext.to_normalized_string());
        assert_eq!(lines[1], int.to_normalized_string());
    }

    fn sh_wsh_sortedmulti_desc() -> ExtendedDescriptor {
        "sh(wsh(sortedmulti(2,02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5,03774ae7f858a9411e5ef4246b70c65aac5649980be5c17891bbec17895da008cb)))"
            .parse()
            .expect("valid sh(wsh(sortedmulti)) descriptor")
    }

    #[test]
    fn sh_wsh_sortedmulti_descriptor_public_key_returns_multisig_not_supported() {
        let desc = sh_wsh_sortedmulti_desc();
        let result = DescriptorExt::descriptor_public_key(&desc);
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }

    #[test]
    fn sh_wsh_sortedmulti_full_origin_returns_multisig_not_supported() {
        let desc = sh_wsh_sortedmulti_desc();
        let result = desc.full_origin();
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }

    #[test]
    fn sh_wsh_sortedmulti_origin_returns_multisig_not_supported() {
        let desc = sh_wsh_sortedmulti_desc();
        let result = desc.origin();
        assert!(
            matches!(result, Err(Error::MultisigNotSupported)),
            "expected MultisigNotSupported, got {result:?}"
        );
    }
}

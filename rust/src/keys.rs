#![allow(dead_code)]
use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint};
use bdk_wallet::chain::miniscript::descriptor::DescriptorType;
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
use cove_bdk::descriptor_ext::DescriptorExt as _;

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
        self.external.full_origin()
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

        let chain_code_bytes: [u8; 32] =
            derive.chain_code.clone().try_into().map_err(|_| Error::InvalidChainCode)?;

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
    pub const fn new_from_public(extended_descriptor: ExtendedDescriptor) -> Self {
        Self { extended_descriptor, key_map: KeyMap::new() }
    }

    /// Parse a descriptor string into a `Descriptor` struct.
    pub fn parse_public_descriptor(descriptor: &str) -> Result<Self, Error> {
        let secp = &secp256k1::Secp256k1::signing_only();
        let (descriptor, key_map) =
            bdk_wallet::miniscript::Descriptor::<BdkDescriptorPublicKey>::parse_descriptor(
                secp, descriptor,
            )?;

        Ok(Self { extended_descriptor: descriptor, key_map })
    }

    pub fn descriptor_public_key(&self) -> Result<&BdkDescriptorPublicKey, Error> {
        self.extended_descriptor.descriptor_public_key().map_err(Into::into)
    }

    pub fn xpub(&self) -> Option<Xpub> {
        self.extended_descriptor.xpub()
    }

    pub fn full_origin(&self) -> Result<String, Error> {
        self.extended_descriptor.full_origin().map_err(Into::into)
    }

    pub fn origin(&self) -> Result<&(Fingerprint, DerivationPath), Error> {
        self.extended_descriptor.origin().map_err(Into::into)
    }

    pub fn derivation_path(&self) -> Result<DerivationPath, Error> {
        self.extended_descriptor.derivation_path().map_err(Into::into)
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
                let (extended_descriptor, key_map, _) =
                    Bip84(derivable_key, keychain_kind).build(network.into()).unwrap();

                Self { extended_descriptor, key_map }
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

                Self { extended_descriptor, key_map }
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
                let (extended_descriptor, key_map, _) =
                    Bip49(derivable_key, keychain_kind).build(network.into()).unwrap();
                Self { extended_descriptor, key_map }
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
                let (extended_descriptor, key_map, _) =
                    Bip44(derivable_key, keychain_kind).build(network.into()).unwrap();
                Self { extended_descriptor, key_map }
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
        Self { extended_descriptor: descriptor, key_map: KeyMap::new() }
    }
}

impl From<pubport::descriptor::Descriptors> for Descriptors {
    fn from(descriptors: pubport::descriptor::Descriptors) -> Self {
        // TODO: remove string round-trip once bdk_wallet updates to miniscript 0.13
        // expect is okay: descriptor already validated by pubport, just bridging miniscript versions
        let external: ExtendedDescriptor =
            descriptors.external.to_string().parse().expect("pubport already validated");
        let internal: ExtendedDescriptor =
            descriptors.internal.to_string().parse().expect("pubport already validated");

        Self { external: external.into(), internal: internal.into() }
    }
}

impl From<cove_bdk::descriptor_ext::Error> for DescriptorKeyParseError {
    fn from(error: cove_bdk::descriptor_ext::Error) -> Self {
        use cove_bdk::descriptor_ext::Error as E;
        match error {
            E::NoOrigin => Self::NoOrigin,
            E::UnsupportedDescriptor(s) => Self::UnsupportedDescriptor(s),
            E::UnsupportedDescriptorType(s) => Self::UnsupportedDescriptorType(s),
            E::NotMatchingPair => Self::UnsupportedDescriptor(error.to_string()),
        }
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

        let origin = descriptor.full_origin();
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
            pubport::descriptor::Descriptors::try_from_line(desc()).unwrap().into();

        let parsed_descriptors = Descriptors::new_from_tap_signer(&derive_info()).unwrap();

        let mut original_wallet =
            original_descriptor.into_create_params().create_wallet_no_persist().unwrap();

        let mut parsed_wallet =
            parsed_descriptors.into_create_params().create_wallet_no_persist().unwrap();

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

    fn test_xpub() -> &'static str {
        "xpub6DM7CYgaTMdMbhTcLTUWmNUE5WLXK5hx8ZMa4sRw8qYJPqtqKYiKnwsmT8A6AijDVAUZRivdBnXdR8QE7Y9vVnqvzPL3fXCmu1WtCRLdAoz"
    }

    #[test]
    fn test_user_reported_checksum_bug() {
        use cove_bdk::descriptor_ext::DescriptorExt;

        let xpub = test_xpub();
        let fingerprint = "a262308d";

        // build the multipath descriptor like try_new_bip84 does
        let multipath = format!("wpkh([{fingerprint}/84h/0h/0h]{xpub}/<0;1>/*)");
        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(&multipath).unwrap();
        let bdk_descs: Descriptors = pubport_descs.into();

        // the raw BDK Display uses ' notation — this is what caused the Sparrow error
        let raw_int = bdk_descs.internal.extended_descriptor.to_string();
        assert!(raw_int.contains("84'"), "raw BDK output uses ' notation");

        // to_normalized_string() should produce h-notation with correct checksum
        let ext_str = bdk_descs.external.extended_descriptor.to_normalized_string();
        let int_str = bdk_descs.internal.extended_descriptor.to_normalized_string();

        assert!(ext_str.contains("84h/0h/0h"), "external should use h-notation");
        assert!(int_str.contains("84h/0h/0h"), "internal should use h-notation");

        assert_valid_checksum(&ext_str, "normalized external");
        assert_valid_checksum(&int_str, "normalized internal");

        // verify the old buggy checksum is NOT present
        let int_checksum = extract_checksum(&int_str);
        assert_ne!(int_checksum, "j3u3ae2x", "should not use apostrophe-notation checksum");

        // multipath format should combine into <0;1> with valid checksum
        let multipath_str = DescriptorExt::to_multipath_string(
            &bdk_descs.external.extended_descriptor,
            &bdk_descs.internal.extended_descriptor,
        )
        .unwrap();

        assert!(multipath_str.contains("/<0;1>/*"), "should use multipath notation");
        assert!(multipath_str.contains("84h/0h/0h"), "multipath should use h-notation");
        assert_valid_checksum(&multipath_str, "multipath");
    }

    /// Helper: extract the checksum from a descriptor string (after '#')
    fn extract_checksum(desc_str: &str) -> &str {
        desc_str.rsplit('#').next().unwrap()
    }

    /// Helper: compute expected checksum from descriptor body (before '#')
    fn compute_checksum(desc_str: &str) -> String {
        let body = desc_str.split('#').next().unwrap();
        bdk_wallet::miniscript::descriptor::checksum::desc_checksum(body).unwrap()
    }

    /// Helper: verify a descriptor string has a valid checksum
    fn assert_valid_checksum(desc_str: &str, label: &str) {
        let attached = extract_checksum(desc_str);
        let expected = compute_checksum(desc_str);
        assert_eq!(
            attached, expected,
            "{label}: checksum mismatch — attached '{attached}', expected '{expected}'\n  descriptor: {desc_str}"
        );
    }

    #[test]
    fn test_pubport_to_bdk_roundtrip_checksum() {
        // test the From<pubport::descriptor::Descriptors> conversion at keys.rs:323-333
        // miniscript 13 → string → miniscript 12 → string should produce valid checksums
        let multipath = "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7";

        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(multipath).unwrap();

        // check pubport (miniscript 13) produces valid checksums
        let ext_13 = pubport_descs.external.to_string();
        let int_13 = pubport_descs.internal.to_string();
        assert_valid_checksum(&ext_13, "pubport external");
        assert_valid_checksum(&int_13, "pubport internal");

        // convert to BDK (miniscript 12) via the From impl
        let bdk_descs: Descriptors = pubport_descs.into();
        let ext_12 = bdk_descs.external.extended_descriptor.to_string();
        let int_12 = bdk_descs.internal.extended_descriptor.to_string();
        assert_valid_checksum(&ext_12, "BDK external");
        assert_valid_checksum(&int_12, "BDK internal");
    }

    #[test]
    fn test_pubport_to_bdk_roundtrip_checksum_apostrophe_notation() {
        // same test with ' (apostrophe) hardened notation
        let multipath = "wpkh([817e7be0/84'/0'/0']xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)";

        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(multipath).unwrap();
        let bdk_descs: Descriptors = pubport_descs.into();

        let ext_str = bdk_descs.external.extended_descriptor.to_string();
        let int_str = bdk_descs.internal.extended_descriptor.to_string();
        assert_valid_checksum(&ext_str, "BDK external (apostrophe input)");
        assert_valid_checksum(&int_str, "BDK internal (apostrophe input)");
    }

    #[test]
    fn test_descriptor_checksum_survives_save_load_cycle() {
        use cove_bdk::descriptor_ext::DescriptorExt as _;

        // simulate save_public_descriptor → get_public_descriptor → export (normalized)
        let multipath = desc();
        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(multipath).unwrap();
        let bdk_descs: Descriptors = pubport_descs.into();

        // save: format!("{ext}\n{int}") — saved with apostrophe notation
        let saved = format!(
            "{}\n{}",
            bdk_descs.external.extended_descriptor, bdk_descs.internal.extended_descriptor
        );

        // load: parse back from string
        let mut lines = saved.lines();
        let ext_loaded: ExtendedDescriptor = lines.next().unwrap().parse().unwrap();
        let int_loaded: ExtendedDescriptor = lines.next().unwrap().parse().unwrap();

        // export: normalize to h-notation (like get_public_descriptor_content does)
        let ext_exported = ext_loaded.to_normalized_string();
        let int_exported = int_loaded.to_normalized_string();

        assert!(ext_exported.contains("84h/0h/0h"), "exported external should use h-notation");
        assert!(int_exported.contains("84h/0h/0h"), "exported internal should use h-notation");
        assert_valid_checksum(&ext_exported, "round-trip external");
        assert_valid_checksum(&int_exported, "round-trip internal");
    }

    #[test]
    fn test_multipath_split_independent_checksums() {
        // verify split descriptors get independent correct checksums, not the multipath's checksum
        let multipath = desc();
        let multipath_checksum = extract_checksum(multipath);

        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(multipath).unwrap();
        let bdk_descs: Descriptors = pubport_descs.into();

        let ext_str = bdk_descs.external.extended_descriptor.to_string();
        let int_str = bdk_descs.internal.extended_descriptor.to_string();

        // each split descriptor should have its own checksum, different from multipath
        let ext_checksum = extract_checksum(&ext_str);
        let int_checksum = extract_checksum(&int_str);

        assert_ne!(
            ext_checksum, multipath_checksum,
            "external checksum should differ from multipath checksum"
        );
        assert_ne!(
            int_checksum, multipath_checksum,
            "internal checksum should differ from multipath checksum"
        );
        assert_ne!(ext_checksum, int_checksum, "external and internal checksums should differ");

        assert_valid_checksum(&ext_str, "split external");
        assert_valid_checksum(&int_str, "split internal");
    }

    #[test]
    fn test_bdk_wallet_public_descriptor_checksums() {
        // test the fallback export path: create BDK wallet → public_descriptor()
        let pubport_descs = pubport::descriptor::Descriptors::try_from_line(desc()).unwrap();
        let bdk_descs: Descriptors = pubport_descs.into();

        let wallet = bdk_descs.into_create_params().create_wallet_no_persist().unwrap();

        let ext = wallet.public_descriptor(KeychainKind::External);
        let int = wallet.public_descriptor(KeychainKind::Internal);

        let ext_str = ext.to_string();
        let int_str = int.to_string();

        assert_valid_checksum(&ext_str, "BDK wallet external");
        assert_valid_checksum(&int_str, "BDK wallet internal");
    }

    #[test]
    fn test_multipath_export_normalizes_notation() {
        use cove_bdk::descriptor_ext::DescriptorExt;

        let xpub = "xpub6DRKtpLKk2qctgengkaD7B6w32X5w6RAUntvLeS1uA9dz93Y1RRopvPBdRdA3KLdnxYyjWiFePzpZpVEJ6LcuiugmrijzzHeatrGcDvz4Yq";
        let fp = "831a3f84";

        // input with h notation
        let h_multipath = format!("wpkh([{fp}/84h/0h/0h]{xpub}/<0;1>/*)");
        let h_descs: Descriptors =
            pubport::descriptor::Descriptors::try_from_line(&h_multipath).unwrap().into();

        // input with ' notation (same key, different notation)
        let apos_multipath = format!("wpkh([{fp}/84'/0'/0']{xpub}/<0;1>/*)");
        let apos_descs: Descriptors =
            pubport::descriptor::Descriptors::try_from_line(&apos_multipath).unwrap().into();

        // both should produce the same multipath export
        let h_export = DescriptorExt::to_multipath_string(
            &h_descs.external.extended_descriptor,
            &h_descs.internal.extended_descriptor,
        )
        .unwrap();
        let apos_export = DescriptorExt::to_multipath_string(
            &apos_descs.external.extended_descriptor,
            &apos_descs.internal.extended_descriptor,
        )
        .unwrap();

        assert_eq!(h_export, apos_export, "both notations should produce identical export");
        assert!(h_export.contains("84h/0h/0h"), "should use h-notation");
        assert!(h_export.contains("/<0;1>/*"), "should use multipath notation");
        assert_valid_checksum(&h_export, "multipath export");
    }

    #[test]
    fn test_tap_signer_descriptor_checksums() {
        let parsed_descriptors = Descriptors::new_from_tap_signer(&derive_info()).unwrap();

        let ext_str = parsed_descriptors.external.extended_descriptor.to_string();
        let int_str = parsed_descriptors.internal.extended_descriptor.to_string();

        assert_valid_checksum(&ext_str, "tap signer external");
        assert_valid_checksum(&int_str, "tap signer internal");
    }
}

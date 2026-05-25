mod backup;
mod existing;
mod identity_set;
mod public;

pub(crate) use backup::identity_key_for_backup;
pub(crate) use existing::{
    collect_existing_wallet_identities, existing_public_wallet_by_identity_strict,
};
pub(crate) use identity_set::{ExistingWalletIdentitySet, WalletIdentityKey};
pub(crate) use public::{PublicWalletIdentity, PublicWalletIdentityError};

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use super::identity_set::test_support::*;
}

use crate::{database, wallet::metadata::WalletId};

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletIdentityError {
    #[error("failed to read wallets: {0}")]
    Database(#[from] database::Error),

    #[error("public identity for existing wallet {wallet_id}: {source}")]
    ExistingWalletPublicIdentity { wallet_id: WalletId, source: PublicWalletIdentityError },

    #[error("same-fingerprint wallet {wallet_id} is missing public identity")]
    MissingExistingWalletPublicIdentity { wallet_id: WalletId },

    #[error("public descriptor identity for {wallet_name}: {source}")]
    BackupDescriptor { wallet_name: String, source: PublicWalletIdentityError },

    #[error("xpub identity for {wallet_name}: {source}")]
    BackupXpub { wallet_name: String, source: PublicWalletIdentityError },
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr as _;
    use std::sync::{Arc, Once};

    use bdk_wallet::bitcoin::bip32::{Fingerprint as BdkFingerprint, Xpub};
    use cove_device::keychain::{Keychain, KeychainAccess, KeychainError};

    use super::existing::matching_public_wallet_by_identity;
    use super::*;
    use crate::backup::model::{DescriptorPair, WalletBackup, WalletSecret};
    use crate::keys::Descriptors;
    use crate::network::Network;
    use crate::wallet::{
        WalletAddressType,
        fingerprint::Fingerprint,
        metadata::{WalletMetadata, WalletType},
    };

    #[derive(Debug, Default)]
    struct TestKeychain(parking_lot::Mutex<HashMap<String, String>>);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.0.lock().remove(&key).is_some()
        }
    }

    fn test_keychain() -> &'static Keychain {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            Keychain::new(Box::<TestKeychain>::default());
        });

        Keychain::global()
    }

    fn descriptor_pair(account: u32) -> Descriptors {
        descriptor_pair_for_address_type(WalletAddressType::NativeSegwit, account)
    }

    fn descriptor_pair_for_address_type(
        address_type: WalletAddressType,
        account: u32,
    ) -> Descriptors {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let descriptor = match address_type {
            WalletAddressType::NativeSegwit => {
                format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/<0;1>/*)")
            }
            WalletAddressType::WrappedSegwit => {
                format!("sh(wpkh([817e7be0/49h/0h/{account}h]{xpub}/<0;1>/*))")
            }
            WalletAddressType::Legacy => {
                format!("pkh([817e7be0/44h/0h/{account}h]{xpub}/<0;1>/*)")
            }
        };

        pubport::descriptor::Descriptors::try_from_line(&descriptor).unwrap().into()
    }

    fn descriptors(account: u32) -> DescriptorPair {
        descriptors_for_address_type(WalletAddressType::NativeSegwit, account)
    }

    fn descriptors_for_address_type(
        address_type: WalletAddressType,
        account: u32,
    ) -> DescriptorPair {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let external = match address_type {
            WalletAddressType::NativeSegwit => {
                format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/0/*)")
            }
            WalletAddressType::WrappedSegwit => {
                format!("sh(wpkh([817e7be0/49h/0h/{account}h]{xpub}/0/*))")
            }
            WalletAddressType::Legacy => {
                format!("pkh([817e7be0/44h/0h/{account}h]{xpub}/0/*)")
            }
        };
        let internal = match address_type {
            WalletAddressType::NativeSegwit => {
                format!("wpkh([817e7be0/84h/0h/{account}h]{xpub}/1/*)")
            }
            WalletAddressType::WrappedSegwit => {
                format!("sh(wpkh([817e7be0/49h/0h/{account}h]{xpub}/1/*))")
            }
            WalletAddressType::Legacy => {
                format!("pkh([817e7be0/44h/0h/{account}h]{xpub}/1/*)")
            }
        };

        DescriptorPair { external, internal }
    }

    fn account_xpub(account: u32) -> (BdkFingerprint, Xpub) {
        account_xpub_for_address_type(WalletAddressType::NativeSegwit, account)
    }

    fn account_xpub_for_address_type(
        address_type: WalletAddressType,
        account: u32,
    ) -> (BdkFingerprint, Xpub) {
        let mnemonic = bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let seed = mnemonic.to_seed("");
        let secp = bdk_wallet::bitcoin::secp256k1::Secp256k1::new();
        let master = bdk_wallet::bitcoin::bip32::Xpriv::new_master(
            bdk_wallet::bitcoin::Network::Bitcoin,
            &seed,
        )
        .unwrap();
        let purpose = match address_type {
            WalletAddressType::NativeSegwit => 84,
            WalletAddressType::WrappedSegwit => 49,
            WalletAddressType::Legacy => 44,
        };
        let path = bdk_wallet::bitcoin::bip32::DerivationPath::from_str(&format!(
            "m/{purpose}h/0h/{account}h"
        ))
        .unwrap();
        let account_key = master.derive_priv(&secp, &path).unwrap();

        (master.fingerprint(&secp), Xpub::from_priv(&secp, &account_key))
    }

    fn account_descriptor_pair(account: u32) -> Descriptors {
        let (fingerprint, xpub) = account_xpub(account);
        let descriptor = format!("wpkh([{fingerprint}/84h/0h/{account}h]{xpub}/<0;1>/*)");

        pubport::descriptor::Descriptors::try_from_line(&descriptor).unwrap().into()
    }

    fn metadata(name: &str, wallet_type: WalletType) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::new();
        metadata.name = name.to_string();
        metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(
            bdk_wallet::bitcoin::bip32::Fingerprint::from_str("817e7be0").unwrap(),
        )));
        metadata.wallet_type = wallet_type;
        metadata
    }

    fn backup(metadata: &WalletMetadata, descriptors: Option<DescriptorPair>) -> WalletBackup {
        WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret: WalletSecret::None,
            descriptors,
            xpub: None,
            labels_jsonl: None,
        }
    }

    fn public_wallet_metadata(name: &str, account: u32) -> WalletMetadata {
        let descriptors = descriptor_pair(account);
        let fingerprint = Fingerprint::from(
            descriptors.fingerprint().expect("test descriptor has a fingerprint"),
        );

        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::new();
        metadata.name = name.to_string();
        metadata.master_fingerprint = Some(Arc::new(fingerprint));
        metadata.verified = true;
        metadata.wallet_type = WalletType::Cold;
        metadata
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
    fn xpub_default_address_type_synthesizes_native_segwit_descriptor_identity() {
        let descriptors = descriptor_pair(0);
        let xpub = descriptors.external.xpub().unwrap();
        let fingerprint = Fingerprint::from(BdkFingerprint::from_str("817e7be0").unwrap());

        let identity = PublicWalletIdentity::from_xpub_default_address_type(
            xpub,
            fingerprint,
            Network::Bitcoin,
            WalletAddressType::NativeSegwit,
        )
        .unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }

    #[test]
    fn xpub_default_address_type_synthesizes_wrapped_segwit_descriptor_identity() {
        let descriptors = descriptor_pair_for_address_type(WalletAddressType::WrappedSegwit, 0);
        let xpub = descriptors.external.xpub().unwrap();
        let fingerprint = Fingerprint::from(BdkFingerprint::from_str("817e7be0").unwrap());

        let identity = PublicWalletIdentity::from_xpub_default_address_type(
            xpub,
            fingerprint,
            Network::Bitcoin,
            WalletAddressType::WrappedSegwit,
        )
        .unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }

    #[test]
    fn xpub_default_address_type_synthesizes_legacy_descriptor_identity() {
        let descriptors = descriptor_pair_for_address_type(WalletAddressType::Legacy, 0);
        let xpub = descriptors.external.xpub().unwrap();
        let fingerprint = Fingerprint::from(BdkFingerprint::from_str("817e7be0").unwrap());

        let identity = PublicWalletIdentity::from_xpub_default_address_type(
            xpub,
            fingerprint,
            Network::Bitcoin,
            WalletAddressType::Legacy,
        )
        .unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }

    #[test]
    fn xpub_default_bip84_uses_account_from_account_xpub() {
        let descriptors = account_descriptor_pair(1);
        let xpub = descriptors.external.xpub().unwrap();
        let fingerprint =
            Fingerprint::from(descriptors.fingerprint().expect("test descriptor has fingerprint"));

        let identity = PublicWalletIdentity::from_xpub_default_address_type(
            xpub,
            fingerprint,
            Network::Bitcoin,
            WalletAddressType::NativeSegwit,
        )
        .unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }

    #[test]
    fn cold_existing_wallet_xpub_synthesizes_default_bip84_identity() {
        let keychain = test_keychain();
        let descriptors = descriptor_pair(0);
        let xpub = descriptors.external.xpub().unwrap();
        let metadata = metadata("Existing cold xpub", WalletType::Cold);

        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();

        let identity =
            PublicWalletIdentity::from_existing_wallet(&metadata, keychain).unwrap().unwrap();

        assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
    }

    #[test]
    fn cold_existing_wallet_xpub_preserves_address_type_identity() {
        let keychain = test_keychain();

        for address_type in [WalletAddressType::WrappedSegwit, WalletAddressType::Legacy] {
            let descriptors = descriptor_pair_for_address_type(address_type, 0);
            let xpub = descriptors.external.xpub().unwrap();
            let mut metadata = metadata("Existing typed xpub", WalletType::Cold);
            metadata.address_type = address_type;

            keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();

            let identity =
                PublicWalletIdentity::from_existing_wallet(&metadata, keychain).unwrap().unwrap();

            assert_eq!(PublicWalletIdentity::from_descriptors(&descriptors), identity);
        }
    }

    #[test]
    fn backup_duplicate_key_allows_same_fingerprint_different_public_identity() {
        let existing_metadata = metadata("Existing account 0", WalletType::Cold);
        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_metadata = metadata("Incoming account 1", WalletType::Cold);
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(1)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(!existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_skips_same_public_identity_with_different_name() {
        let existing_metadata = metadata("Existing name", WalletType::Cold);
        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_metadata = metadata("Incoming renamed", WalletType::Cold);
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(0)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_matches_degraded_existing_fingerprint() {
        let existing_metadata = metadata("Existing degraded", WalletType::Cold);
        let existing_backup = backup(&existing_metadata, None);
        let incoming_metadata = metadata("Incoming restored", WalletType::Cold);
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(0)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_matches_degraded_incoming_fingerprint() {
        let existing_metadata = metadata("Existing public", WalletType::Cold);
        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_metadata = metadata("Incoming degraded", WalletType::Cold);
        let incoming_backup = backup(&incoming_metadata, None);

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_preserves_hot_wallet_fingerprint_fallback() {
        let existing_metadata = metadata("Existing hot", WalletType::Hot);
        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_metadata = metadata("Incoming hot account 1", WalletType::Hot);
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(1)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_uses_wallet_id_when_no_identity_or_fingerprint() {
        let mut existing_metadata = WalletMetadata::preview_new();
        existing_metadata.master_fingerprint = None;
        existing_metadata.wallet_type = WalletType::WatchOnly;

        let mut incoming_metadata = existing_metadata.clone();
        incoming_metadata.name = "Renamed no identity".to_string();

        let existing_backup = backup(&existing_metadata, None);
        let incoming_backup = backup(&incoming_metadata, None);

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_matches_no_fingerprint_wallet_id_with_different_public_identity() {
        let mut existing_metadata = metadata("Existing no fingerprint account 0", WalletType::Cold);
        existing_metadata.master_fingerprint = None;
        let mut incoming_metadata = existing_metadata.clone();
        incoming_metadata.name = "Incoming no fingerprint account 1".to_string();

        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(1)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_matches_no_fingerprint_public_identity_to_wallet_id() {
        let mut existing_metadata = metadata("Existing no fingerprint public", WalletType::Cold);
        existing_metadata.master_fingerprint = None;
        let mut incoming_metadata = existing_metadata.clone();
        incoming_metadata.name = "Incoming no fingerprint degraded".to_string();

        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_backup = backup(&incoming_metadata, None);

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_matches_no_fingerprint_wallet_id_to_public_identity() {
        let mut existing_metadata = metadata("Existing no fingerprint degraded", WalletType::Cold);
        existing_metadata.master_fingerprint = None;
        let mut incoming_metadata = existing_metadata.clone();
        incoming_metadata.name = "Incoming no fingerprint public".to_string();

        let existing_backup = backup(&existing_metadata, None);
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(0)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(existing.contains(&incoming_key));
    }

    #[test]
    fn backup_duplicate_key_allows_no_fingerprint_different_id_and_public_identity() {
        let mut existing_metadata = metadata("Existing no fingerprint account 0", WalletType::Cold);
        existing_metadata.master_fingerprint = None;
        let mut incoming_metadata = metadata("Incoming no fingerprint account 1", WalletType::Cold);
        incoming_metadata.master_fingerprint = None;

        let existing_backup = backup(&existing_metadata, Some(descriptors(0)));
        let incoming_backup = backup(&incoming_metadata, Some(descriptors(1)));

        let mut existing = ExistingWalletIdentitySet::default();
        existing.insert(identity_key_for_backup(&existing_metadata, &existing_backup).unwrap());

        let incoming_key = identity_key_for_backup(&incoming_metadata, &incoming_backup).unwrap();

        assert!(!existing.contains(&incoming_key));
    }

    #[test]
    fn public_wallet_identity_matching_skips_same_fingerprint_different_account() {
        let keychain = test_keychain();
        let existing = public_wallet_metadata("Existing account 0", 0);
        let incoming = descriptor_pair(1);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        keychain
            .save_public_descriptor(
                &existing.id,
                descriptor_pair(0).external.extended_descriptor,
                descriptor_pair(0).internal.extended_descriptor,
            )
            .unwrap();

        let matched = matching_public_wallet_by_identity(
            vec![existing],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            true,
        )
        .unwrap();

        assert!(matched.is_none());
    }

    #[test]
    fn public_wallet_identity_matching_routes_exact_identity() {
        let keychain = test_keychain();
        let existing = public_wallet_metadata("Existing account 0", 0);
        let incoming = descriptor_pair(0);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        keychain
            .save_public_descriptor(
                &existing.id,
                descriptor_pair(0).external.extended_descriptor,
                descriptor_pair(0).internal.extended_descriptor,
            )
            .unwrap();

        let matched = matching_public_wallet_by_identity(
            vec![existing.clone()],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            true,
        )
        .unwrap();

        assert_eq!(Some(existing.id), matched.map(|metadata| metadata.id));
    }

    #[test]
    fn public_wallet_identity_matching_falls_back_to_degraded_same_fingerprint() {
        let keychain = test_keychain();
        let degraded = public_wallet_metadata("Degraded account", 0);
        let expected_id = degraded.id.clone();
        let incoming = descriptor_pair(1);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        let matched = matching_public_wallet_by_identity(
            vec![degraded],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            true,
        )
        .unwrap();

        assert_eq!(Some(expected_id), matched.map(|metadata| metadata.id));
    }

    #[test]
    fn strict_public_wallet_identity_matching_routes_exact_identity() {
        let keychain = test_keychain();
        let existing = public_wallet_metadata("Existing account 0", 0);
        let incoming = descriptor_pair(0);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        keychain
            .save_public_descriptor(
                &existing.id,
                descriptor_pair(0).external.extended_descriptor,
                descriptor_pair(0).internal.extended_descriptor,
            )
            .unwrap();

        let matched = matching_public_wallet_by_identity(
            vec![existing.clone()],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            false,
        )
        .unwrap();

        assert_eq!(Some(existing.id), matched.map(|metadata| metadata.id));
    }

    #[test]
    fn strict_public_wallet_identity_matching_skips_same_fingerprint_different_account() {
        let keychain = test_keychain();
        let existing = public_wallet_metadata("Existing account 0", 0);
        let incoming = descriptor_pair(1);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        keychain
            .save_public_descriptor(
                &existing.id,
                descriptor_pair(0).external.extended_descriptor,
                descriptor_pair(0).internal.extended_descriptor,
            )
            .unwrap();

        let matched = matching_public_wallet_by_identity(
            vec![existing],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            false,
        )
        .unwrap();

        assert!(matched.is_none());
    }

    #[test]
    fn strict_public_wallet_identity_matching_errors_on_degraded_same_fingerprint() {
        let keychain = test_keychain();
        let degraded = public_wallet_metadata("Degraded account", 0);
        let expected_id = degraded.id.clone();
        let incoming = descriptor_pair(1);
        let incoming_fingerprint =
            Fingerprint::from(incoming.fingerprint().expect("test descriptor has a fingerprint"));

        let error = matching_public_wallet_by_identity(
            vec![degraded],
            keychain,
            incoming_fingerprint,
            &PublicWalletIdentity::from_descriptors(&incoming),
            false,
        )
        .unwrap_err();

        match error {
            WalletIdentityError::MissingExistingWalletPublicIdentity { wallet_id } => {
                assert_eq!(expected_id, wallet_id);
            }
            error => panic!("unexpected error: {error}"),
        }
    }
}

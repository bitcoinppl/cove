use std::collections::HashSet;
use std::str::FromStr as _;

use bdk_wallet::bitcoin::bip32::{ChildNumber, Fingerprint as BdkFingerprint, Xpub};
use bdk_wallet::descriptor::ExtendedDescriptor;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_device::keychain::{Keychain, KeychainError};
use cove_util::{result_ext::ResultExt as _, sha256_hash};
use strum::IntoEnumIterator as _;
use tracing::warn;

use crate::backup::model::WalletBackup;
use crate::database::{self, Database};
use crate::keys::{self, Descriptors};
use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::{
    WalletAddressType,
    metadata::{WalletId, WalletMetadata, WalletMode, WalletType},
};

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
        let account = account_from_xpub(&xpub);
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

fn account_from_xpub(xpub: &Xpub) -> u32 {
    match (xpub.depth, xpub.child_number) {
        (3, ChildNumber::Hardened { index }) => index,
        _ => 0,
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) enum WalletIdentityKey {
    PublicIdentity {
        identity: PublicWalletIdentity,
        fingerprint: Option<Fingerprint>,
        wallet_id: Option<WalletId>,
        network: Network,
        mode: WalletMode,
    },
    Fingerprint {
        fingerprint: Fingerprint,
        network: Network,
        mode: WalletMode,
    },
    WalletId {
        id: WalletId,
        network: Network,
        mode: WalletMode,
    },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExistingWalletIdentitySet {
    public_identities: HashSet<(PublicWalletIdentity, Network, WalletMode)>,
    public_identity_fingerprints: HashSet<(Fingerprint, Network, WalletMode)>,
    fingerprints: HashSet<(Fingerprint, Network, WalletMode)>,
    wallet_ids: HashSet<(WalletId, Network, WalletMode)>,
}

impl ExistingWalletIdentitySet {
    pub(crate) fn contains(&self, key: &WalletIdentityKey) -> bool {
        match key {
            WalletIdentityKey::PublicIdentity {
                identity,
                fingerprint,
                wallet_id,
                network,
                mode,
            } => {
                self.public_identities.contains(&(identity.clone(), *network, *mode))
                    || fingerprint.is_some_and(|fingerprint| {
                        self.fingerprints.contains(&(fingerprint, *network, *mode))
                    })
                    || wallet_id
                        .as_ref()
                        .is_some_and(|id| self.wallet_ids.contains(&(id.clone(), *network, *mode)))
            }
            WalletIdentityKey::Fingerprint { fingerprint, network, mode } => {
                self.fingerprints.contains(&(*fingerprint, *network, *mode))
                    || self.public_identity_fingerprints.contains(&(*fingerprint, *network, *mode))
            }
            WalletIdentityKey::WalletId { id, network, mode } => {
                self.wallet_ids.contains(&(id.clone(), *network, *mode))
            }
        }
    }

    pub(crate) fn insert(&mut self, key: WalletIdentityKey) {
        match key {
            WalletIdentityKey::PublicIdentity {
                identity,
                fingerprint,
                wallet_id,
                network,
                mode,
            } => {
                self.public_identities.insert((identity, network, mode));

                if let Some(fingerprint) = fingerprint {
                    self.public_identity_fingerprints.insert((fingerprint, network, mode));
                }

                if let Some(wallet_id) = wallet_id {
                    self.wallet_ids.insert((wallet_id, network, mode));
                }
            }
            WalletIdentityKey::Fingerprint { fingerprint, network, mode } => {
                self.fingerprints.insert((fingerprint, network, mode));
            }
            WalletIdentityKey::WalletId { id, network, mode } => {
                self.wallet_ids.insert((id, network, mode));
            }
        }
    }
}

pub(crate) fn identity_key_for_backup(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<WalletIdentityKey, WalletIdentityError> {
    if metadata.wallet_type == WalletType::Hot
        && let Some(fingerprint) = metadata.master_fingerprint.as_deref().copied()
    {
        return Ok(WalletIdentityKey::Fingerprint {
            fingerprint,
            network: metadata.network,
            mode: metadata.wallet_mode,
        });
    }

    if let Some(identity) = public_identity_from_backup(metadata, backup)? {
        return Ok(WalletIdentityKey::PublicIdentity {
            identity,
            fingerprint: metadata.master_fingerprint.as_deref().copied(),
            wallet_id: no_fingerprint_wallet_id(metadata),
            network: metadata.network,
            mode: metadata.wallet_mode,
        });
    }

    if let Some(fingerprint) = metadata.master_fingerprint.as_deref().copied() {
        return Ok(WalletIdentityKey::Fingerprint {
            fingerprint,
            network: metadata.network,
            mode: metadata.wallet_mode,
        });
    }

    Ok(WalletIdentityKey::WalletId {
        id: metadata.id.clone(),
        network: metadata.network,
        mode: metadata.wallet_mode,
    })
}

fn no_fingerprint_wallet_id(metadata: &WalletMetadata) -> Option<WalletId> {
    metadata.master_fingerprint.is_none().then(|| metadata.id.clone())
}

pub(crate) fn collect_existing_wallet_identities()
-> Result<ExistingWalletIdentitySet, WalletIdentityError> {
    let db = Database::global();
    let keychain = Keychain::global();
    let mut identities = ExistingWalletIdentitySet::default();

    for network in Network::iter() {
        for mode in [WalletMode::Main, WalletMode::Decoy] {
            let wallets = db.wallets.get_all(network, mode)?;

            for wallet in wallets {
                let duplicate_key = existing_wallet_identity_key(wallet, keychain)?;
                identities.insert(duplicate_key);
            }
        }
    }

    Ok(identities)
}

fn existing_wallet_identity_key(
    metadata: WalletMetadata,
    keychain: &Keychain,
) -> Result<WalletIdentityKey, WalletIdentityError> {
    if metadata.wallet_type != WalletType::Hot {
        let identity =
            PublicWalletIdentity::from_existing_wallet(&metadata, keychain).map_err(|source| {
                WalletIdentityError::ExistingWalletPublicIdentity {
                    wallet_id: metadata.id.clone(),
                    source,
                }
            })?;

        if let Some(identity) = identity {
            return Ok(WalletIdentityKey::PublicIdentity {
                identity,
                fingerprint: metadata.master_fingerprint.as_deref().copied(),
                wallet_id: no_fingerprint_wallet_id(&metadata),
                network: metadata.network,
                mode: metadata.wallet_mode,
            });
        }
    }

    if let Some(fingerprint) = metadata.master_fingerprint.as_deref().copied() {
        return Ok(WalletIdentityKey::Fingerprint {
            fingerprint,
            network: metadata.network,
            mode: metadata.wallet_mode,
        });
    }

    Ok(WalletIdentityKey::WalletId {
        id: metadata.id,
        network: metadata.network,
        mode: metadata.wallet_mode,
    })
}

fn public_identity_from_backup(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<Option<PublicWalletIdentity>, WalletIdentityError> {
    if let Some(descriptors) = &backup.descriptors {
        let identity = PublicWalletIdentity::from_descriptor_strs(
            &descriptors.external,
            &descriptors.internal,
        )
        .map_err(|source| WalletIdentityError::BackupDescriptor {
            wallet_name: metadata.name.clone(),
            source,
        })?;

        return Ok(Some(identity));
    }

    if let Some(xpub) = &backup.xpub {
        let fingerprint = metadata.master_fingerprint.as_deref().copied();
        let identity = PublicWalletIdentity::from_xpub_str_default_address_type(
            xpub,
            fingerprint,
            metadata.network,
            metadata.address_type,
        )
        .map_err(|source| WalletIdentityError::BackupXpub {
            wallet_name: metadata.name.clone(),
            source,
        })?;

        return Ok(Some(identity));
    }

    Ok(None)
}

pub(crate) fn existing_public_wallet_by_identity_strict(
    database: &Database,
    keychain: &Keychain,
    network: Network,
    mode: WalletMode,
    fingerprint: Fingerprint,
    incoming_identity: &PublicWalletIdentity,
) -> Result<Option<WalletMetadata>, WalletIdentityError> {
    let wallets = database.wallets.get_all(network, mode)?;

    matching_public_wallet_by_identity(wallets, keychain, fingerprint, incoming_identity, false)
}

fn matching_public_wallet_by_identity(
    wallets: Vec<WalletMetadata>,
    keychain: &Keychain,
    fingerprint: Fingerprint,
    incoming_identity: &PublicWalletIdentity,
    allow_degraded_fingerprint_match: bool,
) -> Result<Option<WalletMetadata>, WalletIdentityError> {
    let mut degraded_same_fingerprint_wallet = None;

    for wallet_metadata in wallets {
        if !wallet_metadata.matches_fingerprint(fingerprint) {
            continue;
        }

        let wallet_identity =
            PublicWalletIdentity::from_existing_wallet(&wallet_metadata, keychain).map_err(
                |source| WalletIdentityError::ExistingWalletPublicIdentity {
                    wallet_id: wallet_metadata.id.clone(),
                    source,
                },
            )?;

        let Some(wallet_identity) = wallet_identity else {
            degraded_same_fingerprint_wallet.get_or_insert(wallet_metadata);
            continue;
        };

        if &wallet_identity == incoming_identity {
            return Ok(Some(wallet_metadata));
        }
    }

    let Some(wallet_metadata) = degraded_same_fingerprint_wallet else {
        return Ok(None);
    };

    if !allow_degraded_fingerprint_match {
        return Err(WalletIdentityError::MissingExistingWalletPublicIdentity {
            wallet_id: wallet_metadata.id,
        });
    }

    let wallet_id = wallet_metadata.id.clone();
    let incoming_identity_hash = incoming_identity.redacted_hash();
    warn!(
        "same-fingerprint wallet missing public identity wallet_id={wallet_id} incoming_identity_hash={incoming_identity_hash}, falling back to fingerprint match"
    );

    Ok(Some(wallet_metadata))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::ExistingWalletIdentitySet;

    pub(crate) trait ExistingWalletIdentitySetTestExt {
        fn len(&self) -> usize;
    }

    impl ExistingWalletIdentitySetTestExt for ExistingWalletIdentitySet {
        fn len(&self) -> usize {
            self.public_identities.len() + self.fingerprints.len() + self.wallet_ids.len()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Once};

    use cove_device::keychain::{KeychainAccess, KeychainError};

    use super::*;
    use crate::backup::model::{DescriptorPair, WalletSecret};

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

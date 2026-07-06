use std::sync::Arc;

use bdk_wallet::KeychainKind;
use bdk_wallet::bitcoin::bip32::Xpub;
use bdk_wallet::miniscript::descriptor::ShInner;
use bip39::Mnemonic;
use cove_types::Network;
use cove_util::result_ext::ResultExt as _;
use parking_lot::Mutex;
use pubport::formats::Format;
use tracing::{error, warn};

use crate::{
    app::reconcile::{Update, Updater},
    bdk_store::BdkStore,
    database::Database,
    keychain::Keychain,
    keys::{Descriptor, Descriptors},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    mnemonic::MnemonicExt as _,
    tap_card::tap_signer_reader::DeriveInfo,
    wallet_identity::{PublicWalletIdentity, existing_public_wallet_by_identity_strict},
    xpub::{self, XpubError},
};

use super::{
    Wallet, WalletAddressType, WalletError, delete_wallet_specific_data,
    fingerprint::Fingerprint,
    metadata,
    metadata::{
        DiscoveryState, HardwareWalletMetadata, WalletBirthday, WalletId, WalletMetadata,
        WalletType, tap_signer_import_birthday,
    },
};

pub(crate) struct WalletBuilder {
    source: WalletSource,
}

pub(crate) enum WalletSource {
    PersistedAndSelected {
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    },
    Xpub(String),
    Pubport(Box<pubport::Format>),
    TapSigner {
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive: DeriveInfo,
        backup: Option<Vec<u8>>,
        birthday: Option<WalletBirthday>,
    },
    Mnemonic {
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
        address_type: WalletAddressType,
    },
}

impl WalletBuilder {
    pub(crate) fn new(source: WalletSource) -> Self {
        Self { source }
    }

    pub(crate) fn build(self) -> Result<Wallet, WalletError> {
        match self.source {
            WalletSource::PersistedAndSelected { metadata, mnemonic, passphrase } => {
                Self::build_persisted_and_selected(metadata, mnemonic, passphrase)
            }
            WalletSource::Xpub(xpub) => Self::build_from_xpub(xpub),
            WalletSource::Pubport(pubport) => Self::build_from_pubport(*pubport),
            WalletSource::TapSigner { tap_signer, derive, backup, birthday } => {
                Self::build_from_tap_signer(tap_signer, derive, backup, birthday)
            }
            WalletSource::Mnemonic { metadata, mnemonic, passphrase, address_type } => {
                Self::build_from_mnemonic(metadata, mnemonic, passphrase, address_type)
            }
        }
    }

    fn build_persisted_and_selected(
        metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
    ) -> Result<Wallet, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();

        let create_wallet = || -> Result<Wallet, WalletError> {
            // create bdk wallet filestore, set id to metadata id
            let me = Self::build_from_mnemonic(
                metadata.clone(),
                mnemonic.clone(),
                passphrase,
                WalletAddressType::NativeSegwit,
            )?;

            // save mnemonic for private key
            keychain.save_wallet_key(&me.id, mnemonic.clone())?;

            // save public key in keychain too
            let xpub = mnemonic.xpub(me.network.into());
            keychain.save_wallet_xpub(&me.id, xpub)?;

            let (external_descriptor, internal_descriptor) = {
                let external_descriptor = me.bdk.public_descriptor(KeychainKind::External);
                let internal_descriptor = me.bdk.public_descriptor(KeychainKind::Internal);

                (external_descriptor.clone(), internal_descriptor.clone())
            };

            // save public descriptors in keychain too
            keychain.save_public_descriptor(&me.id, external_descriptor, internal_descriptor)?;

            // save wallet_metadata to database
            database.wallets.save_new_wallet_metadata(me.metadata.clone())?;

            // set this wallet as the selected wallet
            database.global_config.select_wallet(me.id.clone())?;

            Ok(me)
        };

        // clean up if we fail to create the wallet
        let me = match create_wallet() {
            Ok(me) => me,
            Err(error) => {
                error!("failed to create wallet: {error}");

                // delete the secret key, xpub and public descriptor from the keychain
                keychain.delete_wallet_items(&metadata.id);

                if let Err(error) = delete_wallet_specific_data(&metadata.id) {
                    warn!("clean up failed, failed to delete wallet data: {error}");
                }

                if let Err(error) = database.wallets.delete(&metadata.id) {
                    warn!("clean up failed, failed to delete wallet: {error}");
                }

                if let Err(error) = database.global_config.clear_selected_wallet() {
                    warn!("clean up failed, failed to clear selected wallet: {error}");
                }

                return Err(error);
            }
        };

        Ok(me)
    }

    fn build_from_xpub(xpub: String) -> Result<Wallet, WalletError> {
        let xpub = xpub.trim();
        let hardware_export = pubport::Format::try_new_from_str(xpub)
            .map_err(Into::into)
            .map_err(WalletError::ParseXpubError);

        if let Ok(hardware_export) = hardware_export {
            return Self::new(WalletSource::Pubport(Box::new(hardware_export))).build();
        }

        // already returned if its a valid xpub
        Err(hardware_export.unwrap_err())
    }

    fn build_from_pubport(pubport: pubport::Format) -> Result<Wallet, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();
        let network = database.global_config.selected_network();
        let mode = database.global_config.wallet_mode();

        let id = WalletId::new();
        let mut metadata = WalletMetadata::new_for_hardware(id.clone(), "", None);

        let pubport_descriptors = match pubport {
            Format::Descriptor(descriptors) => descriptors,
            Format::Json(json) => {
                let (descriptors, address_type) = preferred_json_descriptors(&json)?;

                if should_start_json_discovery(&json, address_type) {
                    metadata.discovery_state =
                        DiscoveryState::StartedJson(Arc::new((*json).into()));
                }

                descriptors
            }
            Format::Wasabi(descriptors) => descriptors,
            Format::Electrum(descriptors) => descriptors,
            Format::KeyExpression(descriptors) => descriptors,
        };

        // compute xpub and descriptors early so they're available for the upgrade path
        let descriptors: Descriptors = pubport_descriptors.into();
        metadata.address_type = address_type_from_descriptors(&descriptors)?;
        let fingerprint = descriptors.fingerprint();
        let xpub = xpub_from_descriptors(&descriptors)?;

        let incoming_identity = PublicWalletIdentity::from_descriptors(&descriptors);

        // check for existing wallet with same public identity, upgrade watch-only to cold
        if let Some(fingerprint) = fingerprint.as_ref() {
            let fingerprint: Fingerprint = (*fingerprint).into();
            metadata.master_fingerprint = Some(fingerprint.into());

            let existing = existing_public_wallet_by_identity_strict(
                &database,
                keychain,
                network,
                mode,
                fingerprint,
                &incoming_identity,
            )
            .map_err_str(WalletError::LoadError)?;

            if let Some(existing_metadata) = existing {
                return Self::upgrade_to_cold(
                    existing_metadata,
                    &metadata,
                    xpub,
                    descriptors,
                    keychain,
                    &database,
                );
            }
        }

        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let fingerprint = fingerprint.map(|s| s.to_string());

        metadata.name = match &fingerprint {
            Some(fingerprint) => format!("Imported {}", fingerprint.to_ascii_uppercase()),
            None => "Imported XPub".to_string(),
        };

        metadata.wallet_type = match &fingerprint {
            Some(_) => WalletType::Cold,
            None => WalletType::XpubOnly,
        };

        // get origin only if its not a watch only wallet
        match metadata.wallet_type {
            WalletType::Hot | WalletType::Cold => {
                metadata.origin = descriptors.origin().ok();
            }
            _ => {}
        }

        let wallet = descriptors
            .clone()
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        // save public key in keychain too
        keychain.save_wallet_xpub(&id, xpub)?;

        // save public descriptor in keychain too
        keychain.save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        database.wallets.save_new_wallet_metadata(metadata.clone())?;
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

        Ok(Wallet { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    fn build_from_tap_signer(
        tap_signer: Arc<cove_tap_card::TapSigner>,
        derive: DeriveInfo,
        backup: Option<Vec<u8>>,
        birthday: Option<WalletBirthday>,
    ) -> Result<Wallet, WalletError> {
        let keychain = Keychain::global();
        let database = Database::global();
        let mode = database.global_config.wallet_mode();
        let network = database.global_config.selected_network();
        assert!(network == derive.network);

        let id = WalletId::new();

        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let descriptors = Descriptors::new_from_tap_signer(&derive)
            .map_err_str(WalletError::DescriptorKeyParseError)?;

        let fingerprint = Fingerprint::from(derive.master_fingerprint());

        // set metadata
        let mut metadata = WalletMetadata::new_for_hardware(id.clone(), "", None);
        metadata.name = "TAPSIGNER".to_string();
        metadata.wallet_mode = mode;
        metadata.hardware_metadata = Some(HardwareWalletMetadata::TapSigner(tap_signer));
        metadata.origin = descriptors.origin().ok();
        metadata.master_fingerprint = Some(Arc::new(fingerprint));
        metadata.wallet_type = WalletType::Cold;
        metadata.birthday =
            birthday.or_else(|| tap_signer_import_birthday(network, derive.birth_height));

        // make sure its not already imported
        check_for_duplicate_wallet(network, mode, fingerprint)?;

        let xpub =
            descriptors.external.xpub().expect("tap_signer descriptor always made with xpub");

        let wallet = descriptors
            .clone()
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        // save public key in keychain too
        keychain.save_wallet_xpub(&id, xpub)?;

        // save public descriptor in keychain too
        keychain.save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        // if theres a backup for this wallet, save it in the keychain
        if let Some(backup) = backup {
            keychain.save_tap_signer_backup(&id, backup.as_slice())?;
        }

        database.wallets.save_new_wallet_metadata(metadata.clone())?;
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

        Ok(Wallet { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    fn build_from_mnemonic(
        mut metadata: WalletMetadata,
        mnemonic: Mnemonic,
        passphrase: Option<String>,
        address_type: WalletAddressType,
    ) -> Result<Wallet, WalletError> {
        let network = Database::global().global_config.selected_network();

        let id = metadata.id.clone();
        let mut store = BdkStore::try_new(&id, network).map_err_str(WalletError::LoadError)?;

        let descriptors = mnemonic.into_descriptors(passphrase, network, address_type);
        let origin = descriptors.origin().ok();

        metadata.master_fingerprint = descriptors.fingerprint().map(|f| Arc::new(f.into()));
        metadata.origin = origin;

        let wallet = descriptors
            .into_create_params()
            .network(network.into())
            .create_wallet(&mut store.conn)
            .map_err_str(WalletError::BdkError)?;

        Ok(Wallet { id, metadata, network, bdk: wallet, db: Mutex::new(store.conn) })
    }

    fn upgrade_to_cold(
        mut metadata: WalletMetadata,
        import_metadata: &WalletMetadata,
        xpub: Xpub,
        descriptors: Descriptors,
        keychain: &Keychain,
        database: &Database,
    ) -> Result<Wallet, WalletError> {
        if metadata.wallet_type != WalletType::WatchOnly {
            return Err(WalletError::WalletAlreadyExists(metadata.id));
        }

        let id = metadata.id.clone();
        keychain.save_wallet_xpub(&id, xpub)?;

        metadata = metadata_for_cold_upgrade(metadata, import_metadata, &descriptors);

        keychain.save_public_descriptor(
            &id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )?;

        database.wallets.update_wallet_metadata(metadata)?;
        database.global_config.select_wallet(id.clone())?;

        Updater::send_update(Update::ClearCachedWalletManager(id.clone()));
        CLOUD_BACKUP_MANAGER.handle_wallet_backup_change_and_reverify(id.clone());
        Wallet::try_load_persisted(id)
    }
}

fn check_for_duplicate_wallet(
    network: Network,
    mode: metadata::WalletMode,
    fingerprint: Fingerprint,
) -> Result<(), WalletError> {
    let all_fingerprints: Vec<(WalletId, Arc<Fingerprint>)> = Database::global()
        .wallets
        .get_all(network, mode)
        .map(|wallets| {
            wallets
                .into_iter()
                .filter_map(|wallet_metadata| {
                    let fingerprint = wallet_metadata.master_fingerprint?;
                    Some((wallet_metadata.id, fingerprint))
                })
                .collect()
        })
        .unwrap_or_default();

    if let Some((id, _)) = all_fingerprints.into_iter().find(|(_, f)| f.as_ref() == &fingerprint) {
        return Err(WalletError::WalletAlreadyExists(id));
    }

    Ok(())
}

fn metadata_for_cold_upgrade(
    mut existing_metadata: WalletMetadata,
    import_metadata: &WalletMetadata,
    descriptors: &Descriptors,
) -> WalletMetadata {
    existing_metadata.wallet_type = WalletType::Cold;
    existing_metadata.origin = descriptors.origin().ok();
    existing_metadata.address_type = import_metadata.address_type;

    if import_metadata.discovery_state != DiscoveryState::Single {
        existing_metadata.discovery_state = import_metadata.discovery_state.clone();
    }

    existing_metadata
}

/// Selects the JSON descriptor set that becomes the imported wallet
///
/// A JSON export can carry several standard account types. Cove imports the most modern
/// supported primary wallet and lets discovery scan any supported older alternates separately
fn preferred_json_descriptors(
    json: &pubport::formats::Json,
) -> Result<(pubport::descriptor::Descriptors, WalletAddressType), WalletError> {
    [
        (&json.bip84, WalletAddressType::NativeSegwit),
        (&json.bip49, WalletAddressType::WrappedSegwit),
        (&json.bip44, WalletAddressType::Legacy),
    ]
    .into_iter()
    .find_map(|(descriptors, address_type)| {
        descriptors.as_ref().map(|descriptors| (descriptors.clone(), address_type))
    })
    .ok_or_else(|| {
        WalletError::ParseXpubError(xpub::XpubError::MissingXpub(
            "No supported BIP44, BIP49, or BIP84 xpub found".to_string(),
        ))
    })
}

fn xpub_from_descriptors(descriptors: &Descriptors) -> Result<Xpub, WalletError> {
    descriptors.external.xpub().ok_or(WalletError::ParseXpubError(XpubError::InvalidDescriptor(
        xpub::DescriptorError::NoXpubInDescriptor,
    )))
}

fn address_type_from_descriptors(
    descriptors: &Descriptors,
) -> Result<WalletAddressType, WalletError> {
    let external = address_type_from_descriptor(&descriptors.external)?;
    let internal = address_type_from_descriptor(&descriptors.internal)?;

    if external != internal {
        return Err(WalletError::UnsupportedWallet(
            "external and internal descriptors use different address types".to_string(),
        ));
    }

    Ok(external)
}

fn address_type_from_descriptor(descriptor: &Descriptor) -> Result<WalletAddressType, WalletError> {
    match &descriptor.extended_descriptor {
        bdk_wallet::miniscript::Descriptor::Pkh(_) => Ok(WalletAddressType::Legacy),
        bdk_wallet::miniscript::Descriptor::Wpkh(_) => Ok(WalletAddressType::NativeSegwit),
        bdk_wallet::miniscript::Descriptor::Sh(sh) => match sh.as_inner() {
            ShInner::Wpkh(_) => Ok(WalletAddressType::WrappedSegwit),
            _ => Err(unsupported_descriptor_address_type("non-wrapped-SegWit P2SH")),
        },
        bdk_wallet::miniscript::Descriptor::Tr(_) => {
            Err(unsupported_descriptor_address_type("Taproot"))
        }
        _ => Err(unsupported_descriptor_address_type("this descriptor type")),
    }
}

fn unsupported_descriptor_address_type(name: &str) -> WalletError {
    WalletError::UnsupportedWallet(format!("{name} descriptors are not supported"))
}

/// Returns whether a JSON import needs alternate address-type discovery
///
/// Discovery only starts when the export has a supported alternate that differs from the
/// primary wallet, because the primary descriptor is synced by the main wallet
///
/// Native SegWit (`bip84`) is intentionally absent because it is the preferred primary
/// descriptor when present, not an alternate discovery target
fn should_start_json_discovery(
    json: &pubport::formats::Json,
    address_type: WalletAddressType,
) -> bool {
    [(&json.bip49, WalletAddressType::WrappedSegwit), (&json.bip44, WalletAddressType::Legacy)]
        .into_iter()
        .any(|(descriptors, type_)| descriptors.is_some() && type_ != address_type)
}

#[cfg(test)]
mod tests {
    use bip39::Mnemonic;

    use super::*;

    const BIP49_YPUB: &str = "ypub6Ww3ibxVfGzLrAH1PNcjyAWenMTbbAosGNB6VvmSEgytSER9azLDWCxoJwW7Ke7icmizBMXrzBx9979FfaHxHcrArf3zbeJJJUZPf663zsP";
    const BIP84_ZPUB: &str = "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs";

    fn pubport_descriptors(descriptor: &str) -> pubport::descriptor::Descriptors {
        pubport::descriptor::Descriptors::try_from_line(descriptor)
            .expect("descriptor fixture is valid")
    }

    fn descriptor_json(
        bip44: Option<pubport::descriptor::Descriptors>,
        bip49: Option<pubport::descriptor::Descriptors>,
        bip84: Option<pubport::descriptor::Descriptors>,
    ) -> pubport::formats::Json {
        pubport::formats::Json { bip44, bip49, bip84, bip86: None }
    }

    fn json_from_export(export: &str) -> pubport::formats::Json {
        let format = pubport::Format::try_new_from_str(export).expect("export fixture is valid");
        let pubport::Format::Json(json) = format else {
            panic!("Expected JSON export");
        };

        *json
    }

    fn bip44_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "pkh([817e7be0/44h/0h/0h]xpub6BoKN14JzSFN1T3cqe9FnrwnXGAsmbgETJyeazoa3F7aMXh4XndvVrJAYyM127FsrH8KFv5XFXDroqXNfZMfsinow7xp93ueYSpnrjBBFs4/<0;1>/*)#tdtrl3y9",
        )
    }

    fn bip49_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "sh(wpkh([817e7be0/49h/0h/0h]xpub6CCKAvUTNursEnaJ8k1d27LfqEUzeAx2N9wFqYE3W1xh7nqgJEBEbLSSmohwDxzsSvcsYqiQqFzRvta65Njbe5o84bF5YXHFqfSH2Dkhonm/<0;1>/*))#8llmt36x",
        )
    }

    fn bip84_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7",
        )
    }

    fn bip86_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "tr([817e7be0/86h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)",
        )
    }

    fn assert_missing_supported_xpub(error: WalletError) {
        let WalletError::ParseXpubError(xpub::XpubError::MissingXpub(message)) = error else {
            panic!("expected missing xpub error");
        };

        assert_eq!(message, "No supported BIP44, BIP49, or BIP84 xpub found");
    }

    #[test]
    fn preferred_json_descriptors_uses_native_segwit_when_present() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(descriptors.external.to_string().starts_with("wpkh("));
    }

    #[test]
    fn preferred_json_descriptors_falls_back_to_wrapped_segwit() {
        let json = descriptor_json(Some(bip44_descriptors()), Some(bip49_descriptors()), None);

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(descriptors.external.to_string().starts_with("sh(wpkh("));
    }

    #[test]
    fn preferred_json_descriptors_falls_back_to_legacy() {
        let json = descriptor_json(Some(bip44_descriptors()), None, None);

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::Legacy);
        assert!(descriptors.external.to_string().starts_with("pkh("));
    }

    #[test]
    fn preferred_json_descriptors_errors_without_supported_xpub() {
        let json = descriptor_json(None, None, None);
        let error = preferred_json_descriptors(&json).unwrap_err();

        assert_missing_supported_xpub(error);
    }

    #[test]
    fn preferred_json_descriptors_rejects_bip86_only_export() {
        let json = json_from_export(
            r#"{
  "xfp": "817e7be0",
  "bip86": {
    "deriv": "m/86'/0'/0'",
    "xpub": "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    "first": "bc1p5cyxnuxmeuwuvkwfem96l9z7k3d5en0fhzc3wkvsgq4wv5q3xpqsv0gz6u"
  }
}"#,
        );
        let error = preferred_json_descriptors(&json).unwrap_err();

        assert_missing_supported_xpub(error);
    }

    #[test]
    fn bare_ypub_import_selects_wrapped_segwit_descriptor() {
        let json = json_from_export(BIP49_YPUB);

        assert!(json.bip49.is_some());
        assert!(json.bip84.is_none());

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(descriptors.external.to_string().starts_with("sh(wpkh("));
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn wrapped_segwit_json_import_material_exposes_nested_xpub() {
        let json = descriptor_json(None, Some(bip49_descriptors()), None);

        let (pubport_descriptors, address_type) = preferred_json_descriptors(&json).unwrap();
        let descriptors: Descriptors = pubport_descriptors.into();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert_eq!(
            descriptors.fingerprint().map(|fingerprint| fingerprint.to_string()),
            Some("817e7be0".to_string())
        );
        assert_eq!(
            xpub_from_descriptors(&descriptors).unwrap().to_string(),
            "xpub6CCKAvUTNursEnaJ8k1d27LfqEUzeAx2N9wFqYE3W1xh7nqgJEBEbLSSmohwDxzsSvcsYqiQqFzRvta65Njbe5o84bF5YXHFqfSH2Dkhonm"
        );
    }

    #[test]
    fn converted_json_import_material_matches_pubport_for_primary_descriptors() {
        for pubport_descriptors in [bip84_descriptors(), bip44_descriptors()] {
            let descriptors: Descriptors = pubport_descriptors.clone().into();

            assert_eq!(descriptors.fingerprint(), pubport_descriptors.fingerprint());
            assert_eq!(
                xpub_from_descriptors(&descriptors).unwrap(),
                pubport_descriptors.xpub().unwrap()
            );
        }
    }

    #[test]
    fn descriptor_address_type_infers_supported_descriptor_formats() {
        for (pubport_descriptors, expected) in [
            (bip84_descriptors(), WalletAddressType::NativeSegwit),
            (bip49_descriptors(), WalletAddressType::WrappedSegwit),
            (bip44_descriptors(), WalletAddressType::Legacy),
        ] {
            let descriptors = Descriptors::from(pubport_descriptors);

            assert_eq!(address_type_from_descriptors(&descriptors).unwrap(), expected);
        }
    }

    #[test]
    fn descriptor_address_type_rejects_taproot() {
        let descriptors = Descriptors::from(bip86_descriptors());
        let error = address_type_from_descriptors(&descriptors).unwrap_err();

        assert_eq!(
            error,
            WalletError::UnsupportedWallet("Taproot descriptors are not supported".to_string())
        );
    }

    #[test]
    fn bare_zpub_import_selects_native_segwit_descriptor() {
        let json = json_from_export(BIP84_ZPUB);

        assert!(json.bip84.is_some());
        assert!(json.bip49.is_none());

        let (descriptors, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(descriptors.external.to_string().starts_with("wpkh("));
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_starts_for_native_segwit_with_alternates() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_starts_for_wrapped_segwit_with_legacy_alternate() {
        let json = descriptor_json(Some(bip44_descriptors()), Some(bip49_descriptors()), None);

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_does_not_start_for_single_native_segwit_export() {
        let json = descriptor_json(None, None, Some(bip84_descriptors()));

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::NativeSegwit);
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn json_discovery_does_not_start_for_wrapped_segwit_export() {
        let json = descriptor_json(None, Some(bip49_descriptors()), None);

        let (_, address_type) = preferred_json_descriptors(&json).unwrap();

        assert_eq!(address_type, WalletAddressType::WrappedSegwit);
        assert!(!should_start_json_discovery(&json, address_type));
    }

    #[test]
    fn cold_upgrade_metadata_carries_json_discovery_state() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );
        let descriptors = Descriptors::from(bip84_descriptors());

        let mut existing_metadata = WalletMetadata::preview_new();
        existing_metadata.name = "Existing watch-only wallet".to_string();
        existing_metadata.wallet_type = WalletType::WatchOnly;
        existing_metadata.discovery_state = DiscoveryState::Single;
        existing_metadata.address_type = WalletAddressType::Legacy;

        let mut import_metadata = existing_metadata.clone();
        import_metadata.name = "Incoming hardware wallet".to_string();
        import_metadata.wallet_type = WalletType::Cold;
        import_metadata.address_type = WalletAddressType::NativeSegwit;
        import_metadata.discovery_state =
            DiscoveryState::StartedJson(Arc::new(json.clone().into()));

        let updated =
            metadata_for_cold_upgrade(existing_metadata.clone(), &import_metadata, &descriptors);

        assert_eq!(updated.name, existing_metadata.name);
        assert_ne!(updated.name, import_metadata.name);
        assert_eq!(updated.wallet_type, WalletType::Cold);
        assert_eq!(updated.address_type, WalletAddressType::NativeSegwit);
        assert_eq!(updated.origin, descriptors.origin().ok());

        let DiscoveryState::StartedJson(found_json) = updated.discovery_state else {
            panic!("expected JSON discovery to start after cold upgrade");
        };

        assert_eq!(found_json.as_ref().0, json);
    }

    #[test]
    fn test_fingerprint() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::delete_database();

        let mnemonic = Mnemonic::parse_normalized(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();

        let metadata = WalletMetadata::preview_new();

        let wallet =
            Wallet::try_new_persisted_from_mnemonic_segwit(metadata.clone(), mnemonic, None)
                .unwrap();

        let fingerprint = wallet.metadata.master_fingerprint.as_ref().unwrap().as_lowercase();

        let _ = delete_wallet_specific_data(&metadata.id);
        assert_eq!("73c5da0a", fingerprint.as_str());
    }
}

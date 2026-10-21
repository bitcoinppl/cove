use cove_device::keychain::Keychain;
use strum::IntoEnumIterator as _;
use tracing::warn;

use crate::database::Database;
use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletMetadata, WalletMode, WalletType};

use super::WalletIdentityKey;
use super::backup::no_fingerprint_wallet_id;
use super::{ExistingWalletIdentitySet, PublicWalletIdentity, WalletIdentityError};

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
        if metadata.wallet_type == WalletType::Hot {
            if hot_wallet_secret_is_readable(&metadata, keychain) {
                return Ok(WalletIdentityKey::HotFingerprint {
                    fingerprint,
                    network: metadata.network,
                    mode: metadata.wallet_mode,
                });
            }

            return Ok(unsigned_hot_wallet_identity_key(metadata, keychain, fingerprint));
        }

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

/// Identity of a hot wallet whose secret is missing or unreadable
///
/// It cannot cover a seed backup, and a fingerprint match would also cover every other public
/// wallet under that fingerprint, such as another address type of the same seed, so it matches
/// only its own public descriptors, or only its wallet ID when those cannot be read either
fn unsigned_hot_wallet_identity_key(
    metadata: WalletMetadata,
    keychain: &Keychain,
    fingerprint: Fingerprint,
) -> WalletIdentityKey {
    match PublicWalletIdentity::from_existing_wallet(&metadata, keychain) {
        Ok(Some(identity)) => {
            return WalletIdentityKey::PublicIdentity {
                identity,
                fingerprint: Some(fingerprint),
                wallet_id: None,
                network: metadata.network,
                mode: metadata.wallet_mode,
            };
        }
        Ok(None) => {}
        Err(error) => {
            warn!("hot wallet {} public identity could not be read: {error}", metadata.id);
        }
    }

    WalletIdentityKey::WalletId {
        id: metadata.id,
        network: metadata.network,
        mode: metadata.wallet_mode,
    }
}

fn hot_wallet_secret_is_readable(metadata: &WalletMetadata, keychain: &Keychain) -> bool {
    let id = &metadata.id;

    match keychain.get_wallet_secret(id) {
        Ok(Some(_)) => true,
        Ok(None) => {
            warn!("hot wallet {id} has no stored secret");
            false
        }
        Err(error) => {
            warn!("hot wallet {id} secret could not be read: {error}");
            false
        }
    }
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

/// Finds a wallet with no master fingerprint whose public descriptors match the incoming ones
///
/// A bare xpub has no key origin, so fingerprint matching can never see its duplicates
pub(crate) fn existing_origin_less_wallet_by_identity(
    database: &Database,
    keychain: &Keychain,
    network: Network,
    mode: WalletMode,
    incoming_identity: &PublicWalletIdentity,
) -> Result<Option<WalletMetadata>, WalletIdentityError> {
    for wallet_metadata in database.wallets.get_all(network, mode)? {
        if wallet_metadata.master_fingerprint.is_some() {
            continue;
        }

        let wallet_identity =
            PublicWalletIdentity::from_existing_wallet(&wallet_metadata, keychain).map_err(
                |source| WalletIdentityError::ExistingWalletPublicIdentity {
                    wallet_id: wallet_metadata.id.clone(),
                    source,
                },
            )?;

        if wallet_identity.as_ref() == Some(incoming_identity) {
            return Ok(Some(wallet_metadata));
        }
    }

    Ok(None)
}

pub(crate) fn matching_public_wallet_by_identity(
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

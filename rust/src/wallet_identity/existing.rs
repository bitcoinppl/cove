use cove_device::keychain::{Keychain, KeychainError};
use strum::IntoEnumIterator as _;
use tracing::warn;

use crate::database::Database;
use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletMetadata, WalletMode, WalletType};

use super::WalletIdentityKey;
use super::backup::no_fingerprint_wallet_id;

use super::{
    BackupRecoveryMaterial, CoveringWallet, ExistingWalletIdentitySet, LocalWalletIdentities,
    NamedWalletCoverage, PublicWalletIdentity, PublicWalletIdentityError, WalletIdentityError,
};

pub(crate) fn collect_existing_wallet_identities()
-> Result<ExistingWalletIdentitySet, WalletIdentityError> {
    collect_local_wallet_identities().map(LocalWalletIdentities::into_all)
}

/// Every local wallet's identity, kept per wallet
///
/// A keychain read error fails the whole read instead of leaving a wallet out, since a wallet
/// left out could make a backup it covers look restorable or one it lacks look covered
pub(crate) fn collect_local_wallet_identities() -> Result<LocalWalletIdentities, WalletIdentityError>
{
    let db = Database::global();
    let keychain = Keychain::global();
    let mut identities = LocalWalletIdentities::default();

    for network in Network::iter() {
        for mode in [WalletMode::Main, WalletMode::Decoy] {
            let wallets = db.wallets.get_all(network, mode)?;

            for wallet in wallets {
                let covering = CoveringWallet { id: wallet.id.clone(), name: wallet.name.clone() };
                let duplicate_key = existing_wallet_identity_key(wallet, keychain)?;
                identities.insert(covering, duplicate_key);
            }
        }
    }

    Ok(identities)
}

/// Whether a local wallet holds the seed a backup of `metadata`'s wallet could hold
///
/// Keys that backup as the wallet itself, since a wallet's own backup is of that wallet, and
/// asks the coverage a restore asks of a secret backup: only a local hot wallet whose secret is
/// stored and decrypts covers it, so a public-only wallet never does. A keychain read error is
/// returned instead of read as covered
pub(crate) fn local_wallets_hold_seed_of(
    identities: &LocalWalletIdentities,
    metadata: &WalletMetadata,
    keychain: &Keychain,
) -> Result<bool, WalletIdentityError> {
    let key = existing_wallet_identity_key(metadata.clone(), keychain)?;
    let coverage = identities.coverage(&key, BackupRecoveryMaterial::Secret);

    Ok(matches!(coverage, NamedWalletCoverage::Covered(_)))
}

pub(crate) fn existing_wallet_identity_key(
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
            if hot_wallet_has_usable_secret(&metadata, keychain)? {
                return Ok(WalletIdentityKey::HotFingerprint {
                    fingerprint,
                    network: metadata.network,
                    mode: metadata.wallet_mode,
                });
            }

            return unsigned_hot_wallet_identity_key(metadata, keychain, fingerprint);
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

/// Identity of a hot wallet whose secret is missing or cannot be decrypted
///
/// It cannot cover a seed backup, and a fingerprint match would also cover every other public
/// wallet under that fingerprint, such as another address type of the same seed, so it matches
/// only its own public descriptors, or only its wallet ID when those are invalid
fn unsigned_hot_wallet_identity_key(
    metadata: WalletMetadata,
    keychain: &Keychain,
    fingerprint: Fingerprint,
) -> Result<WalletIdentityKey, WalletIdentityError> {
    match PublicWalletIdentity::from_existing_wallet(&metadata, keychain) {
        Ok(Some(identity)) => {
            return Ok(WalletIdentityKey::PublicIdentity {
                identity,
                fingerprint: Some(fingerprint),
                wallet_id: None,
                network: metadata.network,
                mode: metadata.wallet_mode,
            });
        }
        Ok(None) => {}

        // a wallet ID match is narrower than its descriptors, so falling back while they cannot
        // be read would let a restore add a second copy of this wallet
        Err(source @ PublicWalletIdentityError::Keychain(KeychainError::Read(_))) => {
            return Err(WalletIdentityError::ExistingWalletPublicIdentity {
                wallet_id: metadata.id,
                source,
            });
        }
        Err(error) => {
            warn!("hot wallet {} public identity is invalid: {error}", metadata.id);
        }
    }

    Ok(WalletIdentityKey::WalletId {
        id: metadata.id,
        network: metadata.network,
        mode: metadata.wallet_mode,
    })
}

/// Whether a hot wallet's secret is stored and decrypts
///
/// A read failure is returned as an error instead of false, since the secret may still be there
/// and a wallet that can sign would otherwise be treated as one that cannot
fn hot_wallet_has_usable_secret(
    metadata: &WalletMetadata,
    keychain: &Keychain,
) -> Result<bool, WalletIdentityError> {
    let id = &metadata.id;

    match keychain.get_wallet_secret(id) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => {
            warn!("hot wallet {id} has no stored secret");
            Ok(false)
        }
        Err(source @ KeychainError::Read(_)) => {
            Err(WalletIdentityError::ExistingWalletSecret { wallet_id: id.clone(), source })
        }
        Err(error) => {
            warn!("hot wallet {id} secret is invalid: {error}");
            Ok(false)
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

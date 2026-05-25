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

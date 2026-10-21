use std::collections::HashSet;

use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletId, WalletMode};

use super::PublicWalletIdentity;

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
    HotFingerprint {
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

/// What a backup can recover a wallet from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupRecoveryMaterial {
    /// A seed or extended private key that can sign
    Secret,
    /// Public keys only, which can watch but not sign
    PublicOnly,
}

/// How much of a backup's recovery material the local wallets already hold
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalWalletCoverage {
    /// A local wallet holds everything the backup can recover, so the backup is an extra copy
    Covered,
    /// A local wallet matches the backup's identity but lacks the secret only the backup holds
    SecretMissing,
    /// No local wallet matches the backup
    Absent,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExistingWalletIdentitySet {
    public_identities: HashSet<(PublicWalletIdentity, Network, WalletMode)>,
    public_identity_fingerprints: HashSet<(Fingerprint, Network, WalletMode)>,
    degraded_fingerprints: HashSet<(Fingerprint, Network, WalletMode)>,
    hot_fingerprints: HashSet<(Fingerprint, Network, WalletMode)>,
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
                let public_identity_exists =
                    self.public_identities.contains(&(identity.clone(), *network, *mode));

                let degraded_fingerprint_exists = fingerprint.is_some_and(|fingerprint| {
                    self.degraded_fingerprints.contains(&(fingerprint, *network, *mode))
                });

                let wallet_id_exists = wallet_id
                    .as_ref()
                    .is_some_and(|id| self.wallet_ids.contains(&(id.clone(), *network, *mode)));

                public_identity_exists || degraded_fingerprint_exists || wallet_id_exists
            }

            WalletIdentityKey::Fingerprint { fingerprint, network, mode }
            | WalletIdentityKey::HotFingerprint { fingerprint, network, mode } => {
                let degraded_fingerprint_exists =
                    self.degraded_fingerprints.contains(&(*fingerprint, *network, *mode));

                let public_identity_fingerprint_exists =
                    self.public_identity_fingerprints.contains(&(*fingerprint, *network, *mode));

                let hot_fingerprint_exists =
                    self.hot_fingerprints.contains(&(*fingerprint, *network, *mode));

                degraded_fingerprint_exists
                    || public_identity_fingerprint_exists
                    || hot_fingerprint_exists
            }

            WalletIdentityKey::WalletId { id, network, mode } => {
                self.wallet_ids.contains(&(id.clone(), *network, *mode))
            }
        }
    }

    /// Whether a local wallet already holds the recovery material of a backup with this key
    ///
    /// Stricter than `contains` for secret backups: a watch-only or cold wallet with the same
    /// fingerprint cannot sign, so only a local hot wallet with that master fingerprint covers
    /// the backup's secret
    pub(crate) fn coverage(
        &self,
        key: &WalletIdentityKey,
        material: BackupRecoveryMaterial,
    ) -> LocalWalletCoverage {
        let holds_identity = self.contains(key);

        match material {
            BackupRecoveryMaterial::PublicOnly if holds_identity => LocalWalletCoverage::Covered,
            BackupRecoveryMaterial::Secret if self.holds_secret_for(key) => {
                LocalWalletCoverage::Covered
            }
            BackupRecoveryMaterial::Secret if holds_identity => LocalWalletCoverage::SecretMissing,
            BackupRecoveryMaterial::PublicOnly | BackupRecoveryMaterial::Secret => {
                LocalWalletCoverage::Absent
            }
        }
    }

    /// Whether a local hot wallet holds the master secret behind this key's fingerprint
    fn holds_secret_for(&self, key: &WalletIdentityKey) -> bool {
        let (fingerprint, network, mode) = match key {
            WalletIdentityKey::PublicIdentity { fingerprint, network, mode, .. } => {
                let Some(fingerprint) = fingerprint else { return false };
                (*fingerprint, *network, *mode)
            }
            WalletIdentityKey::Fingerprint { fingerprint, network, mode }
            | WalletIdentityKey::HotFingerprint { fingerprint, network, mode } => {
                (*fingerprint, *network, *mode)
            }
            WalletIdentityKey::WalletId { .. } => return false,
        };

        self.hot_fingerprints.contains(&(fingerprint, network, mode))
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
                self.degraded_fingerprints.insert((fingerprint, network, mode));
            }

            WalletIdentityKey::HotFingerprint { fingerprint, network, mode } => {
                self.hot_fingerprints.insert((fingerprint, network, mode));
            }

            WalletIdentityKey::WalletId { id, network, mode } => {
                self.wallet_ids.insert((id, network, mode));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::ExistingWalletIdentitySet;

    pub(crate) trait ExistingWalletIdentitySetTestExt {
        fn len(&self) -> usize;
    }

    impl ExistingWalletIdentitySetTestExt for ExistingWalletIdentitySet {
        fn len(&self) -> usize {
            self.public_identities.len()
                + self.degraded_fingerprints.len()
                + self.hot_fingerprints.len()
                + self.wallet_ids.len()
        }
    }
}

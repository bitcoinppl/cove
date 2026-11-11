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

/// A local wallet that holds everything a backup can recover
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoveringWallet {
    pub(crate) id: WalletId,
    /// The wallet's name when coverage was read, for display only; never used to match
    pub(crate) name: String,
}

/// The local wallets that each cover a backup; never empty, since one identity can be held by
/// several local wallets
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoveringWallets {
    first: CoveringWallet,
    rest: Vec<CoveringWallet>,
}

impl CoveringWallets {
    pub(crate) fn one(wallet: CoveringWallet) -> Self {
        Self { first: wallet, rest: Vec::new() }
    }

    /// `None` when no wallet covers the backup
    pub(crate) fn from_wallets(wallets: impl IntoIterator<Item = CoveringWallet>) -> Option<Self> {
        let mut wallets = wallets.into_iter();
        let first = wallets.next()?;

        Some(Self { first, rest: wallets.collect() })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CoveringWallet> {
        std::iter::once(&self.first).chain(&self.rest)
    }

    /// The name shown for the covering wallet: the first one coverage found
    pub(crate) fn display_name(&self) -> &str {
        &self.first.name
    }
}

/// The local wallets' identities, each also kept per wallet, so a covered backup can say which
/// local wallets cover it
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalWalletIdentities {
    all: ExistingWalletIdentitySet,
    wallets: Vec<(CoveringWallet, ExistingWalletIdentitySet)>,
}

/// How much of a backup the local wallets hold, naming the wallets that hold all of it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NamedWalletCoverage {
    Covered(CoveringWallets),
    SecretMissing,
    Absent,
}

impl LocalWalletIdentities {
    pub(crate) fn insert(&mut self, wallet: CoveringWallet, key: WalletIdentityKey) {
        let mut identity = ExistingWalletIdentitySet::default();
        identity.insert(key.clone());

        self.all.insert(key);
        self.wallets.push((wallet, identity));
    }

    /// Every local wallet's identity in one set
    pub(crate) fn into_all(self) -> ExistingWalletIdentitySet {
        self.all
    }

    /// How much of a backup with this key the local wallets hold
    ///
    /// Each coverage check is a membership test against the key one wallet inserted, so the
    /// local wallets together cover a backup exactly when a single one of them does
    pub(crate) fn coverage(
        &self,
        key: &WalletIdentityKey,
        material: BackupRecoveryMaterial,
    ) -> NamedWalletCoverage {
        let covering = self
            .wallets
            .iter()
            .filter(|(_, identity)| {
                identity.coverage(key, material) == LocalWalletCoverage::Covered
            })
            .map(|(wallet, _)| wallet.clone());

        if let Some(covering) = CoveringWallets::from_wallets(covering) {
            return NamedWalletCoverage::Covered(covering);
        }

        match self.all.coverage(key, material) {
            LocalWalletCoverage::SecretMissing => NamedWalletCoverage::SecretMissing,

            // no single wallet covers the backup, so it is not an extra copy; treating it as
            // absent keeps it restorable, which is the safe side
            LocalWalletCoverage::Covered | LocalWalletCoverage::Absent => {
                NamedWalletCoverage::Absent
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

#[cfg(test)]
mod tests {
    use super::{
        BackupRecoveryMaterial, CoveringWallet, LocalWalletIdentities, NamedWalletCoverage,
        WalletIdentityKey,
    };
    use crate::network::Network;
    use crate::wallet::metadata::{WalletId, WalletMode};

    fn wallet(id: &str, name: &str) -> CoveringWallet {
        CoveringWallet { id: WalletId::from(id.to_string()), name: name.into() }
    }

    fn key(id: &str) -> WalletIdentityKey {
        WalletIdentityKey::WalletId {
            id: WalletId::from(id.to_string()),
            network: Network::Bitcoin,
            mode: WalletMode::Main,
        }
    }

    fn covering_ids(coverage: NamedWalletCoverage) -> Vec<WalletId> {
        let NamedWalletCoverage::Covered(covering) = coverage else {
            panic!("expected the backup to be covered, got {coverage:?}");
        };

        covering.iter().map(|wallet| wallet.id.clone()).collect()
    }

    #[test]
    fn coverage_names_the_wallet_holding_the_identity_not_a_same_name_wallet() {
        let mut local = LocalWalletIdentities::default();
        local.insert(wallet("wallet-a", "Test Wallet"), key("identity-a"));
        local.insert(wallet("wallet-b", "Test Wallet"), key("identity-b"));

        let covering = local.coverage(&key("identity-b"), BackupRecoveryMaterial::PublicOnly);
        assert_eq!(covering_ids(covering), vec![WalletId::from("wallet-b".to_string())]);

        let absent = local.coverage(&key("identity-c"), BackupRecoveryMaterial::PublicOnly);
        assert_eq!(absent, NamedWalletCoverage::Absent);
    }

    #[test]
    fn coverage_names_every_wallet_holding_one_identity() {
        let mut local = LocalWalletIdentities::default();
        local.insert(wallet("wallet-a", "Spending"), key("shared"));
        local.insert(wallet("wallet-b", "Savings"), key("shared"));
        local.insert(wallet("wallet-c", "Other"), key("other"));

        let covering = local.coverage(&key("shared"), BackupRecoveryMaterial::PublicOnly);
        assert_eq!(
            covering_ids(covering),
            vec![WalletId::from("wallet-a".to_string()), WalletId::from("wallet-b".to_string())]
        );
    }
}

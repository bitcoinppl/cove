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
                let public_identity_exists =
                    self.public_identities.contains(&(identity.clone(), *network, *mode));

                let fingerprint_exists = fingerprint.is_some_and(|fingerprint| {
                    self.fingerprints.contains(&(fingerprint, *network, *mode))
                });

                let wallet_id_exists = wallet_id
                    .as_ref()
                    .is_some_and(|id| self.wallet_ids.contains(&(id.clone(), *network, *mode)));

                public_identity_exists || fingerprint_exists || wallet_id_exists
            }

            WalletIdentityKey::Fingerprint { fingerprint, network, mode } => {
                let fingerprint_exists =
                    self.fingerprints.contains(&(*fingerprint, *network, *mode));

                let public_identity_fingerprint_exists =
                    self.public_identity_fingerprints.contains(&(*fingerprint, *network, *mode));

                fingerprint_exists || public_identity_fingerprint_exists
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

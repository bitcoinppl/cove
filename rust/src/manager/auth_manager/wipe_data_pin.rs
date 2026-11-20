use cove_types::{Network, WalletId};

use super::SecurityAlertState;
use crate::{
    database::{self, Database, global_config::SelectedWalletTarget, read_health},
    manager::cloud_backup_manager::CloudBackupRecoveryCoverage,
    wallet::metadata::{WalletMetadata, WalletMode},
};

/// Why the Wipe Data PIN can't be turned on yet
///
/// The wipe deletes every wallet on every network in both modes, so a hot wallet with neither
/// verified words nor a confirmed Cloud Backup copy would lose its only keys
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WipeDataPinBlocker {
    /// A wallet the current mode lists has no backup, so Go To Wallet can open it
    UnbackedWallet(WalletId),
    /// A decoy wallet has no backup, and main mode can't open it
    UnbackedDecoyWallet,
    /// A wallet list couldn't be read, so a wallet with no backup can't be ruled out
    WalletsUnreadable,
    /// A wallet has no backup while the provider can't confirm Cloud Backup's master key, so its
    /// cloud copy can't restore it; main mode only, since decoy mode hides Cloud Backup
    CloudBackupKeyUnconfirmed,
}

impl WipeDataPinBlocker {
    /// Finds what keeps the Wipe Data PIN off, if anything does
    pub(crate) fn find() -> Option<Self> {
        let database = Database::global();
        let target = database.global_config.wallet_target();
        let coverage = CloudBackupRecoveryCoverage::load();

        let blocker = Self::find_with(
            target,
            |mode, first| database.wallets.all_networks(mode, first),
            |wallet| coverage.needs_backup(wallet),
        )?;

        // the cloud copies count for nothing while the key is unconfirmed, so that is the reason to
        // give where Cloud Backup shows; either syncing it or verifying the words clears it
        let key_unconfirmed = target.mode == WalletMode::Main
            && CloudBackupRecoveryCoverage::master_key_unconfirmed_at_provider();

        match blocker {
            Self::UnbackedWallet(_) | Self::UnbackedDecoyWallet if key_unconfirmed => {
                Some(Self::CloudBackupKeyUnconfirmed)
            }
            blocker => Some(blocker),
        }
    }

    /// Checks the current mode's wallets first, selected network ahead, so the alert names a
    /// wallet Go To Wallet can open whenever one qualifies
    fn find_with(
        target: SelectedWalletTarget,
        mut load: impl FnMut(WalletMode, Network) -> Result<Vec<WalletMetadata>, database::Error>,
        needs_backup: impl Fn(&WalletMetadata) -> bool,
    ) -> Option<Self> {
        let SelectedWalletTarget { network, mode } = target;

        let Some(shown) = read_health::observe("wallet list on every network", load(mode, network))
        else {
            return Some(Self::WalletsUnreadable);
        };

        if let Some(wallet) = shown.into_iter().find(|wallet| needs_backup(wallet)) {
            return Some(Self::UnbackedWallet(wallet.id));
        }

        // decoy mode must never reveal the main wallets, and its Settings change only decoy-local
        // toggles, so it checks its own wallets as main mode would
        if mode == WalletMode::Decoy {
            return None;
        }

        let Some(decoy) =
            read_health::observe("decoy wallet list", load(WalletMode::Decoy, network))
        else {
            return Some(Self::WalletsUnreadable);
        };

        decoy.iter().any(needs_backup).then_some(Self::UnbackedDecoyWallet)
    }
}

impl From<WipeDataPinBlocker> for SecurityAlertState {
    fn from(blocker: WipeDataPinBlocker) -> Self {
        match blocker {
            WipeDataPinBlocker::UnbackedWallet(wallet_id) => Self::UnverifiedWallets { wallet_id },
            WipeDataPinBlocker::UnbackedDecoyWallet => Self::UnverifiedDecoyWallets,
            WipeDataPinBlocker::WalletsUnreadable => Self::UnreadableWallets,
            WipeDataPinBlocker::CloudBackupKeyUnconfirmed => Self::CloudBackupKeyUnconfirmed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::wallet::WalletTableError;

    fn wallet(name: &str, mode: WalletMode) -> WalletMetadata {
        let mut wallet = WalletMetadata::preview_new();
        wallet.id = WalletId::from(name.to_string());
        wallet.name = name.to_string();
        wallet.wallet_mode = mode;
        wallet
    }

    fn target(mode: WalletMode) -> SelectedWalletTarget {
        SelectedWalletTarget { network: Network::Signet, mode }
    }

    fn unreadable() -> database::Error {
        WalletTableError::ReadError("injected".into()).into()
    }

    fn find(
        mode: WalletMode,
        main: Result<Vec<WalletMetadata>, ()>,
        decoy: Result<Vec<WalletMetadata>, ()>,
    ) -> Option<WipeDataPinBlocker> {
        WipeDataPinBlocker::find_with(
            target(mode),
            |mode, _| match mode {
                WalletMode::Main => main.clone().map_err(|()| unreadable()),
                WalletMode::Decoy => decoy.clone().map_err(|()| unreadable()),
            },
            |wallet| wallet.name.starts_with("unbacked"),
        )
    }

    #[test]
    fn main_mode_names_its_own_wallet_first_then_checks_the_decoy_wallets() {
        use WalletMode::{Decoy, Main};

        let main_unbacked = wallet("unbacked-main", Main);
        let decoy_unbacked = wallet("unbacked-decoy", Decoy);
        let backed = wallet("backed", Main);

        assert_eq!(
            find(Main, Ok(vec![backed.clone(), main_unbacked]), Ok(vec![decoy_unbacked.clone()])),
            Some(WipeDataPinBlocker::UnbackedWallet(WalletId::from("unbacked-main".to_string())))
        );
        assert_eq!(
            find(Main, Ok(vec![backed.clone()]), Ok(vec![decoy_unbacked.clone()])),
            Some(WipeDataPinBlocker::UnbackedDecoyWallet)
        );
        assert_eq!(find(Main, Ok(vec![backed]), Ok(vec![wallet("backed-decoy", Decoy)])), None);

        // decoy mode never looks at the main wallets, even unreadable ones
        assert_eq!(find(Decoy, Err(()), Ok(vec![])), None);
        assert_eq!(
            find(Decoy, Ok(vec![]), Ok(vec![decoy_unbacked])),
            Some(WipeDataPinBlocker::UnbackedWallet(WalletId::from("unbacked-decoy".to_string())))
        );
    }

    #[test]
    fn any_wallet_list_the_wipe_deletes_that_fails_to_read_keeps_the_pin_off() {
        use WalletMode::{Decoy, Main};

        assert_eq!(find(Main, Err(()), Ok(vec![])), Some(WipeDataPinBlocker::WalletsUnreadable));
        assert_eq!(find(Main, Ok(vec![]), Err(())), Some(WipeDataPinBlocker::WalletsUnreadable));
        assert_eq!(find(Decoy, Ok(vec![]), Err(())), Some(WipeDataPinBlocker::WalletsUnreadable));
    }
}

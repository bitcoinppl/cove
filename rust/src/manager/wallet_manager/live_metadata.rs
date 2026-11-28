use parking_lot::{RwLock, RwLockReadGuard};
use tokio::sync::watch;

use crate::wallet::metadata::{FiatOrBtc, WalletMetadata};
use cove_types::unit::BitcoinUnit;

/// The wallet preferences that decide how amounts are shown and entered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletDisplayPreferences {
    pub selected_unit: BitcoinUnit,
    pub fiat_or_btc: FiatOrBtc,
    pub sensitive_visible: bool,
}

/// The wallet's current metadata, shared by the wallet manager and its actor
///
/// The wallet actor is the only writer. Every write also publishes the display preferences on a
/// watch channel, so flows that render amounts for this wallet, such as Send, follow changes from
/// any screen without the platform forwarding them
#[derive(Debug)]
pub struct LiveWalletMetadata {
    metadata: RwLock<WalletMetadata>,
    display_preferences: watch::Sender<WalletDisplayPreferences>,
}

impl From<&WalletMetadata> for WalletDisplayPreferences {
    fn from(metadata: &WalletMetadata) -> Self {
        Self {
            selected_unit: metadata.selected_unit,
            fiat_or_btc: metadata.fiat_or_btc,
            sensitive_visible: metadata.sensitive_visible,
        }
    }
}

impl LiveWalletMetadata {
    pub fn new(metadata: WalletMetadata) -> Self {
        let (display_preferences, _) = watch::channel(WalletDisplayPreferences::from(&metadata));
        Self { metadata: RwLock::new(metadata), display_preferences }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, WalletMetadata> {
        self.metadata.read()
    }

    /// The current display preferences, read live rather than from a copy
    pub fn display_preferences(&self) -> WalletDisplayPreferences {
        *self.display_preferences.borrow()
    }

    /// Subscribes to display preference changes
    ///
    /// The receiver starts at the current preferences, so a caller that reads its first value
    /// with `borrow_and_update` sees every later change
    pub fn subscribe_display_preferences(&self) -> watch::Receiver<WalletDisplayPreferences> {
        self.display_preferences.subscribe()
    }

    /// Replaces the metadata and publishes its display preferences when they changed
    ///
    /// Only the wallet actor calls this, as the sole metadata writer
    pub(crate) fn replace(&self, metadata: WalletMetadata) {
        let preferences = WalletDisplayPreferences::from(&metadata);

        // publish under the write lock, so a reader never sees metadata and preferences disagree
        let mut current = self.metadata.write();
        *current = metadata;
        self.display_preferences.send_if_modified(|published| {
            let changed = *published != preferences;
            *published = preferences;
            changed
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_publishes_only_display_preference_changes() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.selected_unit = BitcoinUnit::Btc;
        let live = LiveWalletMetadata::new(metadata.clone());
        let mut receiver = live.subscribe_display_preferences();
        receiver.borrow_and_update();

        metadata.name = "renamed".to_string();
        live.replace(metadata.clone());
        assert!(!receiver.has_changed().unwrap(), "a rename is not a display change");
        assert_eq!(live.read().name, "renamed");

        metadata.selected_unit = BitcoinUnit::Sat;
        live.replace(metadata);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow_and_update().selected_unit, BitcoinUnit::Sat);
    }
}

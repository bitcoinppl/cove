//! Whether an address belongs to one of the user's wallets
//!
//! A lookup derives scripts from the public descriptors stored on this device and never touches
//! the network, so checking an address cannot reveal it to a server. It only searches wallets in
//! the active wallet mode, so decoy mode never reveals a main-mode wallet and main mode never
//! reveals a decoy wallet

use bdk_wallet::{KeychainKind, chain::SpkIterator, descriptor::ExtendedDescriptor};
use bitcoin::{Script, ScriptBuf};
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_types::{
    Network, WalletId,
    address::{AddressError, AddressWithNetwork},
};
use cove_util::result_ext::ResultExt as _;
use strum::IntoEnumIterator as _;
use tracing::warn;

use crate::{
    database::Database,
    wallet::{
        WalletError,
        addressing::{self, PublishedWalletScripts},
        metadata::{WalletMetadata, WalletMode},
    },
};

/// How many addresses past the last revealed one each address chain is searched
///
/// Software that shares a wallet's keys, such as a hardware wallet's companion app, can hand out
/// addresses Cove has not revealed yet, so the search reaches past what Cove has shown
pub const ADDRESS_OWNERSHIP_LOOKUP_GAP: u32 = 100;

/// Which of a wallet's two address chains an address comes from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum AddressChain {
    Receive,
    Change,
}

/// One wallet that owns the looked-up address
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct OwnedAddress {
    pub wallet_id: WalletId,
    pub wallet_name: String,
    pub network: Network,
    pub chain: AddressChain,
    pub index: u32,
    /// The account in the wallet's derivation path, when its descriptor records one
    pub account: Option<u32>,
}

/// A wallet the lookup could not read, so the address might still belong to it
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct UncheckedWallet {
    pub wallet_id: WalletId,
    pub wallet_name: String,
    pub network: Network,
}

/// The answer to whether an address belongs to one of the user's wallets
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum AddressOwnership {
    /// The address belongs to these wallets, in wallet list order
    ///
    /// More than one wallet can own the same address, such as one key set imported on both
    /// testnet and signet, which share an address format. The list is never empty
    Owned { owners: Vec<OwnedAddress> },

    /// No searched wallet owns the address within the lookup gap
    ///
    /// Wallets that could not be read are listed, since the address may belong to one of them
    NotFound { unchecked_wallets: Vec<UncheckedWallet> },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum AddressOwnershipError {
    #[error("not a valid bitcoin address: {0}")]
    InvalidAddress(String),

    #[error("unable to read the wallet list: {0}")]
    WalletListUnavailable(String),

    /// The lookup's mode is not, or stopped being, the app's active mode, so its result would
    /// reveal the other mode's wallets and is withheld
    #[error("the wallet mode changed during the lookup")]
    WalletModeChanged,
}

/// Finds which of the user's wallets own an address
///
/// The mode is fixed when the lookup is created and must be the app's active mode, so no caller
/// can ask about wallets in the other mode
#[derive(Debug, Clone, Copy)]
pub struct AddressOwnershipLookup {
    mode: WalletMode,

    /// The only network searched, or `None` to search every network
    network: Option<Network>,
}

impl AddressOwnershipLookup {
    /// A lookup over every network's wallets in the app's active wallet mode
    pub fn for_active_mode() -> Self {
        Self { mode: Database::global().global_config.wallet_mode(), network: None }
    }

    /// A lookup over the wallets that share this wallet's network and mode
    ///
    /// Searches fail with [`AddressOwnershipError::WalletModeChanged`] unless that mode is the
    /// app's active mode, so a wallet left over from the other mode cannot reveal its wallets
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the send flow and transfer labels stacked on this call it")
    )]
    pub fn for_wallet(metadata: &WalletMetadata) -> Self {
        Self { mode: metadata.wallet_mode, network: Some(metadata.network) }
    }

    /// Search every wallet in scope whose network accepts the address
    ///
    /// This reads wallet stores from disk, so async callers should run it on a blocking thread
    pub fn find_owners(
        &self,
        address: &AddressWithNetwork,
    ) -> Result<AddressOwnership, AddressOwnershipError> {
        let script = address.address.script_pubkey();
        let networks = self.networks().filter(|network| address.is_valid_for_network(*network));

        self.find_in_networks(networks, &script)
    }

    /// Search every wallet in scope for an output script, such as a transaction output's
    ///
    /// A script carries no network, so a lookup over every network matches wallets on any of
    /// them; use [`Self::for_wallet`] to stay on one network.
    /// This reads wallet stores from disk, so async callers should run it on a blocking thread
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "transfer labels at broadcast, stacked on this, call it")
    )]
    pub fn find_script_owners(
        &self,
        script: &Script,
    ) -> Result<AddressOwnership, AddressOwnershipError> {
        self.find_in_networks(self.networks(), &script.to_owned())
    }

    fn networks(&self) -> impl Iterator<Item = Network> + use<> {
        let only = self.network;
        Network::iter().filter(move |network| only.is_none_or(|only| only == *network))
    }

    fn find_in_networks(
        &self,
        networks: impl Iterator<Item = Network>,
        script: &ScriptBuf,
    ) -> Result<AddressOwnership, AddressOwnershipError> {
        self.ensure_mode_is_active()?;

        let mut owners = Vec::new();
        let mut unchecked_wallets = Vec::new();
        for metadata in self.wallets_in(networks)? {
            match find_in_wallet(&metadata, script) {
                Ok(Some(owned)) => owners.push(owned),
                Ok(None) => {}
                Err(error) => {
                    warn!(id = %metadata.id, %error, "unable to check whether a wallet owns an address");
                    unchecked_wallets.push(UncheckedWallet::from(&metadata));
                }
            }
        }

        // a mode switch while the stores were read must not hand this mode's wallets to the other
        self.ensure_mode_is_active()?;

        if owners.is_empty() {
            return Ok(AddressOwnership::NotFound { unchecked_wallets });
        }

        Ok(AddressOwnership::Owned { owners })
    }

    fn ensure_mode_is_active(&self) -> Result<(), AddressOwnershipError> {
        if Database::global().global_config.wallet_mode() != self.mode {
            return Err(AddressOwnershipError::WalletModeChanged);
        }

        Ok(())
    }

    fn wallets_in(
        &self,
        networks: impl Iterator<Item = Network>,
    ) -> Result<Vec<WalletMetadata>, AddressOwnershipError> {
        let wallets_table = &Database::global().wallets;
        let mut wallets = Vec::new();
        for network in networks {
            let network_wallets = wallets_table
                .get_all(network, self.mode)
                .map_err_str(AddressOwnershipError::WalletListUnavailable)?;

            wallets.extend(network_wallets);
        }

        Ok(wallets)
    }
}

/// Check whether a scanned or pasted address, or a `bitcoin:` URI, belongs to one of the user's
/// wallets in the active wallet mode
///
/// Runs entirely on the device; nothing about the address is sent to a server
#[uniffi::export(async_runtime = "tokio")]
pub async fn check_address_ownership(
    address: String,
) -> Result<AddressOwnership, AddressOwnershipError> {
    let address = AddressWithNetwork::try_new(&address)?;
    let lookup = AddressOwnershipLookup::for_active_mode();

    cove_tokio::unblock::run_blocking(move || lookup.find_owners(&address)).await
}

/// Where one wallet derives the script, or `None` when the wallet's search window lacks it
fn find_in_wallet(
    metadata: &WalletMetadata,
    script: &ScriptBuf,
) -> Result<Option<OwnedAddress>, WalletError> {
    let scripts =
        addressing::read_published_wallet_scripts(metadata)?.ok_or(WalletError::WalletNotFound)?;

    let found = scripts
        .indexed_derivation(script)
        .or_else(|| scripts.search_past_revealed(KeychainKind::External, script))
        .or_else(|| scripts.search_past_revealed(KeychainKind::Internal, script));

    let Some((keychain, index)) = found else {
        return Ok(None);
    };

    Ok(Some(OwnedAddress {
        wallet_id: metadata.id.clone(),
        wallet_name: metadata.name.clone(),
        network: metadata.network,
        chain: keychain.into(),
        index,
        account: scripts.descriptor(KeychainKind::External).account_index(),
    }))
}

impl PublishedWalletScripts {
    fn descriptor(&self, keychain: KeychainKind) -> &ExtendedDescriptor {
        match (self, keychain) {
            (Self::Store(wallet), keychain) => wallet.public_descriptor(keychain),
            (Self::KeychainMirror(pair), KeychainKind::External) => &pair.0,
            (Self::KeychainMirror(pair), KeychainKind::Internal) => &pair.1,
        }
    }

    /// The store's own index, which covers every revealed address plus BDK's lookahead
    fn indexed_derivation(&self, script: &ScriptBuf) -> Option<(KeychainKind, u32)> {
        match self {
            Self::Store(wallet) => wallet.derivation_of_spk(script.clone()),
            Self::KeychainMirror(_) => None,
        }
    }

    /// Derive the unrevealed addresses up to the lookup gap
    ///
    /// The keychain mirror has no record of revealed addresses, so it is searched from index 0
    fn search_past_revealed(
        &self,
        keychain: KeychainKind,
        script: &ScriptBuf,
    ) -> Option<(KeychainKind, u32)> {
        let last_revealed = match self {
            Self::Store(wallet) => wallet.derivation_index(keychain),
            Self::KeychainMirror(_) => None,
        };

        let first_unrevealed = last_revealed.map_or(0, |index| index.saturating_add(1));
        let last_searched = last_revealed.map_or(ADDRESS_OWNERSHIP_LOOKUP_GAP - 1, |index| {
            index.saturating_add(ADDRESS_OWNERSHIP_LOOKUP_GAP)
        });

        SpkIterator::new_with_range(self.descriptor(keychain), first_unrevealed..=last_searched)
            .find(|(_, candidate)| candidate == script)
            .map(|(index, _)| (keychain, index))
    }
}

impl From<AddressError> for AddressOwnershipError {
    fn from(error: AddressError) -> Self {
        Self::InvalidAddress(error.to_string())
    }
}

impl From<KeychainKind> for AddressChain {
    fn from(keychain: KeychainKind) -> Self {
        match keychain {
            KeychainKind::External => Self::Receive,
            KeychainKind::Internal => Self::Change,
        }
    }
}

impl From<&WalletMetadata> for UncheckedWallet {
    fn from(metadata: &WalletMetadata) -> Self {
        Self {
            wallet_id: metadata.id.clone(),
            wallet_name: metadata.name.clone(),
            network: metadata.network,
        }
    }
}

#[cfg(test)]
mod tests;

//! The note under the send flow's address field when the address is one of the user's own

use std::sync::Arc;

use cove_types::{WalletId, address::Address};
use tracing::warn;

use super::{Message, RustSendFlowManager};
use crate::address_ownership::{AddressOwnership, AddressOwnershipError, AddressOwnershipLookup};

/// Tells the user that the address they are sending to is one of their own
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SendFlowAddressOwnership {
    /// The address this note describes; the apps show the note only while the send is to it, so
    /// a note that arrives after the address changed is never shown
    pub address: Arc<Address>,
    pub message: String,
}

impl SendFlowAddressOwnership {
    /// The note for an address the user owns, or `None` when none of their wallets do
    fn new(address: Arc<Address>, ownership: &AddressOwnership, sender: &WalletId) -> Option<Self> {
        let message = ownership.owner_to_name(sender)?.describe(sender);
        Some(Self { address, message })
    }
}

impl RustSendFlowManager {
    /// Check whether `address` is one of the user's own off the async runtime, and send the apps
    /// a note when it is
    ///
    /// Only wallets on the sending wallet's network and in its wallet mode are searched, and
    /// nothing leaves the device
    pub(crate) fn check_address_ownership(self: &Arc<Self>, address: Arc<Address>) {
        let me = self.clone();
        cove_tokio::task::spawn(async move {
            let sender = me.state.lock().metadata.clone();
            let lookup = AddressOwnershipLookup::for_wallet(&sender);
            let script = address.script_pubkey();

            let found =
                cove_tokio::unblock::run_blocking(move || lookup.find_script_owners(&script))
                    .await
                    // the mode can switch while this task waits to resume
                    .and_then(|ownership| lookup.ensure_mode_is_active().map(|()| ownership));

            let ownership = match found {
                Ok(ownership) => ownership,

                // a mode switch closes this send flow, so there is no one left to tell
                Err(AddressOwnershipError::WalletModeChanged) => return,

                Err(error) => {
                    warn!(id = %sender.id, %error, "unable to check whether the send address is owned");
                    return;
                }
            };

            if let Some(note) = SendFlowAddressOwnership::new(address, &ownership, &sender.id) {
                me.send_address_ownership(note);
            }
        });
    }

    /// Send the apps a note unless the send has moved on to another address
    ///
    /// The check and the send happen under the state lock, which every address change takes, so
    /// a note always reaches the apps before the note for any address that replaces it, and a
    /// slow lookup can never overwrite a newer note
    pub(crate) fn send_address_ownership(&self, note: SendFlowAddressOwnership) {
        let state = self.state.lock();
        if state.address.as_ref() != Some(&note.address) {
            return;
        }

        // sending never blocks, so holding the lock here cannot stall the apps
        self.reconciler.send(Message::UpdateAddressOwnership(note));
    }
}

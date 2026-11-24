//! Moving a pending send's row from recent to waiting to delayed as time passes, without a sync

use std::time::Duration;

use act_zero::{ActorResult, AddrLike as _, Produces, send};
use tracing::warn;

use crate::{
    fee_client::FEE_CLIENT,
    manager::wallet_manager::{WalletManagerReconcileMessage, actor::WalletActor},
    transaction::Transaction,
};

impl WalletActor {
    /// Rebuilds the list when the next pending send's status changes with time alone, at
    /// `at` in unix seconds
    ///
    /// Every list build asks again, so only a new time schedules a rebuild; a rebuild scheduled
    /// for a time no longer wanted still wakes, and does nothing
    pub(crate) fn schedule_pending_send_refresh(&mut self, at: Option<u64>, now: u64) {
        if self.pending_send_refresh_at == at {
            return;
        }

        self.pending_send_refresh_at = at;
        let Some(at) = at else { return };

        // one second late, so the rebuild's clock reads past the threshold
        let delay = Duration::from_secs(at.saturating_sub(now).saturating_add(1));
        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep(delay).await;
            send!(addr.refresh_pending_send_statuses(at));
        });
    }

    /// Judges the pending sends again whenever the cached Medium rate moves, for as long as the
    /// actor lives
    ///
    /// A send counts as delayed only while it pays under Medium, so a new rate can mark one late
    /// or clear it without a sync. The watch holds only a weak address, so it ends with the actor
    pub(crate) fn watch_cached_medium(&self) {
        let mut medium = FEE_CLIENT.watch_cached_medium();

        // the list this actor builds reads the rate cached now, so only a later one is news
        medium.mark_unchanged();

        self.addr.send_fut_with(move |addr| async move {
            while medium.changed().await.is_ok() {
                send!(addr.cached_medium_changed());
            }
        });
    }

    /// Sends the list again, and the details of each pending send, once a status changed
    pub async fn refresh_pending_send_statuses(&mut self, at: u64) -> ActorResult<()> {
        if self.pending_send_refresh_at != Some(at) {
            return Produces::ok(());
        }

        self.pending_send_refresh_at = None;
        self.send_pending_send_statuses().await;

        Produces::ok(())
    }

    /// Sends the pending sends' statuses again under the new Medium rate, when there are any
    pub async fn cached_medium_changed(&mut self) -> ActorResult<()> {
        self.send_pending_send_statuses().await;

        Produces::ok(())
    }

    async fn send_pending_send_statuses(&mut self) {
        let transactions = self.do_transactions().await;

        // an open details screen shows the same status as the row, so it moves with it
        let pending_sends = transactions
            .iter()
            .filter_map(|transaction| match transaction {
                Transaction::Unconfirmed(unconfirmed) if unconfirmed.pending_send.is_some() => {
                    Some(unconfirmed.txid)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        // with no pending send, no row depends on the time or the rate
        if pending_sends.is_empty() {
            return;
        }

        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));

        for tx_id in pending_sends {
            match self.transaction_details_presentation_for_tx_id(tx_id) {
                Ok(presentation) => self.send(
                    WalletManagerReconcileMessage::TransactionDetailsUpdated(presentation.into()),
                ),

                Err(error) => {
                    warn!("unable to refresh the details of pending send {tx_id}: {error}")
                }
            }
        }
    }
}

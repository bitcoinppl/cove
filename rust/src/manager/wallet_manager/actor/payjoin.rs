use std::time::{SystemTime, UNIX_EPOCH};

use act_zero::{
    ActorResult, Addr, AddrLike as _, Produces, WeakAddr, call, runtimes::tokio::spawn_actor, send,
};
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{Transaction as BdkTransaction, Txid};
use cove_types::{PayjoinIntent, PayjoinSessionId};
use cove_util::result_ext::ResultExt as _;
use tracing::{error, warn};

use crate::{
    database::Database,
    manager::wallet_manager::{
        Error, PayjoinBroadcastOutcome, PayjoinStatus, WalletManagerReconcileMessage,
        payjoin::{
            PayjoinActor, PayjoinSessionPersister, SessionResumption, build_sender, resume_session,
        },
    },
    node::{Node, client::NodeClient},
    wallet_lifecycle::TerminalPayjoinPersistenceAuthority,
};

use super::{
    WalletActor,
    broadcast::{
        BroadcastTransactionError, broadcast_to_node_with_connection, transaction_known_to_node,
    },
};

#[derive(Debug)]
pub(crate) enum ActivePayjoin {
    Negotiating { session_id: PayjoinSessionId, actor: Addr<PayjoinActor> },
    Broadcasting { session_id: PayjoinSessionId, outcome: PayjoinBroadcastOutcome },
    RecoveryBlocked { session_id: PayjoinSessionId },
}

impl ActivePayjoin {
    fn session_id(&self) -> &PayjoinSessionId {
        match self {
            Self::Negotiating { session_id, .. }
            | Self::Broadcasting { session_id, .. }
            | Self::RecoveryBlocked { session_id } => session_id,
        }
    }

    fn is_broadcasting(
        &self,
        session_id: &PayjoinSessionId,
        outcome: PayjoinBroadcastOutcome,
    ) -> bool {
        matches!(
            self,
            Self::Broadcasting {
                session_id: active_id,
                outcome: active_outcome,
            } if active_id == session_id && *active_outcome == outcome
        )
    }
}

impl WalletActor {
    /// Resumes a persisted payjoin session from a previous app run, if one exists
    pub async fn resume_payjoin_session(&mut self) -> ActorResult<()> {
        if self.payjoin.is_some() {
            return Produces::ok(());
        }

        match resume_session(self.db.clone(), self.addr.clone()) {
            SessionResumption::None => {}

            SessionResumption::Resume(actor) => self.spawn_payjoin_actor(*actor),

            SessionResumption::BroadcastStoredProposal { session_id, proposal_tx } => {
                self.payjoin = Some(ActivePayjoin::Broadcasting {
                    session_id: session_id.clone(),
                    outcome: PayjoinBroadcastOutcome::Proposal,
                });
                send!(self.addr.handle_payjoin_proposal_broadcast(session_id, proposal_tx));
            }

            SessionResumption::SignRecoveredProposal { session_id, proposal_psbt, fallback_tx } => {
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                send!(self.addr.handle_recovered_payjoin_success(
                    session_id,
                    proposal_psbt,
                    fallback_tx
                ));
            }

            SessionResumption::BroadcastFallback { session_id, fallback_tx } => {
                self.payjoin = Some(ActivePayjoin::Broadcasting {
                    session_id: session_id.clone(),
                    outcome: PayjoinBroadcastOutcome::Fallback,
                });
                send!(self.addr.handle_payjoin_fallback(session_id, fallback_tx));
            }

            SessionResumption::ReportError { session_id: Some(session_id), message } => {
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                self.send_payjoin_failure(session_id, message);
            }

            SessionResumption::ReportError { session_id: None, message } => {
                self.send(WalletManagerReconcileMessage::WalletError(Error::PayjoinSessionError(
                    message,
                )));
            }
        }

        Produces::ok(())
    }

    pub async fn notify_payjoin_error(
        &mut self,
        session_id: PayjoinSessionId,
        message: String,
    ) -> ActorResult<()> {
        if !self.is_active_payjoin(&session_id) {
            return Produces::ok(());
        }

        self.payjoin = Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
        self.send_payjoin_failure(session_id, message);
        Produces::ok(())
    }

    pub async fn cancel_payjoin(
        &mut self,
        requested_id: PayjoinSessionId,
    ) -> ActorResult<Result<(), Error>> {
        let Some(active) = self.payjoin.take() else {
            return Produces::ok(Ok(()));
        };

        let ActivePayjoin::Negotiating { session_id, actor } = active else {
            let active_id = active.session_id().clone();
            self.payjoin = Some(active);

            return if active_id == requested_id {
                Produces::ok(Ok(()))
            } else {
                Produces::ok(Err(Error::PayjoinSessionMismatch {
                    requested: requested_id,
                    active: active_id,
                }))
            };
        };

        if session_id != requested_id {
            self.payjoin =
                Some(ActivePayjoin::Negotiating { session_id: session_id.clone(), actor });
            return Produces::ok(Err(Error::PayjoinSessionMismatch {
                requested: requested_id,
                active: session_id,
            }));
        }

        match call!(actor.cancel_and_fallback()).await {
            Ok(Some(fallback_tx)) => {
                self.payjoin = Some(ActivePayjoin::Broadcasting {
                    session_id: session_id.clone(),
                    outcome: PayjoinBroadcastOutcome::Fallback,
                });
                send!(self.addr.handle_payjoin_fallback(session_id, fallback_tx));
                Produces::ok(Ok(()))
            }

            Ok(None) => {
                self.payjoin = Some(ActivePayjoin::RecoveryBlocked { session_id });
                Produces::ok(Ok(()))
            }

            Err(error) => {
                self.payjoin = Some(ActivePayjoin::Negotiating { session_id, actor });
                Produces::ok(Err(error).map_err_str(Error::PayjoinCancellationFailed))
            }
        }
    }

    pub async fn notify_payjoin_polling_started(
        &mut self,
        session_id: PayjoinSessionId,
        deadline_secs: u64,
    ) -> ActorResult<()> {
        if !self.is_active_payjoin(&session_id) {
            return Produces::ok(());
        }

        self.send(WalletManagerReconcileMessage::PayjoinStatusChanged(PayjoinStatus::Polling {
            session_id,
            deadline_secs,
        }));
        Produces::ok(())
    }

    pub(crate) async fn quiesce_payjoin(
        &mut self,
        terminal_payjoin_authority: Option<TerminalPayjoinPersistenceAuthority>,
    ) -> Result<(), String> {
        match terminal_payjoin_authority {
            Some(authority) => {
                if let Some(active) = self.payjoin.take() {
                    match active {
                        ActivePayjoin::Negotiating { session_id, actor } => {
                            if let Err(error) = call!(
                                actor.cancel_and_fallback_for_terminal_shutdown(authority.clone())
                            )
                            .await
                            {
                                self.payjoin =
                                    Some(ActivePayjoin::Negotiating { session_id, actor });
                                return Err(error.to_string());
                            }
                        }

                        terminal => self.payjoin = Some(terminal),
                    }
                }

                let terminal_transaction = PayjoinSessionPersister::new(self.db.clone())
                    .terminal_transaction_for_shutdown(&authority)
                    .map_err(|error| error.to_string())?;

                if let Some(transaction) = terminal_transaction {
                    // keep node latency outside the destructive shutdown deadline
                    self.schedule_payjoin_terminal_broadcast(transaction);
                }
            }
            None => {
                if let Some(active) = self.payjoin.take() {
                    match active {
                        ActivePayjoin::Negotiating { session_id, actor } => {
                            let fallback = match call!(actor.cancel_and_fallback()).await {
                                Ok(fallback) => fallback,
                                Err(error) => {
                                    self.payjoin =
                                        Some(ActivePayjoin::Negotiating { session_id, actor });
                                    return Err(error.to_string());
                                }
                            };

                            if let Some(fallback) = fallback {
                                self.broadcast_payjoin_terminal_for_shutdown(fallback)
                                    .await
                                    .map_err(|error| error.to_string())?;
                            }
                        }

                        terminal => self.payjoin = Some(terminal),
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn active_payjoin_session_id(&self) -> Option<&PayjoinSessionId> {
        self.payjoin.as_ref().map(ActivePayjoin::session_id)
    }

    fn is_active_payjoin(&self, session_id: &PayjoinSessionId) -> bool {
        self.active_payjoin_session_id() == Some(session_id)
    }

    fn is_active_payjoin_broadcast(
        &self,
        session_id: &PayjoinSessionId,
        outcome: PayjoinBroadcastOutcome,
    ) -> bool {
        self.payjoin.as_ref().is_some_and(|active| active.is_broadcasting(session_id, outcome))
    }

    pub(crate) fn send_payjoin_failure(&self, session_id: PayjoinSessionId, message: String) {
        self.send(WalletManagerReconcileMessage::PayjoinStatusChanged(PayjoinStatus::Failed {
            session_id,
            message,
        }));
    }

    pub(crate) fn prepare_for_payment(&mut self) -> Result<(), Error> {
        if self.payjoin.is_some() {
            return Err(Error::PayjoinSessionError(
                "a payjoin session is already in progress".to_string(),
            ));
        }

        let Some(_) = self.db.get_payjoin_sender_session().map_err(|error| {
            error!("failed to check for pending payjoin session: {error}");
            Error::PayjoinSessionError(
                "unable to verify payjoin session state; please try again later".to_string(),
            )
        })?
        else {
            return Ok(());
        };

        // if the session has a terminal marker and the tx is already in the local wallet,
        // the only remaining work is session cleanup and does not need the network
        let persister = PayjoinSessionPersister::new(self.db.clone());
        let can_cleanup =
            persister.pending_txid().is_some_and(|txid| self.wallet.bdk.get_tx(txid).is_some());

        if !can_cleanup {
            // resume the pending broadcast so the user does not need to restart
            send!(self.addr.resume_payjoin_session());
            return Err(Error::PayjoinSessionError(
                "retrying a previous payjoin broadcast; please try again in a moment".to_string(),
            ));
        }

        // a prior broadcast can apply the tx in memory before the wallet reaches disk
        if let Err(error) = self.wallet.persist() {
            warn!("failed to persist wallet at send gate before payjoin cleanup: {error}");
            return Err(Error::PayjoinSessionError(
                "a previous payjoin session is pending cleanup; please try again later".to_string(),
            ));
        }

        if let Err(error) = self.db.delete_payjoin_sender_session() {
            warn!("payjoin session cleanup at send gate failed: {error}");
            return Err(Error::PayjoinSessionError(
                "a previous payjoin session is pending cleanup; please try again later".to_string(),
            ));
        }

        // the completed tx can be the proposal, while the supplied PSBT is still the fallback
        Err(Error::PayjoinSessionError(
            "previous payjoin session cleared; please confirm your payment again".to_string(),
        ))
    }

    pub(crate) async fn initiate_payjoin_payment(
        &mut self,
        psbt: Psbt,
        intent: PayjoinIntent,
    ) -> Result<(), Error> {
        let PayjoinIntent { session_id, endpoint } = intent;
        let (signed_psbt, fallback_tx) = self.do_sign_original_psbt(psbt).await?;
        let network: bitcoin::Network = self.wallet.network.into();

        // persist the session before the first network request so it survives app restarts
        let persister = PayjoinSessionPersister::new(self.db.clone());
        if let Err(error) = persister.create_session(&fallback_tx, session_id.clone()) {
            warn!("payjoin session could not be persisted, broadcasting fallback tx: {error}");
            self.payjoin = Some(ActivePayjoin::Broadcasting {
                session_id: session_id.clone(),
                outcome: PayjoinBroadcastOutcome::Fallback,
            });
            send!(self.addr.handle_payjoin_fallback(session_id, fallback_tx));
            return Ok(());
        }

        let Ok(sender) = build_sender(
            signed_psbt,
            &fallback_tx,
            endpoint.as_str().to_string(),
            network,
            &persister,
        )
        .inspect_err(|error| warn!("payjoin setup failed, broadcasting fallback tx: {error}")) else {
            self.payjoin = Some(ActivePayjoin::Broadcasting {
                session_id: session_id.clone(),
                outcome: PayjoinBroadcastOutcome::Fallback,
            });
            send!(self.addr.handle_payjoin_fallback(session_id, fallback_tx));
            return Ok(());
        };

        let actor =
            PayjoinActor::new(self.addr.clone(), persister, sender, fallback_tx, session_id);
        self.spawn_payjoin_actor(actor);
        Ok(())
    }

    fn start_payjoin_terminal_broadcast(
        &mut self,
        session_id: PayjoinSessionId,
        outcome: PayjoinBroadcastOutcome,
        tx: BdkTransaction,
    ) {
        if !self.is_active_payjoin(&session_id) {
            return;
        }

        self.payjoin =
            Some(ActivePayjoin::Broadcasting { session_id: session_id.clone(), outcome });
        let connection = self.deferred_node_connection();

        self.addr.send_fut_with(|addr| async move {
            let result = broadcast_payjoin_terminal_with_connection(
                addr.clone(),
                connection,
                session_id.clone(),
                outcome,
                tx,
            )
            .await;
            send!(addr.handle_payjoin_terminal_broadcast_result(session_id, outcome, result));
        });
    }

    fn start_payjoin_fallback_broadcast(
        &mut self,
        session_id: PayjoinSessionId,
        tx: BdkTransaction,
    ) {
        if !self.is_active_payjoin(&session_id) {
            return;
        }

        match self.db.get_payjoin_sender_session() {
            Ok(None) => {}

            Ok(Some(_)) => {
                let persister = PayjoinSessionPersister::new(self.db.clone());
                if let Err(error) = persister.set_pending_fallback() {
                    error!("failed to persist fallback intent before broadcast, aborting: {error}");
                    self.payjoin =
                        Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                    self.send_payjoin_failure(
                        session_id,
                        "failed to persist recovery state; please restart the app".to_string(),
                    );
                    return;
                }
            }

            Err(error) => {
                error!("failed to check for payjoin session before fallback, aborting: {error}");
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                self.send_payjoin_failure(
                    session_id,
                    "failed to persist recovery state; please restart the app".to_string(),
                );
                return;
            }
        }

        self.start_payjoin_terminal_broadcast(session_id, PayjoinBroadcastOutcome::Fallback, tx);
    }

    /// Schedule best-effort broadcast of the exact committed Payjoin transaction
    fn schedule_payjoin_terminal_broadcast(&mut self, tx: BdkTransaction) {
        let node = Database::global().global_config.selected_node();
        let node_client = self.node_client().ok().cloned();

        cove_tokio::task::spawn(async move {
            if let Err(error) = broadcast_payjoin_terminal_with_client(node_client, node, tx).await
            {
                warn!(
                    "failed to broadcast committed Payjoin transaction during destructive wallet shutdown; the durable terminal marker remains for retry: {error}"
                );
            }
        });
    }

    /// Broadcast the exact committed Payjoin transaction
    async fn broadcast_payjoin_terminal_for_shutdown(
        &mut self,
        tx: BdkTransaction,
    ) -> Result<(), Error> {
        let node = Database::global().global_config.selected_node();
        let node_client = self.node_client().ok().cloned();

        broadcast_payjoin_terminal_with_client(node_client, node, tx).await
    }

    fn start_payjoin_proposal_broadcast(
        &mut self,
        session_id: PayjoinSessionId,
        proposal_tx: BdkTransaction,
    ) {
        if !self.is_active_payjoin(&session_id) {
            return;
        }

        let persister = PayjoinSessionPersister::new(self.db.clone());
        if let Err(error) = persister.set_pending_proposal(&proposal_tx) {
            error!("failed to persist proposal broadcast intent, aborting: {error}");
            self.payjoin = Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
            self.send_payjoin_failure(
                session_id,
                "failed to persist recovery state; please restart the app".to_string(),
            );
            return;
        }

        self.start_payjoin_terminal_broadcast(
            session_id,
            PayjoinBroadcastOutcome::Proposal,
            proposal_tx,
        );
    }

    async fn payjoin_terminal_tx_in_wallet(&mut self, txid: Txid) -> ActorResult<bool> {
        Produces::ok(self.wallet.bdk.get_tx(txid).is_some())
    }

    async fn apply_payjoin_terminal_broadcast(
        &mut self,
        session_id: PayjoinSessionId,
        outcome: PayjoinBroadcastOutcome,
        tx: BdkTransaction,
    ) -> ActorResult<Result<(), Error>> {
        use WalletManagerReconcileMessage as Msg;

        if !self.is_active_payjoin_broadcast(&session_id, outcome) {
            return Produces::ok(Ok(()));
        }

        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_else(|e| {
                warn!("System clock skew detected: {e}");
                0
            });

        let txid = tx.compute_txid();

        self.wallet.bdk.apply_unconfirmed_txs([(tx, now)]);

        // keep the session record until wallet state is durable so startup can recover
        if let Err(error) = self.wallet.persist() {
            error!(
                "failed to persist wallet after payjoin broadcast; retaining session record for recovery: {error}"
            );
            return Produces::ok(Err(Error::PayjoinSessionError(
                "transaction was broadcast but wallet state could not be saved; please restart the app"
                    .to_string(),
            )));
        }

        let session_cleared = match self.db.delete_payjoin_sender_session() {
            Ok(()) => true,
            Err(error) => {
                warn!("failed to clear payjoin session record: {error}");
                false
            }
        };

        let balance = self.wallet.balance();
        self.send(Msg::WalletBalanceChanged(balance.into()));

        let transactions = self.do_transactions().await;
        self.send(Msg::UpdatedTransactions(transactions));

        send!(self.addr.start_transaction_watcher(txid));

        if session_cleared {
            self.send(Msg::PayjoinStatusChanged(PayjoinStatus::Broadcast { session_id, outcome }));
            self.payjoin = None;
        } else {
            // tx is broadcast and wallet-persisted, but the session record remains
            // initiate_payment will reject new sends until the record is gone
            // on restart resume_payjoin_session re-dispatches the stored terminal tx
            // because it is already in the wallet, broadcast is skipped and cleanup is retried
            self.payjoin = Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
            self.send_payjoin_failure(
                session_id,
                "transaction was broadcast; restart the app to unlock sending".to_string(),
            );
        }

        Produces::ok(Ok(()))
    }

    async fn handle_payjoin_terminal_broadcast_result(
        &mut self,
        session_id: PayjoinSessionId,
        outcome: PayjoinBroadcastOutcome,
        result: Result<(), BroadcastTransactionError>,
    ) -> ActorResult<()> {
        if !self.is_active_payjoin_broadcast(&session_id, outcome) {
            return Produces::ok(());
        }

        match result {
            Ok(()) => {}

            Err(BroadcastTransactionError::BroadcastFailed(error)) => {
                error!("payjoin broadcast failed: {error}");
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                self.send_payjoin_failure(session_id, error.to_string());
            }

            Err(BroadcastTransactionError::PostBroadcastFailed(error)) => {
                error!("payjoin broadcast bookkeeping failed: {error}");
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                self.send_payjoin_failure(session_id, error.to_string());
            }
        }

        Produces::ok(())
    }

    pub async fn handle_payjoin_success(
        &mut self,
        session_id: PayjoinSessionId,
        proposal_psbt: Psbt,
        fallback_tx: BdkTransaction,
    ) -> ActorResult<()> {
        if !self.is_active_payjoin(&session_id) {
            return Produces::ok(());
        }

        let Ok((_, proposal_tx)) =
            self.do_sign_original_psbt(proposal_psbt).await.inspect_err(|error| {
                error!("failed to sign payjoin proposal, falling back to original tx: {error:?}")
            })
        else {
            self.start_payjoin_fallback_broadcast(session_id, fallback_tx);
            return Produces::ok(());
        };

        self.start_payjoin_proposal_broadcast(session_id, proposal_tx);
        Produces::ok(())
    }

    pub async fn handle_payjoin_proposal_broadcast(
        &mut self,
        session_id: PayjoinSessionId,
        proposal_tx: BdkTransaction,
    ) -> ActorResult<()> {
        self.start_payjoin_terminal_broadcast(
            session_id,
            PayjoinBroadcastOutcome::Proposal,
            proposal_tx,
        );
        Produces::ok(())
    }

    /// Handles a recovered session that closed with a receiver proposal but no stored tx.
    /// On signing failure the session record is retained — the receiver's proposal was valid
    /// and the user should retry after resolving the signing issue.
    /// If the proposal intent cannot be persisted, `fallback_tx` is broadcast instead —
    /// no terminal marker was written so falling back is safe at that point.
    pub async fn handle_recovered_payjoin_success(
        &mut self,
        session_id: PayjoinSessionId,
        proposal_psbt: Psbt,
        fallback_tx: BdkTransaction,
    ) -> ActorResult<()> {
        if !self.is_active_payjoin(&session_id) {
            return Produces::ok(());
        }

        let proposal_tx = match self.do_sign_original_psbt(proposal_psbt).await {
            Ok((_, tx)) => tx,
            Err(error) => {
                error!("failed to sign recovered payjoin proposal, pausing for retry: {error:?}");
                // the receiver accepted a valid proposal; do not fall back — retain the record
                // so the user can retry after resolving the signing failure
                self.payjoin =
                    Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
                self.send_payjoin_failure(
                    session_id,
                    "could not sign the recovered payjoin proposal; unlock the wallet and restart"
                        .to_string(),
                );
                return Produces::ok(());
            }
        };

        let persister = PayjoinSessionPersister::new(self.db.clone());
        if let Err(error) = persister.set_pending_proposal(&proposal_tx) {
            error!("failed to persist recovered proposal intent, falling back: {error}");
            // no terminal marker was written yet; safe to fall back to the original tx
            self.start_payjoin_fallback_broadcast(session_id, fallback_tx);
            return Produces::ok(());
        }

        self.start_payjoin_terminal_broadcast(
            session_id,
            PayjoinBroadcastOutcome::Proposal,
            proposal_tx,
        );
        Produces::ok(())
    }

    pub async fn handle_payjoin_fallback(
        &mut self,
        session_id: PayjoinSessionId,
        fallback_tx: BdkTransaction,
    ) -> ActorResult<()> {
        self.start_payjoin_fallback_broadcast(session_id, fallback_tx);
        Produces::ok(())
    }

    fn spawn_payjoin_actor(&mut self, actor: PayjoinActor) {
        let session_id = actor.session_id.clone();
        self.send(WalletManagerReconcileMessage::PayjoinStatusChanged(
            PayjoinStatus::Negotiating { session_id: session_id.clone() },
        ));
        self.payjoin = Some(ActivePayjoin::Negotiating { session_id, actor: spawn_actor(actor) });
    }
}

async fn broadcast_payjoin_terminal_with_client(
    node_client: Option<NodeClient>,
    node: Node,
    tx: BdkTransaction,
) -> Result<(), Error> {
    let node_client = match node_client {
        Some(node_client) => node_client,
        None => super::node::checked_node_client(&node).await?,
    };
    let txid = tx.compute_txid();

    if let Err(error) = node_client.broadcast_transaction(tx).await {
        let known_to_node = node_client
            .get_transaction(txid)
            .await
            .is_ok_and(|found| found.is_some_and(|transaction| transaction.compute_txid() == txid));
        if !known_to_node {
            return Err(Error::BroadcastError(format!(
                "failed to broadcast committed Payjoin transaction during wallet shutdown: {error:?}"
            )));
        }
    }

    Ok(())
}

async fn broadcast_payjoin_terminal_with_connection(
    addr: WeakAddr<WalletActor>,
    connection: Produces<Result<(), Error>>,
    session_id: PayjoinSessionId,
    outcome: PayjoinBroadcastOutcome,
    transaction: BdkTransaction,
) -> Result<(), BroadcastTransactionError> {
    let txid = transaction.compute_txid();
    let already_in_wallet = call!(addr.payjoin_terminal_tx_in_wallet(txid))
        .await
        .map_err(|_| BroadcastTransactionError::BroadcastFailed(Error::ActorNotFound))?;

    if !already_in_wallet {
        match broadcast_to_node_with_connection(addr.clone(), connection, &transaction).await {
            Ok(()) => {}

            Err(BroadcastTransactionError::BroadcastFailed(error)) => {
                if !transaction_known_to_node(addr.clone(), txid).await {
                    return Err(BroadcastTransactionError::BroadcastFailed(error));
                }
            }

            Err(error) => return Err(error),
        }
    }

    call!(addr.apply_payjoin_terminal_broadcast(session_id, outcome, transaction))
        .await
        .map_err(|_| BroadcastTransactionError::PostBroadcastFailed(Error::ActorNotFound))?
        .map_err(BroadcastTransactionError::PostBroadcastFailed)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::Ordering, time::Duration};

    use act_zero::{call, runtimes::tokio::spawn_actor};
    use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
    use bitcoin::{
        Amount, BlockHash, Psbt, Transaction, absolute::LockTime, hashes::Hash as _,
        transaction::Version,
    };
    use cove_types::PayjoinSessionId;

    use crate::{
        database::wallet_data::{
            PendingAction, WalletDataDb, test_support::new_test_wallet_data_db,
        },
        manager::wallet_manager::{
            Error, PayjoinBroadcastOutcome,
            actor::test_support::{
                actor_value, mark_wallet_ledger_ready, new_test_wallet_actor,
                new_test_wallet_actor_with_db, restore_default_bitcoin_node,
                set_broadcast_esplora_node, set_pending_broadcast_esplora_node,
                test_broadcast_transaction, test_keychain, wait_for_broadcast_request_count,
            },
            payjoin::{PayjoinSessionPersister, test_support::terminal_actor},
        },
        router::UnsignedPaymentMode,
        wallet::Wallet,
        wallet_lifecycle::test_support::begin_wallet_deletion,
    };

    use super::ActivePayjoin;

    fn new_actor_with_db(wallet: Wallet, db: WalletDataDb) -> super::WalletActor {
        let (sender, _receiver) = flume::bounded(10);
        new_test_wallet_actor_with_db(wallet, sender, db)
    }

    fn empty_transaction() -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_payjoin_fallback_waits_for_node_broadcast() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_broadcast_esplora_node(200).await;
        let wallet = Wallet::preview_new_wallet();
        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);

        actor
            .broadcast_payjoin_terminal_for_shutdown(test_broadcast_transaction())
            .await
            .expect("terminal fallback reaches the node before shutdown returns");

        restore_default_bitcoin_node();
        server.server.abort();
        assert_eq!(server.broadcast_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn destructive_quiesce_succeeds_after_terminal_broadcast_failure() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let failed_server = set_broadcast_esplora_node(500).await;
        let wallet = Wallet::preview_new_wallet();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let terminal_tx = test_broadcast_transaction();
        let persister = PayjoinSessionPersister::new(db.clone());
        let session_id = PayjoinSessionId::generate();
        persister.create_session(&terminal_tx, session_id.clone()).unwrap();

        let mut actor = new_actor_with_db(wallet, db.clone());
        actor.payjoin = Some(ActivePayjoin::Negotiating {
            session_id: session_id.clone(),
            actor: spawn_actor(terminal_actor(persister, terminal_tx, session_id)),
        });

        let (authority, preparation) = begin_wallet_deletion(db.id.clone());
        actor_value(actor.quiesce_for_terminal_shutdown(authority).await).await;
        wait_for_broadcast_request_count(&failed_server.broadcast_requests, 1).await;
        drop(preparation);
        failed_server.server.abort();

        assert!(actor.payjoin.is_none(), "the failed broadcast leaves no active child actor");
        assert_eq!(
            db.get_payjoin_sender_session().unwrap().unwrap().pending_action,
            Some(PendingAction::BroadcastFallback),
            "the retry marker must remain after the failed broadcast"
        );

        restore_default_bitcoin_node();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_payjoin_keeps_child_when_fallback_commit_fails() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();

        let wallet = Wallet::preview_new_wallet();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let persister = PayjoinSessionPersister::new(db.clone());
        let session_id = PayjoinSessionId::generate();
        let mut actor = new_actor_with_db(wallet, db);

        actor.payjoin = Some(ActivePayjoin::Negotiating {
            session_id: session_id.clone(),
            actor: spawn_actor(terminal_actor(
                persister,
                test_broadcast_transaction(),
                session_id.clone(),
            )),
        });

        let outcome = actor_value(actor.cancel_payjoin(session_id).await).await;

        assert!(matches!(outcome, Err(Error::PayjoinCancellationFailed(_))));
        assert!(
            matches!(actor.payjoin, Some(ActivePayjoin::Negotiating { .. })),
            "the Payjoin child must remain so cancellation can be retried"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_payjoin_rejects_another_session_without_losing_the_active_child() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();

        let wallet = Wallet::preview_new_wallet();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let persister = PayjoinSessionPersister::new(db.clone());
        let active_id = PayjoinSessionId::generate();
        let requested_id = PayjoinSessionId::generate();
        let mut actor = new_actor_with_db(wallet, db);

        actor.payjoin = Some(ActivePayjoin::Negotiating {
            session_id: active_id.clone(),
            actor: spawn_actor(terminal_actor(
                persister,
                test_broadcast_transaction(),
                active_id.clone(),
            )),
        });

        let outcome = actor_value(actor.cancel_payjoin(requested_id.clone()).await).await;

        assert_eq!(
            outcome,
            Err(Error::PayjoinSessionMismatch {
                requested: requested_id,
                active: active_id.clone(),
            })
        );
        assert_eq!(actor.active_payjoin_session_id(), Some(&active_id));
        assert!(matches!(actor.payjoin, Some(ActivePayjoin::Negotiating { .. })));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn destructive_quiesce_does_not_wait_for_terminal_broadcast() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let pending_server = set_pending_broadcast_esplora_node().await;
        let wallet = Wallet::preview_new_wallet();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let terminal_tx = test_broadcast_transaction();
        let persister = PayjoinSessionPersister::new(db.clone());
        persister.create_session(&terminal_tx, PayjoinSessionId::generate()).unwrap();
        persister.set_pending_fallback().unwrap();

        let mut actor = new_actor_with_db(wallet, db.clone());
        let (authority, preparation) = begin_wallet_deletion(db.id.clone());
        tokio::time::timeout(
            Duration::from_secs(1),
            actor_value(actor.quiesce_for_terminal_shutdown(authority).await),
        )
        .await
        .expect("destructive quiesce does not wait for a pending node request");
        wait_for_broadcast_request_count(&pending_server.broadcast_requests, 1).await;

        pending_server.release.send(true).expect("pending broadcast request is listening");
        drop(preparation);
        pending_server.server.abort();
        restore_default_bitcoin_node();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_gate_retries_pending_terminal_action_and_rejects_new_send() {
        crate::database::test_support::init_test_database();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let fallback_tx = empty_transaction();

        PayjoinSessionPersister::new(db.clone())
            .create_session(&fallback_tx, PayjoinSessionId::generate())
            .unwrap();

        let mut actor = new_actor_with_db(wallet, db);
        let empty_psbt = Psbt::from_unsigned_tx(fallback_tx).unwrap();
        let result = actor.initiate_payment(empty_psbt, UnsignedPaymentMode::Standard).await;
        let outcome = actor_value(result).await;

        assert!(matches!(&outcome, Err(Error::PayjoinSessionError(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_gate_clears_stale_session_when_terminal_tx_already_in_wallet() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();
        test_keychain();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            bdk_wallet::chain::BlockId { height: 1, hash: BlockHash::from_byte_array([4; 32]) },
        );

        let outpoint = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(10_000));
        let terminal_tx =
            (*wallet.bdk.get_tx(outpoint.txid).expect("tx in wallet").tx_node.tx).clone();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let persister = PayjoinSessionPersister::new(db.clone());
        persister.create_session(&terminal_tx, PayjoinSessionId::generate()).unwrap();
        persister.set_pending_fallback().unwrap();

        let mut actor = new_actor_with_db(wallet, db);
        let dummy_psbt = Psbt::from_unsigned_tx(empty_transaction()).unwrap();
        let result = actor.initiate_payment(dummy_psbt, UnsignedPaymentMode::Standard).await;
        let outcome = actor_value(result).await;

        let session = actor.db.get_payjoin_sender_session().expect("db query succeeded");
        assert!(session.is_none(), "gate should have cleared the stale session record");
        assert!(matches!(outcome, Err(Error::PayjoinSessionError(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_terminal_result_cannot_close_the_active_payjoin_session() {
        crate::test_support::ensure_tokio_runtime();

        let wallet = Wallet::preview_new_wallet();
        let wallet_data_db =
            WalletDataDb::new_in_memory(wallet.id.clone()).expect("wallet data database opens");
        let (reconciler, receiver) = flume::bounded(1);
        let mut actor = new_test_wallet_actor_with_db(wallet, reconciler, wallet_data_db);

        let active_id = PayjoinSessionId::generate();
        let stale_id = PayjoinSessionId::generate();
        let transaction = empty_transaction();
        let txid = transaction.compute_txid();

        actor.payjoin = Some(ActivePayjoin::Broadcasting {
            session_id: active_id.clone(),
            outcome: PayjoinBroadcastOutcome::Proposal,
        });

        let outcome = actor_value(
            actor
                .apply_payjoin_terminal_broadcast(
                    stale_id,
                    PayjoinBroadcastOutcome::Proposal,
                    transaction,
                )
                .await,
        )
        .await;

        assert_eq!(outcome, Ok(()));
        assert_eq!(actor.active_payjoin_session_id(), Some(&active_id));
        assert!(actor.wallet.bdk.get_tx(txid).is_none());
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn broadcast_payjoin_terminal_skips_rebroadcast_when_tx_already_in_wallet() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            bdk_wallet::chain::BlockId { height: 1, hash: BlockHash::from_byte_array([5; 32]) },
        );

        let outpoint = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(10_000));
        let terminal_tx =
            (*wallet.bdk.get_tx(outpoint.txid).expect("tx in wallet").tx_node.tx).clone();
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let persister = PayjoinSessionPersister::new(db.clone());
        let session_id = PayjoinSessionId::generate();
        persister.create_session(&terminal_tx, session_id).unwrap();
        persister.set_pending_proposal(&terminal_tx).unwrap();

        let actor = new_actor_with_db(wallet, db.clone());
        let addr = spawn_actor(actor);
        call!(addr.resume_payjoin_session()).await.expect("actor responds");

        for _ in 0..50 {
            let session = db.get_payjoin_sender_session().expect("db query succeeded");
            if session.is_none() {
                return;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("session should be cleared when terminal tx is already in wallet");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recovered_payjoin_signing_failure_retains_session_record() {
        crate::database::test_support::init_test_database();
        test_keychain();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());
        let fallback_tx = empty_transaction();
        let session_id = PayjoinSessionId::generate();

        PayjoinSessionPersister::new(db.clone())
            .create_session(&fallback_tx, session_id.clone())
            .unwrap();

        let mut actor = new_actor_with_db(wallet, db);
        actor.payjoin = Some(ActivePayjoin::RecoveryBlocked { session_id: session_id.clone() });
        let proposal_psbt = Psbt::from_unsigned_tx(empty_transaction()).unwrap();

        let result =
            actor.handle_recovered_payjoin_success(session_id, proposal_psbt, fallback_tx).await;
        actor_value(result).await;

        let session = actor.db.get_payjoin_sender_session().expect("db query succeeded");
        assert!(session.is_some(), "session must be retained when signing fails");
        assert!(
            session.unwrap().pending_action.is_none(),
            "signing failure must not select fallback"
        );
    }
}

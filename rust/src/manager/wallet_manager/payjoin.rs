#![allow(dead_code)]

use act_zero::*;
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{FeeRate as BdkFeeRate, Transaction as BdkTransaction, consensus, params::Params};
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use payjoin::{
    PjParam,
    persist::{OptionalTransitionOutcome, SessionPersister},
    send::{
        ResponseError,
        v2::{
            CreateRequestError, PollingForProposal, SendSession, Sender as V2Sender, SenderBuilder,
            SessionEvent as PayjoinSessionEvent, SessionOutcome, WithReplyKey, replay_event_log,
        },
    },
    uri::v2::PjParam as V2PjParam,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, warn};

use crate::database::wallet_data::{
    PayjoinSenderSession, PendingAction, WalletDataDb, WalletDataError,
};

use super::actor::WalletActor;

mod config;

pub(crate) use config::{OhttpRelays, PayjoinConfig};

/// Persists payjoin session events to the wallet's database so sessions survive app restarts.
///
/// `lock` serialises concurrent calls within a single actor lifetime — the `PayjoinActor`
/// clones this persister for poll tasks, and all clones share the same lock.  Independently
/// constructed instances (e.g. in `WalletActor` terminal handlers) do not share the lock
/// and are safe because the actor is always closed before those handlers run.
#[derive(Debug, Clone)]
pub(crate) struct PayjoinSessionPersister {
    db: WalletDataDb,
    lock: Arc<Mutex<()>>,
}

impl PayjoinSessionPersister {
    pub(crate) fn new(db: WalletDataDb) -> Self {
        Self { db, lock: Arc::new(Mutex::new(())) }
    }

    /// Marks the session as committed to broadcasting the fallback transaction.
    /// Idempotent: `None → BroadcastFallback` and `BroadcastFallback → ok`.
    /// Rejects an attempt to downgrade a `BroadcastProposal` commitment.
    pub(crate) fn set_pending_fallback(&self) -> Result<(), WalletDataError> {
        self.set_pending_fallback_with(|session| self.db.set_payjoin_sender_session(session))
    }

    /// Return the exact committed terminal transaction for destructive shutdown
    ///
    /// If the session has no terminal commitment, this selects and persists the
    /// fallback under the quiescence capability. An existing proposal commitment
    /// is returned unchanged and can never be replaced by the fallback
    pub(crate) fn terminal_transaction_for_shutdown(
        &self,
        authority: &crate::wallet_lifecycle::TerminalPayjoinPersistenceAuthority,
    ) -> Result<Option<BdkTransaction>, WalletDataError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WalletDataError::Save("session lock poisoned".to_string()))?;

        let Some(mut session) = self.db.get_payjoin_sender_session()? else {
            return Ok(None);
        };

        let transaction = match &session.pending_action {
            None => {
                session.pending_action = Some(PendingAction::BroadcastFallback);
                self.db.set_terminal_payjoin_fallback(session.clone(), authority)?;

                session.fallback_tx.as_ref()
            }
            Some(PendingAction::BroadcastFallback) => session.fallback_tx.as_ref(),
            Some(PendingAction::BroadcastProposal { transaction }) => transaction.as_ref(),
        };

        consensus::deserialize(transaction).map(Some).map_err(|error| {
            WalletDataError::Read(format!("committed payjoin transaction is unreadable: {error}"))
        })
    }

    fn set_pending_fallback_with(
        &self,
        save: impl FnOnce(PayjoinSenderSession) -> Result<(), WalletDataError>,
    ) -> Result<(), WalletDataError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WalletDataError::Save("session lock poisoned".to_string()))?;

        let mut session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Save("no payjoin session record".to_string()))?;

        match &session.pending_action {
            None => {}

            Some(PendingAction::BroadcastFallback) => return Ok(()),

            Some(PendingAction::BroadcastProposal { .. }) => {
                return Err(WalletDataError::Save(
                    "cannot overwrite BroadcastProposal commitment with BroadcastFallback"
                        .to_string(),
                ));
            }
        }

        session.pending_action = Some(PendingAction::BroadcastFallback);
        save(session)
    }

    /// Persists the exact consensus-encoded proposal transaction before broadcasting.
    /// Idempotent when called with the same transaction bytes.
    /// Rejects overwriting with a different transaction or any other pending action.
    pub(crate) fn set_pending_proposal(&self, tx: &BdkTransaction) -> Result<(), WalletDataError> {
        let tx_bytes = consensus::serialize(tx);

        let _guard = self
            .lock
            .lock()
            .map_err(|_| WalletDataError::Save("session lock poisoned".to_string()))?;

        let mut session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Save("no payjoin session record".to_string()))?;

        match &session.pending_action {
            None => {}

            Some(PendingAction::BroadcastProposal { transaction })
                if transaction.as_ref() == tx_bytes.as_slice() =>
            {
                return Ok(());
            }

            Some(_) => {
                return Err(WalletDataError::Save(
                    "cannot overwrite existing terminal action with BroadcastProposal".to_string(),
                ));
            }
        }

        session.pending_action =
            Some(PendingAction::BroadcastProposal { transaction: tx_bytes.into() });
        self.db.set_payjoin_sender_session(session)
    }

    /// Creates a fresh session record; errors if a session record already exists.
    /// The caller must clear any existing record before creating a new one.
    pub(crate) fn create_session(
        &self,
        fallback_tx: &BdkTransaction,
    ) -> Result<(), WalletDataError> {
        if self.db.get_payjoin_sender_session()?.is_some() {
            return Err(WalletDataError::Save(
                "payjoin session already exists; clear it before creating a new one".to_string(),
            ));
        }

        let created_at_secs =
            SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        let session = PayjoinSenderSession {
            events: vec![],
            fallback_tx: consensus::serialize(fallback_tx).into(),
            created_at_secs,
            pending_action: None,
        };
        self.db.set_payjoin_sender_session(session)
    }

    /// Returns the txid committed in the session's terminal marker, if any.
    /// Used by the send gate to detect stale-cleanup state without a network call.
    pub(crate) fn pending_txid(&self) -> Option<bitcoin::Txid> {
        let session = self.db.get_payjoin_sender_session().ok()??;
        match &session.pending_action {
            None => None,
            Some(PendingAction::BroadcastFallback) => {
                consensus::deserialize::<BdkTransaction>(session.fallback_tx.as_ref())
                    .ok()
                    .map(|tx| tx.compute_txid())
            }
            Some(PendingAction::BroadcastProposal { transaction }) => {
                consensus::deserialize::<BdkTransaction>(transaction.as_ref())
                    .ok()
                    .map(|tx| tx.compute_txid())
            }
        }
    }
}

impl SessionPersister for PayjoinSessionPersister {
    type InternalStorageError = WalletDataError;
    type SessionEvent = PayjoinSessionEvent;

    fn save_event(&self, event: Self::SessionEvent) -> Result<(), Self::InternalStorageError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WalletDataError::Save("session lock poisoned".to_string()))?;

        let mut session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Save("no payjoin session record".to_string()))?;

        // Reject late events once any terminal intent is committed: an in-flight poll
        // response must not overwrite a BroadcastFallback or BroadcastProposal marker.
        // The shared lock makes this check-then-write atomic with set_pending_fallback
        // and set_pending_proposal within the same actor lifetime.
        if session.pending_action.is_some() {
            return Err(WalletDataError::Save(
                "session has a committed terminal action; rejecting late event".to_string(),
            ));
        }

        let event_json = serde_json::to_string(&event).map_err_str(WalletDataError::Save)?;
        session.events.push(event_json);

        self.db.set_payjoin_sender_session(session)
    }

    fn load(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Self::SessionEvent>>, Self::InternalStorageError> {
        let session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Read("no payjoin session record".to_string()))?;

        let events = session
            .events
            .iter()
            .map(|event| serde_json::from_str(event))
            .collect::<Result<Vec<_>, _>>()
            .map_err_str(WalletDataError::Read)?;

        Ok(Box::new(events.into_iter()))
    }

    // the record is cleared by the wallet actor after the terminal broadcast, so a crash
    // between the library closing the session and the broadcast can still be resumed
    fn close(&self) -> Result<(), Self::InternalStorageError> {
        Ok(())
    }
}

// send a payjoin HTTP request via reqwest and return the response bytes
async fn http_post(
    client: &reqwest::Client,
    req: payjoin::Request,
    timeout: Duration,
) -> Result<Vec<u8>> {
    Ok(client
        .post(&req.url)
        .header("Content-Type", req.content_type)
        .body(req.body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| eyre::eyre!("send failed: {e:?}"))?
        .error_for_status()
        .map_err(|e| eyre::eyre!("relay returned error status: {e:?}"))?
        .bytes()
        .await
        .map_err(|e| eyre::eyre!("body read failed: {e:?}"))?
        .to_vec())
}

/// Why a request could not be carried by any OHTTP relay
#[derive(Debug)]
enum RelayRoundError {
    /// The receiver's pj expiry has passed, so no relay can carry a request for this session
    SessionExpired,
    /// Every relay failed to build or carry the request
    AllRelaysFailed(eyre::Report),
}

// tries each OHTTP relay in shuffled order, returning the first successful (body, context) pair
async fn try_ohttp_relays<C>(
    client: &reqwest::Client,
    relays: &OhttpRelays,
    timeout: Duration,
    build_request: impl Fn(&str) -> Result<(payjoin::Request, C), CreateRequestError>,
) -> Result<(Vec<u8>, C), RelayRoundError> {
    // per-relay failures stay at debug because polls repeat them every tick; callers log
    // the summary returned here
    let relays = relays.shuffled();
    let mut last_err = eyre::eyre!("no OHTTP relays configured");
    for relay in &relays {
        let (req, ctx) = match build_request(relay.as_str()) {
            Ok(pair) => pair,

            // expiry does not depend on the relay, so trying the others cannot help
            Err(e) if e.is_expired() => return Err(RelayRoundError::SessionExpired),

            Err(e) => {
                debug!("payjoin: relay {relay} rejected: {e:?}");
                last_err = eyre::eyre!("{e:?}");
                continue;
            }
        };

        match http_post(client, req, timeout).await {
            Ok(body) => return Ok((body, ctx)),
            Err(e) => {
                debug!("payjoin: relay {relay} request failed: {e:?}");
                last_err = e;
            }
        }
    }

    Err(RelayRoundError::AllRelaysFailed(eyre::eyre!(
        "{} OHTTP relays failed, last error: {last_err}",
        relays.len()
    )))
}

// returns the index of the single external (recipient) output, or Err for batch/self-send PSBTs
fn recipient_output_index(psbt: &Psbt) -> Result<usize> {
    let external: Vec<_> = psbt
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.bip32_derivation.is_empty())
        .map(|(i, _)| i)
        .collect();

    match external.as_slice() {
        [idx] => Ok(*idx),
        [] => Err(eyre::eyre!("no recipient output found in PSBT")),
        outputs => Err(eyre::eyre!(
            "payjoin not supported for batch sends ({} external outputs)",
            outputs.len()
        )),
    }
}

/// Builds the v2 sender state machine from a signed PSBT and payjoin endpoint
pub(crate) fn build_sender(
    signed_psbt: Psbt,
    fallback_tx: &BdkTransaction,
    endpoint: String,
    network: bitcoin::Network,
    persister: &PayjoinSessionPersister,
) -> Result<V2Sender<WithReplyKey>> {
    // TODO: anti-probing (inputs_seen), verify our inputs have not appeared in a prior session
    // TODO: surface payjoin downgrade to the user when the fallback tx is broadcast instead of the proposal

    let pj_param = v2_pj_param(&endpoint)?;

    let idx = recipient_output_index(&signed_psbt)?;
    let txout = &signed_psbt.unsigned_tx.output[idx];
    let amount = txout.value;
    let address = bitcoin::Address::from_script(&txout.script_pubkey, Params::from(network))
        .map_err(|e| eyre::eyre!("could not derive address from PSBT output: {e:?}"))?;

    // use the signed tx weight (includes witness bytes) so the fee floor isn't inflated for SegWit
    let fee_rate = signed_psbt
        .fee()
        .ok()
        .and_then(|fee| {
            let wu = fallback_tx.weight().to_wu();
            fee.to_sat()
                .checked_mul(1000)
                .and_then(|sat_per_kwu| sat_per_kwu.checked_div(wu))
                .map(BdkFeeRate::from_sat_per_kwu)
        })
        .unwrap_or(BdkFeeRate::BROADCAST_MIN);

    let sender = SenderBuilder::from_parts(signed_psbt, &pj_param, &address, Some(amount))
        .always_disable_output_substitution()
        .build_recommended(fee_rate)
        .map_err(|e| eyre::eyre!("failed to build payjoin sender: {e:?}"))?
        .save(persister)
        .map_err(|e| eyre::eyre!("failed to persist payjoin session: {e:?}"))?;

    Ok(sender)
}

// parses the receiver's pj endpoint, accepting only BIP77 v2 OHTTP endpoints
fn v2_pj_param(endpoint: &str) -> Result<V2PjParam> {
    let pj_param = PjParam::parse(endpoint)
        .map_err(|e| eyre::eyre!("payjoin endpoint is not a BIP77 v2 endpoint: {e:?}"))?;

    // PjParam is non_exhaustive and only v2 is compiled in, so any other variant is a
    // version this sender cannot pay
    match pj_param {
        PjParam::V2(pj_param) => Ok(pj_param),
        _ => Err(eyre::eyre!("only BIP77 v2 OHTTP payjoin endpoints are supported")),
    }
}

// polls the payjoin directory for a receiver proposal; called from begin_poll via send_fut_with
async fn do_poll(
    addr: WeakAddr<PayjoinActor>,
    polling_sender: V2Sender<PollingForProposal>,
    persister: PayjoinSessionPersister,
    relays: OhttpRelays,
) {
    let client = match cove_http::new_client() {
        Ok(c) => c,
        Err(e) => {
            warn!("payjoin poll: failed to create HTTP client: {e:?}");
            send!(addr.complete_with_fallback_msg());
            return;
        }
    };

    // polls are long-polling: the directory holds the connection open until a
    // proposal arrives or its own timeout fires, so allow slightly more than
    // the server-side timeout to avoid racing it
    let (poll_response, poll_ctx) =
        match try_ohttp_relays(&client, &relays, Duration::from_secs(35), |relay| {
            polling_sender.create_poll_request(relay)
        })
        .await
        {
            Ok(pair) => pair,
            Err(RelayRoundError::SessionExpired) => {
                warn!("payjoin poll: receiver's session expired, broadcasting fallback");
                send!(addr.complete_with_fallback_msg());
                return;
            }
            Err(RelayRoundError::AllRelaysFailed(e)) => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                send!(addr.retry_poll_after_failure(PollFailure::RelaysUnreachable, e.to_string()));
                return;
            }
        };

    match polling_sender.process_response(&poll_response, poll_ctx).save(&persister) {
        Ok(OptionalTransitionOutcome::Progress(proposal_psbt)) => {
            send!(addr.complete_with_success(proposal_psbt));
        }

        Ok(OptionalTransitionOutcome::Stasis(next)) => {
            debug!("payjoin poll: no proposal yet, continuing");
            send!(addr.update_polling_sender(next));
        }

        // the library has persisted Closed(Aborted), so the session cannot continue
        Err(e) if e.is_fatal() => {
            // receiver-provided data is untrusted: a well-known error's Display can include the
            // receiver's supported_versions list and validation errors can echo bytes of the
            // receiver's PSBT, such as an invalid key, so only the error code or kind is logged
            match e.api_error_ref() {
                Some(ResponseError::WellKnown(well_known)) => {
                    let code = well_known.code();
                    warn!("payjoin poll: receiver rejected session with code {code}");
                }
                Some(ResponseError::Validation(_)) => {
                    warn!("payjoin poll: receiver proposal failed validation");
                }
                Some(ResponseError::Unrecognized { .. }) => {
                    warn!("payjoin poll: receiver rejected session with an unrecognized error");
                }
                Some(_) | None => warn!("payjoin poll: session failed"),
            }

            send!(addr.complete_with_fallback_msg());
        }

        // nothing was persisted, so the same polling state retries until the deadline
        Err(e) if e.is_transient() => {
            send!(addr.retry_poll_after_failure(
                PollFailure::InvalidResponse,
                "transient directory response error".to_string()
            ));
        }

        Err(e) => {
            let (failure, detail) = match e.storage_error_ref() {
                Some(storage) => (PollFailure::Storage, storage.to_string()),
                None => (PollFailure::Other, "unclassified poll error".to_string()),
            };

            send!(addr.retry_poll_after_failure(failure, detail));
        }
    }
}

/// Tracks which phase of the BIP-77 v2 negotiation the actor is currently in
enum PayjoinSession {
    /// Initial POST has not been sent yet; holds the sender state machine ready to POST
    PrePost { sender: V2Sender<WithReplyKey> },
    /// POST request is in flight, so the sender state machine is owned by the request future
    Posting,
    /// POST was accepted by the directory; now polling for the receiver's proposal
    Polling { polling_sender: V2Sender<PollingForProposal> },
    /// Session is canceled or terminal; late async completions must be ignored
    Closed,
}

/// Why the last directory poll failed and will be retried
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollFailure {
    RelaysUnreachable,
    InvalidResponse,
    Storage,
    Other,
}

pub(crate) struct PayjoinActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    persister: PayjoinSessionPersister,
    config: PayjoinConfig,
    fallback_tx: BdkTransaction,
    session: PayjoinSession,
    poll_deadline: Option<Instant>,
    /// Kept so a failure that repeats on every poll logs once until it changes or clears
    last_poll_failure: Option<PollFailure>,
}

impl PayjoinActor {
    pub(crate) fn new(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        config: PayjoinConfig,
        sender: V2Sender<WithReplyKey>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            config,
            fallback_tx,
            session: PayjoinSession::PrePost { sender },
            poll_deadline: None,
            last_poll_failure: None,
        }
    }

    /// Recreates the actor from a replayed session that was already polling for a proposal
    pub(crate) fn resume_polling(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        config: PayjoinConfig,
        polling_sender: V2Sender<PollingForProposal>,
        fallback_tx: BdkTransaction,
        created_at_secs: Option<u64>,
    ) -> Self {
        let session_timeout = config.session_timeout();
        let poll_deadline = Some(match created_at_secs {
            None => Instant::now() + session_timeout,

            Some(start) => {
                let now_secs =
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                let elapsed = Duration::from_secs(now_secs.saturating_sub(start));
                Instant::now() + session_timeout.saturating_sub(elapsed)
            }
        });

        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            config,
            fallback_tx,
            session: PayjoinSession::Polling { polling_sender },
            poll_deadline,
            last_poll_failure: None,
        }
    }
}

#[async_trait::async_trait]
impl Actor for PayjoinActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        match &self.session {
            PayjoinSession::PrePost { .. } => send!(addr.start_post()),
            PayjoinSession::Polling { .. } => send!(addr.begin_poll()),
            _ => {}
        }
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("PayjoinActor error: {error:?}");
        self.complete_with_fallback();
        false
    }
}

impl PayjoinActor {
    /// POSTs the signed PSBT to the payjoin directory and transitions to polling
    pub(crate) async fn start_post(&mut self) -> ActorResult<()> {
        let sender = match std::mem::replace(&mut self.session, PayjoinSession::Posting) {
            PayjoinSession::PrePost { sender } => sender,
            PayjoinSession::Closed => {
                self.session = PayjoinSession::Closed;
                return Produces::ok(());
            }
            unexpected => {
                self.session = unexpected;
                warn!("payjoin start_post called in unexpected state");
                return Produces::ok(());
            }
        };

        let persister = self.persister.clone();
        let relays = self.config.relays().clone();

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin POST: failed to create HTTP client: {e:?}");
                    send!(addr.complete_with_fallback_msg());
                    return;
                }
            };

            let (post_response, post_ctx) =
                match try_ohttp_relays(&client, &relays, Duration::from_secs(30), |relay| {
                    sender.create_v2_post_request(relay)
                })
                .await
                {
                    Ok(pair) => pair,
                    Err(RelayRoundError::SessionExpired) => {
                        warn!("payjoin POST: receiver's session already expired");
                        send!(addr.complete_with_fallback_msg());
                        return;
                    }
                    Err(RelayRoundError::AllRelaysFailed(e)) => {
                        warn!("payjoin POST: all relays failed: {e}");
                        send!(addr.complete_with_fallback_msg());
                        return;
                    }
                };

            let polling_sender =
                match sender.process_response(&post_response, post_ctx).save(&persister) {
                    Ok(ps) => ps,
                    // Display, not Debug: a transient error carries the sender state, whose
                    // Debug output includes the whole signed Original PSBT
                    Err(e) => {
                        warn!("payjoin POST: failed to process response: {e}");
                        send!(addr.complete_with_fallback_msg());
                        return;
                    }
                };

            send!(addr.post_succeeded(polling_sender));
        });

        Produces::ok(())
    }

    /// Stores the polling sender, sets the session deadline, and begins polling
    pub(crate) async fn post_succeeded(
        &mut self,
        polling_sender: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        if !matches!(self.session, PayjoinSession::Posting) {
            warn!("payjoin post_succeeded called in unexpected state");
            return Produces::ok(());
        }

        self.poll_deadline = Some(Instant::now() + self.config.session_timeout());
        self.session = PayjoinSession::Polling { polling_sender };
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Poll the directory for the receiver's proposal PSBT
    pub(crate) async fn begin_poll(&mut self) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        // if the session deadline has passed, broadcast the fallback immediately
        if self.poll_deadline.is_some_and(|d| Instant::now() >= d) {
            let last_failure = self.last_poll_failure;
            warn!(
                "payjoin session deadline passed without a proposal, broadcasting fallback, last poll failure: {last_failure:?}"
            );
            self.complete_with_fallback();
            return Produces::ok(());
        }

        // clone so the original stays in self.session, allowing retry if all relays fail this tick
        let polling_sender = match &self.session {
            PayjoinSession::Polling { polling_sender } => polling_sender.clone(),
            _ => {
                warn!("payjoin begin_poll called in unexpected state");
                return Produces::ok(());
            }
        };

        let persister = self.persister.clone();
        let relays = self.config.relays().clone();
        self.addr.send_fut_with(|addr| do_poll(addr, polling_sender, persister, relays));

        Produces::ok(())
    }

    /// Updates the polling sender on Stasis and continues polling
    pub(crate) async fn update_polling_sender(
        &mut self,
        next: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        if !matches!(self.session, PayjoinSession::Polling { .. }) {
            warn!("payjoin update_polling_sender called in unexpected state");
            return Produces::ok(());
        }

        if let Some(failure) = self.last_poll_failure.take() {
            info!("payjoin poll recovered after {failure:?}");
        }

        self.session = PayjoinSession::Polling { polling_sender: next };
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Logs a poll failure when it differs from the last one, then polls again
    async fn retry_poll_after_failure(
        &mut self,
        failure: PollFailure,
        detail: String,
    ) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        if self.last_poll_failure != Some(failure) {
            warn!(
                "payjoin poll failed ({failure:?}), retrying until the session deadline: {detail}"
            );
            self.last_poll_failure = Some(failure);
        }

        self.begin_next_poll();
        Produces::ok(())
    }

    fn begin_next_poll(&mut self) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            send!(addr.begin_poll());
        });
    }

    /// Cancel the session and return its committed fallback transaction
    pub(crate) async fn cancel_and_fallback(&mut self) -> ActorResult<Option<BdkTransaction>> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(None);
        }

        self.persister
            .set_pending_fallback()
            .map_err(|error| act_zero::ActorError::from(error.to_string()))?;
        self.close_session();
        Produces::ok(Some(self.fallback_tx.clone()))
    }

    /// Commit the exact terminal transaction under the wallet shutdown capability
    pub(crate) async fn cancel_and_fallback_for_terminal_shutdown(
        &mut self,
        authority: crate::wallet_lifecycle::TerminalPayjoinPersistenceAuthority,
    ) -> ActorResult<Option<BdkTransaction>> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(None);
        }

        let terminal_transaction = self
            .persister
            .terminal_transaction_for_shutdown(&authority)
            .map_err(|error| act_zero::ActorError::from(error.to_string()))?;
        let Some(terminal_transaction) = terminal_transaction else {
            return Err(act_zero::ActorError::from(
                "active payjoin session has no persisted recovery data".to_string(),
            ));
        };

        self.close_session();
        Produces::ok(Some(terminal_transaction))
    }

    async fn complete_with_success(&mut self, proposal_psbt: Psbt) -> ActorResult<()> {
        if !self.close_session() {
            return Produces::ok(());
        }

        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_success(proposal_psbt, fallback_tx));
        Produces::ok(())
    }

    async fn complete_with_fallback_msg(&mut self) -> ActorResult<()> {
        self.complete_with_fallback();
        Produces::ok(())
    }

    fn complete_with_fallback(&mut self) {
        if matches!(self.session, PayjoinSession::Closed) {
            return;
        }

        // Persist fallback intent before dispatching so a crash between here and the
        // actual broadcast causes the next startup to go straight to fallback instead
        // of replaying negotiation events and resuming polling or re-posting.
        // Fail closed: do not dispatch if the intent cannot be persisted — the next
        // restart will replay the event log and expire into a fallback via the deadline.
        if let Err(error) = self.persister.set_pending_fallback() {
            error!(
                "failed to persist fallback intent, aborting fallback dispatch to preserve recovery state: {error}"
            );
            send!(self.wallet_addr.notify_payjoin_error(
                "payment is paused — please restart the app to complete or cancel it".to_string()
            ));
            return;
        }

        self.dispatch_persisted_fallback();
    }

    fn dispatch_persisted_fallback(&mut self) {
        if !self.close_session() {
            return;
        }

        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_fallback(fallback_tx));
    }

    fn close_session(&mut self) -> bool {
        if matches!(self.session, PayjoinSession::Closed) {
            return false;
        }

        self.session = PayjoinSession::Closed;
        self.poll_deadline = None;
        true
    }
}

/// What to do with a persisted payjoin session found on startup
pub(crate) enum SessionResumption {
    /// No session was persisted
    None,
    /// Session is still in flight, spawn this actor to continue it
    Resume(Box<PayjoinActor>),
    /// A BroadcastProposal marker was stored: broadcast this exact consensus-encoded tx
    BroadcastStoredProposal { proposal_tx: BdkTransaction },
    /// Session closed with a success outcome but no stored tx: sign the recovered PSBT and
    /// broadcast.  `fallback_tx` is carried so that if persisting the proposal intent fails,
    /// the actor can fall back to the original transaction.  A signing failure retains the
    /// session without selecting the fallback — the user must retry.
    SignRecoveredProposal { proposal_psbt: Psbt, fallback_tx: BdkTransaction },
    /// Session ended without a proposal, broadcast the original tx
    BroadcastFallback { fallback_tx: BdkTransaction },
    /// The session data is unreadable or the session is in an unrecoverable state; the user
    /// must be shown `message`.  Whether the record is retained or cleared depends on which
    /// producer returned this variant.
    ReportError { message: String },
}

/// Replays a persisted payjoin session from the wallet's database, if one exists
pub(crate) fn resume_session(
    db: WalletDataDb,
    wallet_addr: WeakAddr<WalletActor>,
    config: &PayjoinConfig,
) -> SessionResumption {
    let record = match db.get_payjoin_sender_session() {
        Ok(Some(record)) => record,
        Ok(None) => return SessionResumption::None,
        Err(error) => {
            error!("failed to read payjoin session record: {error}");
            return SessionResumption::ReportError {
                message: "could not read payjoin session; reopen the wallet to retry".to_string(),
            };
        }
    };

    // A crash after complete_with_fallback but before the broadcast completed would leave
    // non-terminal events in the log; honour the stored intent instead of replaying them.
    if matches!(record.pending_action, Some(PendingAction::BroadcastFallback)) {
        return fallback_from_record(&db, record);
    }

    // A crash after set_pending_proposal but before the broadcast completed means the
    // proposal tx is committed — re-broadcast it without re-signing instead of replaying.
    if matches!(record.pending_action, Some(PendingAction::BroadcastProposal { .. })) {
        return proposal_from_record(&db, record);
    }

    let persister = PayjoinSessionPersister::new(db.clone());
    let (session, history) = match replay_event_log(&persister) {
        Ok(pair) => pair,
        Err(error) if error.is_expired() => {
            warn!("payjoin session expired before resume, broadcasting fallback");
            return fallback_from_record(&db, record);
        }
        // a replay error's Display and Debug both print the session events, which hold the
        // signed Original PSBT, so only the fact of the failure is logged
        Err(_) => {
            warn!("payjoin session log could not be replayed, broadcasting fallback");
            return fallback_from_record(&db, record);
        }
    };

    let fallback_tx = history.fallback_tx();
    match session {
        SendSession::WithReplyKey(_) => {
            // The POST may have already reached the directory before the crash;
            // re-posting is not retry-safe so fall back to the original transaction.
            SessionResumption::BroadcastFallback { fallback_tx }
        }

        SendSession::PollingForProposal(polling_sender) => {
            let now_secs =
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let elapsed = record
                .created_at_secs
                .map(|start| Duration::from_secs(now_secs.saturating_sub(start)));

            if elapsed.is_some_and(|e| e >= config.session_timeout()) {
                warn!("payjoin session expired before resume, broadcasting fallback");
                return fallback_from_record(&db, record);
            }

            SessionResumption::Resume(Box::new(PayjoinActor::resume_polling(
                wallet_addr,
                persister,
                config.clone(),
                polling_sender,
                fallback_tx,
                record.created_at_secs,
            )))
        }

        // Cove never cancels through the library, but a replayed Cancelled event still means
        // the original transaction is the one to broadcast
        SendSession::PendingFallback(_) => SessionResumption::BroadcastFallback { fallback_tx },

        SendSession::Closed(SessionOutcome::Success(proposal_psbt)) => {
            SessionResumption::SignRecoveredProposal { proposal_psbt, fallback_tx }
        }

        SendSession::Closed(SessionOutcome::Aborted) => {
            SessionResumption::BroadcastFallback { fallback_tx }
        }
    }
}

/// Recovers the fallback tx from the session record when replay is not possible.
fn fallback_from_record(db: &WalletDataDb, record: PayjoinSenderSession) -> SessionResumption {
    match consensus::deserialize(record.fallback_tx.as_ref()) {
        Ok(fallback_tx) => SessionResumption::BroadcastFallback { fallback_tx },

        Err(error) if matches!(record.pending_action, Some(PendingAction::BroadcastFallback)) => {
            // Marker was written before broadcast, so the fallback may have reached the node.
            // Retain the record — we can't know the outcome.
            error!(
                "committed payjoin fallback tx is unreadable; retaining session record: {error}"
            );
            SessionResumption::ReportError {
                message: "payjoin fallback data is unreadable — check your transaction history before retrying".to_string(),
            }
        }

        Err(error) => {
            // No terminal marker means the fallback was never dispatched; safe to clear.
            error!("payjoin fallback tx is corrupt, clearing session: {error}");
            if let Err(delete_error) = db.delete_payjoin_sender_session() {
                warn!("failed to clear corrupt payjoin session record: {delete_error}");
            }
            SessionResumption::ReportError {
                message: "saved payjoin recovery data was unreadable; the payment was not sent"
                    .to_string(),
            }
        }
    }
}

/// Recovers the proposal tx committed in a `BroadcastProposal` marker so it can be
/// re-broadcast on the next startup without re-signing.
///
/// If the stored bytes are corrupt the record is **retained** and `ReportError` is returned.
/// We cannot know whether that proposal reached the network before the corruption/restart,
/// so deleting the record or broadcasting the fallback would be unsafe.
fn proposal_from_record(_db: &WalletDataDb, record: PayjoinSenderSession) -> SessionResumption {
    let tx_bytes = match &record.pending_action {
        Some(PendingAction::BroadcastProposal { transaction }) => transaction.as_ref().to_vec(),
        _ => {
            error!(
                "proposal_from_record called without BroadcastProposal action; retaining record"
            );
            return SessionResumption::ReportError {
                message: "payjoin session state is inconsistent; check your transaction history before retrying".to_string(),
            };
        }
    };
    match consensus::deserialize::<BdkTransaction>(&tx_bytes) {
        Ok(proposal_tx) => SessionResumption::BroadcastStoredProposal { proposal_tx },
        Err(error) => {
            error!(
                "stored payjoin proposal tx is corrupt; retaining record for manual recovery: {error}"
            );
            SessionResumption::ReportError {
                message: "payjoin proposal data is unreadable — check your transaction history before retrying".to_string(),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn terminal_actor(
        persister: PayjoinSessionPersister,
        fallback_tx: BdkTransaction,
    ) -> PayjoinActor {
        PayjoinActor {
            addr: WeakAddr::default(),
            wallet_addr: WeakAddr::default(),
            persister,
            config: PayjoinConfig::default(),
            fallback_tx,
            session: PayjoinSession::Posting,
            poll_deadline: Some(Instant::now()),
            last_poll_failure: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::wallet_data::test_support, wallet::metadata::WalletId};
    use bitcoin::{
        Amount, ScriptBuf, Transaction, TxOut, psbt::Output as PsbtOutput, transaction::Version,
    };

    // in-memory storage skips the process-global lifecycle coordinator, so these
    // persister tests cannot race with wipe-phase tests in the same cargo test process
    fn new_test_persister() -> (PayjoinSessionPersister, WalletDataDb) {
        let db = WalletDataDb::new_in_memory(WalletId::preview_new_random())
            .expect("in-memory payjoin test db");
        (PayjoinSessionPersister::new(db.clone()), db)
    }

    // persistent storage goes through begin_persistence_operation; callers must
    // hold global_state_test_lock so wipe-phase tests cannot observe CoordinatorBusy
    fn new_persistent_test_persister() -> (PayjoinSessionPersister, WalletDataDb, tempfile::TempDir)
    {
        let (db, tmp) = test_support::new_test_wallet_data_db(WalletId::preview_new_random());
        (PayjoinSessionPersister::new(db.clone()), db, tmp)
    }

    fn test_fallback_tx() -> BdkTransaction {
        BdkTransaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        }
    }

    // secp256k1 generator point G — a well-known valid compressed public key (33 bytes)
    const SECP_G: [u8; 33] = [
        0x02, 0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87,
        0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b, 0x16,
        0xf8, 0x17, 0x98,
    ];

    fn make_psbt(outputs: Vec<(TxOut, bool)>) -> Psbt {
        let tx_outputs: Vec<TxOut> = outputs.iter().map(|(o, _)| o.clone()).collect();
        let unsigned_tx = Transaction {
            version: Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: tx_outputs,
        };
        let mut psbt = Psbt::from_unsigned_tx(unsigned_tx).expect("valid tx");
        for (i, (_, is_external)) in outputs.iter().enumerate() {
            if *is_external {
                psbt.outputs[i] = PsbtOutput::default();
            } else {
                let mut out = PsbtOutput::default();
                out.bip32_derivation.insert(
                    bitcoin::secp256k1::PublicKey::from_slice(&SECP_G).expect("valid pubkey"),
                    Default::default(),
                );
                psbt.outputs[i] = out;
            }
        }
        psbt
    }

    fn empty_transaction() -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        }
    }

    fn terminal_test_actor(persister: PayjoinSessionPersister) -> PayjoinActor {
        super::test_support::terminal_actor(persister, empty_transaction())
    }

    async fn assert_terminal_preparation_persists_fallback(full_wipe: bool) {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();

        let (persister, db, _tmp) = new_persistent_test_persister();
        persister.create_session(&test_fallback_tx()).expect("payjoin session is persisted");
        let wallet_id = db.id.clone();
        let actor = cove_tokio::task::spawn_actor(terminal_test_actor(persister));
        let (authority, preparation) = if full_wipe {
            crate::wallet_lifecycle::test_support::begin_full_wipe(wallet_id)
        } else {
            crate::wallet_lifecycle::test_support::begin_wallet_deletion(wallet_id)
        };

        call!(actor.cancel_and_fallback_for_terminal_shutdown(authority))
            .await
            .expect("terminal Payjoin actor persists fallback across its actor task");

        let session = db.get_payjoin_sender_session().unwrap().unwrap();
        assert_eq!(session.pending_action, Some(PendingAction::BroadcastFallback));
        drop(preparation);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_payjoin_actor_persists_fallback_during_wallet_deletion() {
        assert_terminal_preparation_persists_fallback(false).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_payjoin_actor_persists_fallback_during_full_wipe() {
        assert_terminal_preparation_persists_fallback(true).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_shutdown_preserves_exact_proposal_commitment() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();

        let (persister, db, _tmp) = new_persistent_test_persister();
        let fallback = test_fallback_tx();
        let proposal = BdkTransaction {
            version: Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        };
        persister.create_session(&fallback).unwrap();
        persister.set_pending_proposal(&proposal).unwrap();
        let (authority, preparation) =
            crate::wallet_lifecycle::test_support::begin_wallet_deletion(db.id.clone());

        let terminal = persister
            .terminal_transaction_for_shutdown(&authority)
            .expect("stored proposal remains readable")
            .expect("terminal transaction is present");

        assert_eq!(terminal, proposal);
        assert!(matches!(
            db.get_payjoin_sender_session().unwrap().unwrap().pending_action,
            Some(PendingAction::BroadcastProposal { transaction })
                if transaction.as_ref().as_slice() == consensus::serialize(&proposal)
        ));
        drop(preparation);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_payjoin_persistence_failure_keeps_session_retryable() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();

        let (persister, db, _tmp) = new_persistent_test_persister();
        let actor = cove_tokio::task::spawn_actor(terminal_test_actor(persister.clone()));
        let (authority, preparation) =
            crate::wallet_lifecycle::test_support::begin_wallet_deletion(db.id.clone());

        assert!(
            call!(actor.cancel_and_fallback_for_terminal_shutdown(authority)).await.is_err(),
            "missing recovery data must fail quiescence"
        );
        assert!(db.get_payjoin_sender_session().unwrap().is_none());
        drop(preparation);

        persister.create_session(&test_fallback_tx()).expect("recovery data is restored");
        call!(actor.cancel_and_fallback())
            .await
            .expect("the retained Payjoin actor retries after persistence recovers");

        let session = db.get_payjoin_sender_session().unwrap().unwrap();
        assert_eq!(session.pending_action, Some(PendingAction::BroadcastFallback));
    }

    #[test]
    fn test_close_session_is_idempotent() {
        let (persister, _db) = new_test_persister();
        let mut actor = PayjoinActor {
            addr: WeakAddr::default(),
            wallet_addr: WeakAddr::default(),
            persister,
            config: PayjoinConfig::default(),
            fallback_tx: empty_transaction(),
            session: PayjoinSession::Posting,
            poll_deadline: Some(Instant::now()),
            last_poll_failure: None,
        };

        assert!(actor.close_session());
        assert!(matches!(actor.session, PayjoinSession::Closed));
        assert!(actor.poll_deadline.is_none());
        assert!(!actor.close_session());
    }

    #[test]
    fn test_recipient_output_index_accepts_single_recipient() {
        let psbt = make_psbt(vec![
            (TxOut { value: Amount::from_sat(50_000), script_pubkey: ScriptBuf::default() }, true),
            (TxOut { value: Amount::from_sat(49_000), script_pubkey: ScriptBuf::default() }, false),
        ]);
        assert_eq!(recipient_output_index(&psbt).unwrap(), 0);
    }

    #[test]
    fn test_recipient_output_index_rejects_batch_send() {
        let psbt = make_psbt(vec![
            (TxOut { value: Amount::from_sat(50_000), script_pubkey: ScriptBuf::default() }, true),
            (TxOut { value: Amount::from_sat(50_000), script_pubkey: ScriptBuf::default() }, true),
        ]);
        let err = recipient_output_index(&psbt).unwrap_err();
        assert!(err.to_string().contains("batch"), "expected 'batch' in error, got: {err}");
    }

    #[test]
    fn test_recipient_output_index_rejects_no_recipient() {
        let psbt = make_psbt(vec![(
            TxOut { value: Amount::from_sat(99_000), script_pubkey: ScriptBuf::default() },
            false,
        )]);
        let err = recipient_output_index(&psbt).unwrap_err();
        assert!(err.to_string().contains("recipient"), "expected 'recipient' in error, got: {err}");
    }

    // valid BIP77 v2 endpoint from the payjoin crate's session test vectors, in the form
    // cove_types stores it after parsing a pj= URI
    const V2_ENDPOINT: &str = "https://payjo.in/TXJCGKTKXLUUZ#EX1C4UC6ES-OH1QYPM5JXYNS754Y4R45QWE336QFX6ZR8DQGVQCULVZTV20TFVEYDMFQC-RK1Q0DJS3VVDXWQQTLQ8022QGXSX7ML9PHZ6EDSF6AKEWQG758JPS2EV";

    #[test]
    fn v2_pj_param_accepts_a_bip77_endpoint() {
        let pj_param = v2_pj_param(V2_ENDPOINT).expect("v2 endpoint parses");

        assert_eq!(PjParam::V2(pj_param).endpoint(), V2_ENDPOINT);
    }

    #[test]
    fn v2_pj_param_rejects_a_bip78_endpoint() {
        assert!(v2_pj_param("https://example.com/payjoin").is_err());
    }

    #[test]
    fn v2_pj_param_rejects_a_stripped_fragment() {
        let (without_fragment, _) = V2_ENDPOINT.split_once('#').unwrap();

        assert!(v2_pj_param(without_fragment).is_err());
    }

    #[test]
    fn save_event_requires_a_session_record() {
        let (persister, _db) = new_test_persister();

        let result = persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt());

        assert!(result.is_err());
    }

    #[test]
    fn events_round_trip_in_order() {
        let (persister, _db) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();

        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();
        persister.save_event(PayjoinSessionEvent::Closed(SessionOutcome::Aborted)).unwrap();

        let events: Vec<_> = persister.load().unwrap().collect();
        assert_eq!(
            events,
            vec![
                PayjoinSessionEvent::PostedOriginalPsbt(),
                PayjoinSessionEvent::Closed(SessionOutcome::Aborted)
            ]
        );
    }

    #[test]
    fn create_session_rejects_when_session_exists() {
        let (persister, _db) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        let result = persister.create_session(&test_fallback_tx());

        assert!(result.is_err(), "expected error when session already exists");
        assert_eq!(persister.load().unwrap().count(), 1, "existing session must be unchanged");
    }

    #[test]
    fn close_keeps_the_session_record() {
        let (persister, _db) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        persister.close().unwrap();

        assert_eq!(persister.load().unwrap().count(), 1);
    }

    #[test]
    fn resume_with_no_record_is_none() {
        let (_persister, db) = new_test_persister();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        assert!(matches!(resumption, SessionResumption::None));
    }

    #[test]
    fn resume_with_unreplayable_log_broadcasts_stored_fallback() {
        let (persister, db) = new_test_persister();
        let tx = test_fallback_tx();
        persister.create_session(&tx).unwrap();
        // a log that does not start with a Created event cannot be replayed
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastFallback { fallback_tx } => assert_eq!(fallback_tx, tx),
            _ => panic!("expected BroadcastFallback"),
        }
    }

    #[test]
    fn save_event_rejected_after_set_pending_fallback() {
        let (persister, _db) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.set_pending_fallback().unwrap();

        let result = persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt());

        assert!(result.is_err(), "save_event must be rejected after fallback is committed");
    }

    #[test]
    fn resume_prioritises_pending_fallback_over_event_log() {
        let (_persister, db) = new_test_persister();
        let tx = test_fallback_tx();
        let session = PayjoinSenderSession {
            events: vec!["irrelevant_event".to_string()],
            fallback_tx: consensus::serialize(&tx).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastFallback),
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastFallback { fallback_tx } => assert_eq!(fallback_tx, tx),
            _ => panic!("expected BroadcastFallback"),
        }
    }

    #[test]
    fn set_pending_fallback_rejects_overwrite_of_proposal() {
        let (persister, _db) = new_test_persister();
        let tx = empty_transaction();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.set_pending_proposal(&tx).unwrap();

        let result = persister.set_pending_fallback();

        assert!(result.is_err(), "BroadcastFallback must not overwrite BroadcastProposal");
    }

    #[test]
    fn set_pending_proposal_rejects_overwrite_with_different_tx() {
        let (persister, _db) = new_test_persister();
        let tx_a = empty_transaction();
        let mut tx_b = empty_transaction();
        tx_b.version = Version::ONE;
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.set_pending_proposal(&tx_a).unwrap();

        let result = persister.set_pending_proposal(&tx_b);

        assert!(result.is_err(), "BroadcastProposal must not be overwritten with a different tx");
    }

    #[test]
    fn set_pending_proposal_is_idempotent() {
        let (persister, db) = new_test_persister();
        let tx = empty_transaction();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.set_pending_proposal(&tx).unwrap();

        // calling again with the same tx must succeed
        persister.set_pending_proposal(&tx).unwrap();

        let session = db.get_payjoin_sender_session().unwrap().unwrap();
        assert!(
            matches!(session.pending_action, Some(PendingAction::BroadcastProposal { .. })),
            "action must still be BroadcastProposal after idempotent call"
        );
    }

    #[test]
    fn resume_with_proposal_marker_returns_broadcast_stored_proposal() {
        let (_persister, db) = new_test_persister();
        let tx = empty_transaction();
        let session = PayjoinSenderSession {
            events: vec![],
            fallback_tx: consensus::serialize(&test_fallback_tx()).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastProposal {
                transaction: consensus::serialize(&tx).into(),
            }),
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastStoredProposal { proposal_tx } => {
                assert_eq!(proposal_tx, tx)
            }
            _ => panic!("expected BroadcastStoredProposal"),
        }
    }

    #[test]
    fn resume_with_corrupt_proposal_retains_record_and_reports_error() {
        let (_persister, db) = new_test_persister();
        let session = PayjoinSenderSession {
            events: vec![],
            fallback_tx: consensus::serialize(&test_fallback_tx()).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastProposal {
                transaction: vec![0xff].into(),
            }),
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db.clone(), WeakAddr::default(), &PayjoinConfig::default());

        assert!(
            matches!(resumption, SessionResumption::ReportError { .. }),
            "corrupt proposal must return ReportError, not None or BroadcastFallback"
        );
        // record must be retained so a human can investigate before retrying
        assert!(
            db.get_payjoin_sender_session().unwrap().is_some(),
            "session record must not be deleted when proposal bytes are corrupt"
        );
    }

    #[test]
    fn resume_with_corrupt_fallback_clears_the_record() {
        let (_persister, db) = new_test_persister();
        let session = PayjoinSenderSession {
            events: vec![],
            fallback_tx: vec![0xff].into(),
            created_at_secs: None,
            pending_action: None,
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db.clone(), WeakAddr::default(), &PayjoinConfig::default());

        assert!(
            matches!(resumption, SessionResumption::ReportError { .. }),
            "corrupt fallback with no marker must surface an error"
        );
        assert_eq!(
            db.get_payjoin_sender_session().unwrap(),
            None,
            "record must be cleared when fallback was never dispatched"
        );
    }

    #[test]
    fn resume_with_corrupt_committed_fallback_retains_the_record() {
        let (_persister, db) = new_test_persister();
        let session = PayjoinSenderSession {
            events: vec![],
            fallback_tx: vec![0xff].into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastFallback), // marker written before crash
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db.clone(), WeakAddr::default(), &PayjoinConfig::default());

        assert!(
            matches!(resumption, SessionResumption::ReportError { .. }),
            "corrupt committed fallback must surface an error"
        );
        assert!(
            db.get_payjoin_sender_session().unwrap().is_some(),
            "record must be retained when broadcast outcome is unknown"
        );
    }
}

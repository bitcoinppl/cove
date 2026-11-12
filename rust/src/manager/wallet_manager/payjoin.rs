#![allow(dead_code)]

use act_zero::*;
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{
    FeeRate as BdkFeeRate, Script, Transaction as BdkTransaction, consensus, params::Params,
};
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
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::database::wallet_data::{
    PayjoinSenderSession, PendingAction, WalletDataDb, WalletDataError,
};

use super::actor::WalletActor;

mod config;
mod status;

pub(crate) use config::{OhttpRelays, PayjoinConfig};
pub(crate) use status::{PayjoinDeadline, unix_now_secs};
pub use status::{PayjoinIntent, PayjoinSessionId, PayjoinStatus, PayjoinTransactionKind};

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

    /// Saves the OHTTP relay the session adopted, so a resumed session keeps using it
    ///
    /// A session adopts one relay for its lifetime, so replacing a saved relay with another is
    /// rejected. Once a terminal action is committed the session sends nothing more, so the
    /// record is left as it is
    pub(crate) fn adopt_relay(&self, relay: &Url) -> Result<(), WalletDataError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WalletDataError::Save("session lock poisoned".to_string()))?;

        let mut session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Save("no payjoin session record".to_string()))?;

        if session.pending_action.is_some() {
            return Ok(());
        }

        match &session.ohttp_relay {
            None => {}
            Some(adopted) if adopted == relay => return Ok(()),
            Some(_) => {
                return Err(WalletDataError::Save(
                    "payjoin session already adopted a different OHTTP relay".to_string(),
                ));
            }
        }

        session.ohttp_relay = Some(relay.clone());
        self.db.set_payjoin_sender_session(session)
    }

    /// Creates a fresh session record; errors if a session record already exists.
    /// The caller must clear any existing record before creating a new one.
    pub(crate) fn create_session(
        &self,
        session_id: PayjoinSessionId,
        fallback_tx: &BdkTransaction,
        created_at_secs: u64,
    ) -> Result<(), WalletDataError> {
        if self.db.get_payjoin_sender_session()?.is_some() {
            return Err(WalletDataError::Save(
                "payjoin session already exists; clear it before creating a new one".to_string(),
            ));
        }

        let session = PayjoinSenderSession {
            session_id,
            events: vec![],
            fallback_tx: consensus::serialize(fallback_tx).into(),
            created_at_secs: Some(created_at_secs),
            pending_action: None,
            ohttp_relay: None,
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

/// The OHTTP relay a session sends its requests through
///
/// The relay sees the sender's IP, so a session that spread its requests over several relays
/// would let any one of them that colludes with the directory link that IP to the payment. A
/// session therefore adopts the first relay whose answer passes `process_response` and sends
/// every later request through that relay alone, even while it fails, until the session ends
#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionRelay {
    /// No request has completed yet, so each attempt tries the configured relays in turn
    Unadopted,
    Adopted(Url),
}

impl SessionRelay {
    /// The relays the next request may go through, in the order to try them
    fn candidates(&self, relays: &OhttpRelays) -> Vec<Url> {
        match self {
            Self::Unadopted => relays.attempt_order(),
            Self::Adopted(relay) => vec![relay.clone()],
        }
    }
}

/// Why one relay did not complete a request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayFailure {
    /// The request could not be sent or the relay answered with an error status
    Unreachable,
    /// The relay answered, but its answer did not pass `process_response`
    InvalidResponse,
}

/// Why one relay's attempt at a request ended without a result
enum AttemptError<E> {
    /// The relay failed, and nothing was persisted, so another relay may still complete it
    Relay(RelayFailure, eyre::Report),
    /// The session itself failed, so no other relay can help
    Session(E),
}

/// Why no relay completed a request
#[derive(Debug)]
enum RelayRoundError<E> {
    /// The receiver's pj expiry has passed, so no relay can carry a request for this session
    SessionExpired,
    /// Every candidate relay failed, the last one as `last_failure`
    AllRelaysFailed { last_failure: RelayFailure, detail: eyre::Report },
    /// The session failed while processing a relay's answer
    Session(E),
}

// tries each candidate relay in order until one carries the request and its answer passes
// `process_response`, returning that relay with the processed result
async fn send_through_relays<C, T, E>(
    client: &reqwest::Client,
    candidates: &[Url],
    timeout: Duration,
    build_request: impl Fn(&str) -> Result<(payjoin::Request, C), CreateRequestError>,
    mut process_response: impl FnMut(&[u8], C) -> Result<T, AttemptError<E>>,
) -> Result<(Url, T), RelayRoundError<E>> {
    // per-relay failures stay at debug because polls repeat them every tick; callers log
    // the summary returned here
    let mut last_failure = RelayFailure::Unreachable;
    let mut last_err = eyre::eyre!("no OHTTP relays to try");
    for relay in candidates {
        let (req, ctx) = match build_request(relay.as_str()) {
            Ok(pair) => pair,

            // expiry does not depend on the relay, so trying the others cannot help
            Err(e) if e.is_expired() => return Err(RelayRoundError::SessionExpired),

            Err(e) => {
                debug!("payjoin: relay {relay} rejected: {e:?}");
                last_failure = RelayFailure::Unreachable;
                last_err = eyre::eyre!("{e:?}");
                continue;
            }
        };

        let body = match http_post(client, req, timeout).await {
            Ok(body) => body,
            Err(e) => {
                debug!("payjoin: relay {relay} request failed: {e:?}");
                last_failure = RelayFailure::Unreachable;
                last_err = e;
                continue;
            }
        };

        match process_response(&body, ctx) {
            Ok(result) => return Ok((relay.clone(), result)),
            Err(AttemptError::Session(e)) => return Err(RelayRoundError::Session(e)),
            Err(AttemptError::Relay(failure, e)) => {
                debug!("payjoin: relay {relay} answer was unusable: {e}");
                last_failure = failure;
                last_err = e;
            }
        }
    }

    Err(RelayRoundError::AllRelaysFailed {
        last_failure,
        detail: eyre::eyre!("{} OHTTP relays failed, last error: {last_err}", candidates.len()),
    })
}

// returns the index of the single external (recipient) output, or Err for batch/self-send PSBTs
//
// ownership comes from the wallet's own scripts, not PSBT output metadata: BDK clears every
// output's bip32_derivation when it finalizes, so a signed PSBT's change looks external
fn recipient_output_index(psbt: &Psbt, is_mine: impl Fn(&Script) -> bool) -> Result<usize> {
    let external: Vec<_> = psbt
        .unsigned_tx
        .output
        .iter()
        .enumerate()
        .filter(|(_, output)| !is_mine(&output.script_pubkey))
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
///
/// `is_mine` reports whether an output script belongs to the sending wallet
pub(crate) fn build_sender(
    signed_psbt: Psbt,
    fallback_tx: &BdkTransaction,
    endpoint: String,
    network: bitcoin::Network,
    persister: &PayjoinSessionPersister,
    is_mine: impl Fn(&Script) -> bool,
) -> Result<V2Sender<WithReplyKey>> {
    // TODO: anti-probing (inputs_seen), verify our inputs have not appeared in a prior session
    // TODO: surface payjoin downgrade to the user when the fallback tx is broadcast instead of the proposal

    let pj_param = v2_pj_param(&endpoint)?;

    let idx = recipient_output_index(&signed_psbt, is_mine)?;
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
    candidates: Vec<Url>,
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
    let round = send_through_relays(
        &client,
        &candidates,
        Duration::from_secs(35),
        |relay| polling_sender.create_poll_request(relay),
        |response, ctx| match polling_sender
            .clone()
            .process_response(response, ctx)
            .save(&persister)
        {
            Ok(outcome) => Ok(outcome),

            // nothing was persisted, so the same polling state can retry through another
            // relay, or through the adopted one on the next poll
            Err(e) if e.is_transient() => Err(AttemptError::Relay(
                RelayFailure::InvalidResponse,
                eyre::eyre!("transient directory response error"),
            )),

            Err(e) => Err(AttemptError::Session(e)),
        },
    )
    .await;

    let error = match round {
        Ok((_, OptionalTransitionOutcome::Progress(proposal_psbt))) => {
            send!(addr.complete_with_success(proposal_psbt));
            return;
        }

        Ok((relay, OptionalTransitionOutcome::Stasis(next))) => {
            debug!("payjoin poll: no proposal yet, continuing");
            send!(addr.update_polling_sender(next, relay));
            return;
        }

        Err(RelayRoundError::SessionExpired) => {
            warn!("payjoin poll: receiver's session expired, broadcasting fallback");
            send!(addr.complete_with_fallback_msg());
            return;
        }

        Err(RelayRoundError::AllRelaysFailed { last_failure, detail }) => {
            let failure = match last_failure {
                RelayFailure::Unreachable => PollFailure::RelaysUnreachable,
                RelayFailure::InvalidResponse => PollFailure::InvalidResponse,
            };

            tokio::time::sleep(Duration::from_secs(2)).await;
            send!(addr.retry_poll_after_failure(failure, detail.to_string()));
            return;
        }

        Err(RelayRoundError::Session(error)) => error,
    };

    // the library has persisted Closed(Aborted), so the session cannot continue
    if error.is_fatal() {
        // receiver-provided data is untrusted: a well-known error's Display can include the
        // receiver's supported_versions list and validation errors can echo bytes of the
        // receiver's PSBT, such as an invalid key, so only the error code or kind is logged
        match error.api_error_ref() {
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
        return;
    }

    let (failure, detail) = match error.storage_error_ref() {
        Some(storage) => (PollFailure::Storage, storage.to_string()),
        None => (PollFailure::Other, "unclassified poll error".to_string()),
    };

    send!(addr.retry_poll_after_failure(failure, detail));
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
    session_id: PayjoinSessionId,
    deadline: PayjoinDeadline,
    fallback_tx: BdkTransaction,
    session: PayjoinSession,
    relay: SessionRelay,

    /// `deadline` on the monotonic clock: a timer falls back when it passes, and no poll
    /// starts after it
    poll_deadline: Instant,
    /// Kept so a failure that repeats on every poll logs once until it changes or clears
    last_poll_failure: Option<PollFailure>,
}

impl PayjoinActor {
    pub(crate) fn new(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        config: PayjoinConfig,
        session_id: PayjoinSessionId,
        deadline: PayjoinDeadline,
        sender: V2Sender<WithReplyKey>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            config,
            session_id,
            deadline,
            fallback_tx,
            session: PayjoinSession::PrePost { sender },
            relay: SessionRelay::Unadopted,
            poll_deadline: deadline.instant(),
            last_poll_failure: None,
        }
    }

    /// Recreates the actor from a replayed session that was already polling for a proposal
    fn resume_polling(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        config: PayjoinConfig,
        session_id: PayjoinSessionId,
        deadline: PayjoinDeadline,
        polling_sender: V2Sender<PollingForProposal>,
        fallback_tx: BdkTransaction,
        relay: SessionRelay,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            config,
            session_id,
            deadline,
            fallback_tx,
            session: PayjoinSession::Polling { polling_sender },
            relay,
            poll_deadline: deadline.instant(),
            last_poll_failure: None,
        }
    }

    pub(crate) fn session_id(&self) -> &PayjoinSessionId {
        &self.session_id
    }

    pub(crate) const fn deadline(&self) -> PayjoinDeadline {
        self.deadline
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

        // a POST or long poll can run well past the deadline, so the deadline the screens
        // count down to ends the wait by itself instead of waiting for that request
        let deadline = tokio::time::Instant::from_std(self.poll_deadline);
        self.addr.send_fut_with(move |addr| async move {
            tokio::time::sleep_until(deadline).await;
            send!(addr.deadline_passed());
        });

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
        let candidates = self.relay.candidates(self.config.relays());

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin POST: failed to create HTTP client: {e:?}");
                    send!(addr.complete_with_fallback_msg());
                    return;
                }
            };

            let round = send_through_relays(
                &client,
                &candidates,
                Duration::from_secs(30),
                |relay| sender.create_v2_post_request(relay),
                |response, ctx| match sender
                    .clone()
                    .process_response(response, ctx)
                    .save(&persister)
                {
                    Ok(polling_sender) => Ok(polling_sender),

                    // nothing was persisted, so the POST can still go through another relay
                    Err(e) if e.is_transient() => Err(AttemptError::Relay(
                        RelayFailure::InvalidResponse,
                        eyre::eyre!("transient directory response error"),
                    )),

                    Err(e) => Err(AttemptError::Session(e)),
                },
            )
            .await;

            let (relay, polling_sender) = match round {
                Ok(pair) => pair,
                Err(RelayRoundError::SessionExpired) => {
                    warn!("payjoin POST: receiver's session already expired");
                    send!(addr.complete_with_fallback_msg());
                    return;
                }
                Err(RelayRoundError::AllRelaysFailed { detail, .. }) => {
                    warn!("payjoin POST: all relays failed: {detail}");
                    send!(addr.complete_with_fallback_msg());
                    return;
                }
                // Display, not Debug: a session error can carry the sender state, whose
                // Debug output includes the whole signed Original PSBT
                Err(RelayRoundError::Session(e)) => {
                    warn!("payjoin POST: failed to process response: {e}");
                    send!(addr.complete_with_fallback_msg());
                    return;
                }
            };

            send!(addr.post_succeeded(polling_sender, relay));
        });

        Produces::ok(())
    }

    /// Stores the polling sender, sets the session deadline, and begins polling
    pub(crate) async fn post_succeeded(
        &mut self,
        polling_sender: V2Sender<PollingForProposal>,
        relay: Url,
    ) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        if !matches!(self.session, PayjoinSession::Posting) {
            warn!("payjoin post_succeeded called in unexpected state");
            return Produces::ok(());
        }

        self.adopt_relay(relay);
        self.session = PayjoinSession::Polling { polling_sender };
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Poll the directory for the receiver's proposal PSBT
    pub(crate) async fn begin_poll(&mut self) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        // the deadline timer may still be queued behind this poll
        if Instant::now() >= self.poll_deadline {
            self.fall_back_at_deadline();
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
        let candidates = self.relay.candidates(self.config.relays());
        self.addr.send_fut_with(|addr| do_poll(addr, polling_sender, persister, candidates));

        Produces::ok(())
    }

    /// Updates the polling sender on Stasis and continues polling
    pub(crate) async fn update_polling_sender(
        &mut self,
        next: V2Sender<PollingForProposal>,
        relay: Url,
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

        self.adopt_relay(relay);
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

    /// Makes `relay`, which just completed a request, the session's relay if it has none yet
    fn adopt_relay(&mut self, relay: Url) {
        if self.relay != SessionRelay::Unadopted {
            return;
        }

        // a failed save leaves the relay adopted in memory, so only a resume after a restart
        // could pick another one
        if let Err(error) = self.persister.adopt_relay(&relay) {
            warn!("payjoin: failed to save the session's OHTTP relay {relay}: {error}");
        }

        info!("payjoin: session adopted OHTTP relay {relay}");
        self.relay = SessionRelay::Adopted(relay);
    }

    fn begin_next_poll(&mut self) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            send!(addr.begin_poll());
        });
    }

    /// Broadcast the original once the session's deadline passes, even with a request in flight
    async fn deadline_passed(&mut self) -> ActorResult<()> {
        if matches!(self.session, PayjoinSession::Closed) {
            return Produces::ok(());
        }

        self.fall_back_at_deadline();
        Produces::ok(())
    }

    fn fall_back_at_deadline(&mut self) {
        let last_failure = self.last_poll_failure;
        warn!(
            "payjoin session deadline passed without a proposal, broadcasting fallback, last poll failure: {last_failure:?}"
        );
        self.complete_with_fallback();
    }

    /// Stop waiting for the receiver and hand the original back for broadcast, exactly as
    /// when the deadline passes
    ///
    /// Sent without waiting for an answer, so the wallet actor never waits on this actor or
    /// its network requests. A request still in flight finds the session closed, and its
    /// late event is rejected once the fallback is committed
    pub(crate) async fn cancel_negotiation(&mut self) -> ActorResult<()> {
        info!("payjoin negotiation cancelled, broadcasting fallback");
        self.complete_with_fallback();
        Produces::ok(())
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

        let session_id = self.session_id.clone();
        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_success(session_id, proposal_psbt, fallback_tx));
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

        let session_id = self.session_id.clone();
        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_fallback(session_id, fallback_tx));
    }

    fn close_session(&mut self) -> bool {
        if matches!(self.session, PayjoinSession::Closed) {
            return false;
        }

        self.session = PayjoinSession::Closed;
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
    BroadcastStoredProposal { session_id: PayjoinSessionId, proposal_tx: BdkTransaction },
    /// Session closed with a success outcome but no stored tx: sign the recovered PSBT and
    /// broadcast.  `fallback_tx` is carried so that if persisting the proposal intent fails,
    /// the actor can fall back to the original transaction.  A signing failure retains the
    /// session without selecting the fallback — the user must retry.
    SignRecoveredProposal {
        session_id: PayjoinSessionId,
        proposal_psbt: Psbt,
        fallback_tx: BdkTransaction,
    },
    /// Session ended without a proposal, broadcast the original tx
    BroadcastFallback { session_id: PayjoinSessionId, fallback_tx: BdkTransaction },
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

    let session_id = record.session_id.clone();
    let fallback_tx = history.fallback_tx();
    match session {
        SendSession::WithReplyKey(_) => {
            // The POST may have already reached the directory before the crash;
            // re-posting is not retry-safe so fall back to the original transaction.
            SessionResumption::BroadcastFallback { session_id, fallback_tx }
        }

        SendSession::PollingForProposal(polling_sender) => {
            let deadline =
                PayjoinDeadline::for_session(record.created_at_secs, config.session_timeout());

            if deadline.has_passed() {
                warn!("payjoin session expired before resume, broadcasting fallback");
                return fallback_from_record(&db, record);
            }

            let relay = match record.ohttp_relay {
                None => SessionRelay::Unadopted,
                Some(relay) if config.relays().contains(&relay) => SessionRelay::Adopted(relay),

                // a relay dropped from the configured list is no longer trusted, so the
                // session picks another rather than keep sending through it
                Some(relay) => {
                    warn!("payjoin: saved OHTTP relay {relay} is no longer configured");
                    SessionRelay::Unadopted
                }
            };

            SessionResumption::Resume(Box::new(PayjoinActor::resume_polling(
                wallet_addr,
                persister,
                config.clone(),
                session_id,
                deadline,
                polling_sender,
                fallback_tx,
                relay,
            )))
        }

        // Cove never cancels through the library, but a replayed Cancelled event still means
        // the original transaction is the one to broadcast
        SendSession::PendingFallback(_) => {
            SessionResumption::BroadcastFallback { session_id, fallback_tx }
        }

        SendSession::Closed(SessionOutcome::Success(proposal_psbt)) => {
            SessionResumption::SignRecoveredProposal { session_id, proposal_psbt, fallback_tx }
        }

        SendSession::Closed(SessionOutcome::Aborted) => {
            SessionResumption::BroadcastFallback { session_id, fallback_tx }
        }
    }
}

/// Recovers the fallback tx from the session record when replay is not possible.
fn fallback_from_record(db: &WalletDataDb, record: PayjoinSenderSession) -> SessionResumption {
    match consensus::deserialize(record.fallback_tx.as_ref()) {
        Ok(fallback_tx) => {
            SessionResumption::BroadcastFallback { session_id: record.session_id, fallback_tx }
        }

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
        Ok(proposal_tx) => SessionResumption::BroadcastStoredProposal {
            session_id: record.session_id,
            proposal_tx,
        },
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
        let deadline = PayjoinDeadline::for_session(Some(unix_now_secs()), Duration::ZERO);

        PayjoinActor {
            addr: WeakAddr::default(),
            wallet_addr: WeakAddr::default(),
            persister,
            config: PayjoinConfig::default(),
            session_id: PayjoinSessionId::generate(),
            deadline,
            fallback_tx,
            session: PayjoinSession::Posting,
            relay: SessionRelay::Unadopted,
            poll_deadline: deadline.instant(),
            last_poll_failure: None,
        }
    }

    /// Whether the stored session has saved the directory's answer to its POST, the point
    /// from which a relaunch resumes polling instead of falling back
    pub(crate) fn session_is_polling(db: &WalletDataDb) -> bool {
        let persister = PayjoinSessionPersister::new(db.clone());
        matches!(replay_event_log(&persister), Ok((SendSession::PollingForProposal(_), _)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::wallet_data::test_support, wallet::metadata::WalletId};
    use bitcoin::{Amount, ScriptBuf, Transaction, TxOut, transaction::Version};

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

    // builds a PSBT whose outputs each pay a distinct script, and returns the wallet's scripts
    fn make_psbt(output_is_mine: &[bool]) -> (Psbt, Vec<ScriptBuf>) {
        let outputs = (0u8..)
            .zip(output_is_mine)
            .map(|(index, is_mine)| (ScriptBuf::from_bytes(vec![index]), *is_mine))
            .collect::<Vec<_>>();

        let unsigned_tx = Transaction {
            version: Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: outputs
                .iter()
                .map(|(script, _)| TxOut {
                    value: Amount::from_sat(50_000),
                    script_pubkey: script.clone(),
                })
                .collect(),
        };

        let owned =
            outputs.into_iter().filter_map(|(script, is_mine)| is_mine.then_some(script)).collect();

        (Psbt::from_unsigned_tx(unsigned_tx).expect("valid tx"), owned)
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
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .expect("payjoin session is persisted");
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
        persister.create_session(PayjoinSessionId::generate(), &fallback, unix_now_secs()).unwrap();
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

        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .expect("recovery data is restored");
        call!(actor.cancel_and_fallback())
            .await
            .expect("the retained Payjoin actor retries after persistence recovers");

        let session = db.get_payjoin_sender_session().unwrap().unwrap();
        assert_eq!(session.pending_action, Some(PendingAction::BroadcastFallback));
    }

    #[test]
    fn test_close_session_is_idempotent() {
        let (persister, _db) = new_test_persister();
        let mut actor = super::test_support::terminal_actor(persister, empty_transaction());

        assert!(actor.close_session());
        assert!(matches!(actor.session, PayjoinSession::Closed));
        assert!(!actor.close_session());
    }

    #[test]
    fn test_recipient_output_index_accepts_single_recipient() {
        // the change output carries no PSBT metadata, as after BDK finalizes a signed PSBT
        let (psbt, owned) = make_psbt(&[true, false]);
        let index = recipient_output_index(&psbt, |script| owned.iter().any(|own| own == script));
        assert_eq!(index.unwrap(), 1);
    }

    #[test]
    fn test_recipient_output_index_rejects_batch_send() {
        let (psbt, owned) = make_psbt(&[false, false, true]);
        let err = recipient_output_index(&psbt, |script| owned.iter().any(|own| own == script))
            .unwrap_err();
        assert!(err.to_string().contains("batch"), "expected 'batch' in error, got: {err}");
    }

    #[test]
    fn test_recipient_output_index_rejects_no_recipient() {
        let (psbt, owned) = make_psbt(&[true]);
        let err = recipient_output_index(&psbt, |script| owned.iter().any(|own| own == script))
            .unwrap_err();
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
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();

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
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        let result = persister.create_session(
            PayjoinSessionId::generate(),
            &test_fallback_tx(),
            unix_now_secs(),
        );

        assert!(result.is_err(), "expected error when session already exists");
        assert_eq!(persister.load().unwrap().count(), 1, "existing session must be unchanged");
    }

    #[test]
    fn close_keeps_the_session_record() {
        let (persister, _db) = new_test_persister();
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
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
        persister.create_session(PayjoinSessionId::generate(), &tx, unix_now_secs()).unwrap();
        // a log that does not start with a Created event cannot be replayed
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastFallback { fallback_tx, .. } => assert_eq!(fallback_tx, tx),
            _ => panic!("expected BroadcastFallback"),
        }
    }

    #[test]
    fn save_event_rejected_after_set_pending_fallback() {
        let (persister, _db) = new_test_persister();
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
        persister.set_pending_fallback().unwrap();

        let result = persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt());

        assert!(result.is_err(), "save_event must be rejected after fallback is committed");
    }

    #[test]
    fn resume_prioritises_pending_fallback_over_event_log() {
        let (_persister, db) = new_test_persister();
        let tx = test_fallback_tx();
        let session = PayjoinSenderSession {
            session_id: PayjoinSessionId::generate(),
            events: vec!["irrelevant_event".to_string()],
            fallback_tx: consensus::serialize(&tx).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastFallback),
            ohttp_relay: None,
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastFallback { fallback_tx, .. } => assert_eq!(fallback_tx, tx),
            _ => panic!("expected BroadcastFallback"),
        }
    }

    #[test]
    fn set_pending_fallback_rejects_overwrite_of_proposal() {
        let (persister, _db) = new_test_persister();
        let tx = empty_transaction();
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
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
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
        persister.set_pending_proposal(&tx_a).unwrap();

        let result = persister.set_pending_proposal(&tx_b);

        assert!(result.is_err(), "BroadcastProposal must not be overwritten with a different tx");
    }

    #[test]
    fn set_pending_proposal_is_idempotent() {
        let (persister, db) = new_test_persister();
        let tx = empty_transaction();
        persister
            .create_session(PayjoinSessionId::generate(), &test_fallback_tx(), unix_now_secs())
            .unwrap();
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
            session_id: PayjoinSessionId::generate(),
            events: vec![],
            fallback_tx: consensus::serialize(&test_fallback_tx()).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastProposal {
                transaction: consensus::serialize(&tx).into(),
            }),
            ohttp_relay: None,
        };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db, WeakAddr::default(), &PayjoinConfig::default());

        match resumption {
            SessionResumption::BroadcastStoredProposal { proposal_tx, .. } => {
                assert_eq!(proposal_tx, tx)
            }
            _ => panic!("expected BroadcastStoredProposal"),
        }
    }

    #[test]
    fn resume_with_corrupt_proposal_retains_record_and_reports_error() {
        let (_persister, db) = new_test_persister();
        let session = PayjoinSenderSession {
            session_id: PayjoinSessionId::generate(),
            events: vec![],
            fallback_tx: consensus::serialize(&test_fallback_tx()).into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastProposal {
                transaction: vec![0xff].into(),
            }),
            ohttp_relay: None,
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
            session_id: PayjoinSessionId::generate(),
            events: vec![],
            fallback_tx: vec![0xff].into(),
            created_at_secs: None,
            pending_action: None,
            ohttp_relay: None,
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
            session_id: PayjoinSessionId::generate(),
            events: vec![],
            fallback_tx: vec![0xff].into(),
            created_at_secs: None,
            pending_action: Some(PendingAction::BroadcastFallback), // marker written before crash
            ohttp_relay: None,
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

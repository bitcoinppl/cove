use act_zero::*;
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{
    Amount, FeeRate as BdkFeeRate, Transaction as BdkTransaction, consensus, params::Params,
};
use cove_util::result_ext::ResultExt as _;
use payjoin::{
    Uri, UriExt,
    persist::{OptionalTransitionOutcome, SessionPersister},
    send::v2::{
        PollingForProposal, SendSession, Sender as V2Sender, SenderBuilder,
        SessionEvent as PayjoinSessionEvent, SessionOutcome, WithReplyKey, replay_event_log,
    },
};
use rand::seq::SliceRandom as _;
use std::time::Duration;
use tracing::{debug, error, warn};

use crate::database::wallet_data::{PayjoinSenderSession, WalletDataDb, WalletDataError};

use super::actor::WalletActor;

// send a payjoin HTTP request via reqwest and return the response bytes
async fn http_post(
    client: &reqwest::Client,
    req: payjoin::Request,
    timeout: Duration,
) -> eyre::Result<Vec<u8>> {
    Ok(client
        .post(&req.url)
        .header("Content-Type", req.content_type)
        .body(req.body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| eyre::eyre!("send failed: {e:?}"))?
        .bytes()
        .await
        .map_err(|e| eyre::eyre!("body read failed: {e:?}"))?
        .to_vec())
}

// returns OHTTP relay URLs shuffled per call for resilience and privacy
fn ohttp_relays() -> Vec<&'static str> {
    let mut relays = vec![
        "https://relay.payjoin.org",
        "https://ohttp.achow101.com",
        "https://pj.bobspacebkk.com",
    ];
    relays.shuffle(&mut rand::rng());
    relays
}

/// Persists payjoin session events to the wallet's database so sessions survive app restarts
#[derive(Debug, Clone)]
pub(super) struct PayjoinSessionPersister {
    db: WalletDataDb,
}

impl PayjoinSessionPersister {
    pub(super) fn new(db: WalletDataDb) -> Self {
        Self { db }
    }

    /// Creates a fresh session record, replacing any previous session for this wallet
    pub(super) fn create_session(
        &self,
        fallback_tx: &BdkTransaction,
    ) -> Result<(), WalletDataError> {
        let session =
            PayjoinSenderSession { events: vec![], fallback_tx: consensus::serialize(fallback_tx) };
        self.db.set_payjoin_sender_session(session)
    }
}

impl SessionPersister for PayjoinSessionPersister {
    type InternalStorageError = WalletDataError;
    type SessionEvent = PayjoinSessionEvent;

    fn save_event(&self, event: Self::SessionEvent) -> Result<(), Self::InternalStorageError> {
        let mut session = self
            .db
            .get_payjoin_sender_session()?
            .ok_or_else(|| WalletDataError::Save("no payjoin session record".to_string()))?;

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

/// Builds the v2 sender state machine from a signed PSBT and payjoin endpoint
pub(super) fn build_sender(
    signed_psbt: Psbt,
    endpoint: String,
    network: bitcoin::Network,
    persister: &PayjoinSessionPersister,
) -> eyre::Result<V2Sender<WithReplyKey>> {
    // TODO: anti-probing (inputs_seen), verify our inputs have not appeared in a prior session

    // '#' in the endpoint is the OHTTP key and must be percent-encoded as '%23' in the pj= param
    let encoded_endpoint = endpoint.replace('#', "%23");

    // reject batch sends: payjoin requires exactly one external (recipient) output
    let external_count =
        signed_psbt.outputs.iter().filter(|o| o.bip32_derivation.is_empty()).count();
    if external_count > 1 {
        return Err(eyre::eyre!(
            "payjoin not supported for batch sends ({external_count} external outputs)"
        ));
    }

    // outputs with no bip32_derivation are external (recipient) outputs
    let (idx, _) = signed_psbt
        .outputs
        .iter()
        .enumerate()
        .find(|(_, o)| o.bip32_derivation.is_empty())
        .ok_or_else(|| eyre::eyre!("no recipient output found in PSBT"))?;

    let txout = &signed_psbt.unsigned_tx.output[idx];
    let amount_btc = Amount::from_sat(txout.value.to_sat()).to_btc();
    let address = bitcoin::Address::from_script(&txout.script_pubkey, Params::from(network))
        .map_err(|e| eyre::eyre!("could not derive address from PSBT output: {e:?}"))?;

    let bip21 = format!("bitcoin:{address}?amount={amount_btc:.8}&pj={encoded_endpoint}");
    let pj_uri = Uri::try_from(bip21.as_str())
        .map_err(|e| eyre::eyre!("failed to parse payjoin URI: {e:?}"))?
        .assume_checked()
        .check_pj_supported()
        .map_err(|_| eyre::eyre!("URI does not support payjoin (missing pj= param)"))?;

    let sender = SenderBuilder::new(signed_psbt, pj_uri)
        .always_disable_output_substitution()
        .build_recommended(BdkFeeRate::BROADCAST_MIN)
        .map_err(|e| eyre::eyre!("failed to build payjoin sender: {e:?}"))?
        .save(persister)
        .map_err(|e| eyre::eyre!("failed to persist payjoin session: {e:?}"))?;

    Ok(sender)
}

/// Tracks which phase of the BIP-77 v2 negotiation the actor is currently in
enum PayjoinSession {
    /// Initial POST has not been sent yet; holds the sender state machine ready to POST
    PrePost { sender: V2Sender<WithReplyKey> },
    /// POST was accepted by the directory; now polling for the receiver's proposal
    Polling { polling_sender: V2Sender<PollingForProposal> },
}

pub(super) struct PayjoinActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    persister: PayjoinSessionPersister,
    fallback_tx: BdkTransaction,
    session: Option<PayjoinSession>,
}

impl PayjoinActor {
    pub(super) fn new(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        sender: V2Sender<WithReplyKey>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            fallback_tx,
            session: Some(PayjoinSession::PrePost { sender }),
        }
    }

    /// Recreates the actor from a replayed session that was already polling for a proposal
    pub(super) fn resume_polling(
        wallet_addr: WeakAddr<WalletActor>,
        persister: PayjoinSessionPersister,
        polling_sender: V2Sender<PollingForProposal>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            persister,
            fallback_tx,
            session: Some(PayjoinSession::Polling { polling_sender }),
        }
    }
}

#[async_trait::async_trait]
impl Actor for PayjoinActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        match &self.session {
            Some(PayjoinSession::PrePost { .. }) => send!(addr.start_post()),
            Some(PayjoinSession::Polling { .. }) => send!(addr.begin_poll()),
            None => {}
        }
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("PayjoinActor error: {error:?}");
        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_fallback(fallback_tx));
        false
    }
}

impl PayjoinActor {
    /// POSTs the signed PSBT to the payjoin directory and transitions to polling
    pub(super) async fn start_post(&mut self) -> ActorResult<()> {
        let sender = match self.session.take() {
            Some(PayjoinSession::PrePost { sender }) => sender,
            unexpected => {
                self.session = unexpected;
                warn!("payjoin start_post called in unexpected state");
                return Produces::ok(());
            }
        };

        let wallet_addr = self.wallet_addr.clone();
        let persister = self.persister.clone();
        let fallback_tx = self.fallback_tx.clone();

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin POST: failed to create HTTP client: {e:?}");
                    send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                    return;
                }
            };

            let post_result = {
                let mut last_err = eyre::eyre!("no OHTTP relays configured");
                let mut success = None;
                for relay in ohttp_relays() {
                    let (req, ctx) = match sender.create_v2_post_request(relay) {
                        Ok(pair) => pair,
                        Err(e) => {
                            warn!("payjoin: relay {relay} rejected for POST: {e:?}");
                            last_err = eyre::eyre!("relay {relay} rejected: {e:?}");
                            continue;
                        }
                    };
                    match http_post(&client, req, Duration::from_secs(30)).await {
                        Ok(body) => {
                            success = Some((body, ctx));
                            break;
                        }
                        Err(e) => {
                            warn!("payjoin: relay {relay} POST failed: {e}");
                            last_err = e;
                        }
                    }
                }
                success.ok_or(last_err)
            };

            let (post_response, post_ctx) = match post_result {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("payjoin POST: all relays failed: {e}");
                    send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                    return;
                }
            };

            let polling_sender =
                match sender.process_response(&post_response, post_ctx).save(&persister) {
                    Ok(ps) => ps,
                    Err(e) => {
                        warn!("payjoin POST: failed to process response: {e:?}");
                        send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                        return;
                    }
                };

            send!(addr.post_succeeded(polling_sender));
        });

        Produces::ok(())
    }

    /// Stores the polling sender and begins polling
    pub(super) async fn post_succeeded(
        &mut self,
        polling_sender: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.session = Some(PayjoinSession::Polling { polling_sender });
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Poll the directory for the receiver's proposal PSBT
    pub(super) async fn begin_poll(&mut self) -> ActorResult<()> {
        // clone so the original stays in self.session, allowing retry if all relays fail this tick
        let polling_sender = match &self.session {
            Some(PayjoinSession::Polling { polling_sender }) => polling_sender.clone(),
            _ => {
                warn!("payjoin begin_poll called in unexpected state");
                return Produces::ok(());
            }
        };

        let wallet_addr = self.wallet_addr.clone();
        let persister = self.persister.clone();
        let fallback_tx = self.fallback_tx.clone();

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin poll: failed to create HTTP client: {e:?}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    send!(addr.begin_next_poll_msg());
                    return;
                }
            };

            let poll_result = {
                let mut last_err = eyre::eyre!("no OHTTP relays configured");
                let mut success = None;
                for relay in ohttp_relays() {
                    let (req, ctx) = match polling_sender.create_poll_request(relay) {
                        Ok(pair) => pair,
                        Err(e) => {
                            warn!("payjoin: relay {relay} rejected for poll: {e:?}");
                            last_err = eyre::eyre!("relay {relay} rejected: {e:?}");
                            continue;
                        }
                    };
                    // polls are long-polling: the directory holds the connection open until a
                    // proposal arrives or its own timeout fires, so allow slightly more than
                    // the server-side timeout to avoid racing it
                    match http_post(&client, req, Duration::from_secs(35)).await {
                        Ok(body) => {
                            success = Some((body, ctx));
                            break;
                        }
                        Err(e) => {
                            warn!("payjoin: relay {relay} poll failed: {e}");
                            last_err = e;
                        }
                    }
                }
                success.ok_or(last_err)
            };

            let (poll_response, poll_ctx) = match poll_result {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("payjoin poll: all relays failed, retrying: {e}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    send!(addr.begin_next_poll_msg());
                    return;
                }
            };

            match polling_sender.process_response(&poll_response, poll_ctx).save(&persister) {
                Ok(OptionalTransitionOutcome::Progress(proposal_psbt)) => {
                    send!(wallet_addr.handle_payjoin_success(proposal_psbt, fallback_tx));
                }
                Ok(OptionalTransitionOutcome::Stasis(next)) => {
                    debug!("payjoin poll: no proposal yet, continuing");
                    send!(addr.update_polling_sender(next));
                }
                Err(e) => {
                    warn!("payjoin poll: fatal error processing response: {e:?}");
                    send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                }
            }
        });

        Produces::ok(())
    }

    /// Updates the polling sender on Stasis and continues polling
    pub(super) async fn update_polling_sender(
        &mut self,
        next: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.session = Some(PayjoinSession::Polling { polling_sender: next });
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Queues the next poll immediately, called from async task context
    pub(super) async fn begin_next_poll_msg(&mut self) -> ActorResult<()> {
        self.begin_next_poll();
        Produces::ok(())
    }

    fn begin_next_poll(&mut self) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            send!(addr.begin_poll());
        });
    }
}

/// What to do with a persisted payjoin session found on startup
pub(super) enum SessionResumption {
    /// No session was persisted
    None,
    /// Session is still in flight, spawn this actor to continue it
    Resume(Box<PayjoinActor>),
    /// Session already completed with a proposal, sign and broadcast it
    BroadcastProposal { proposal_psbt: Psbt, fallback_tx: BdkTransaction },
    /// Session ended without a proposal, broadcast the original tx
    BroadcastFallback { fallback_tx: BdkTransaction },
}

/// Replays a persisted payjoin session from the wallet's database, if one exists
pub(super) fn resume_session(
    db: WalletDataDb,
    wallet_addr: WeakAddr<WalletActor>,
) -> SessionResumption {
    let record = match db.get_payjoin_sender_session() {
        Ok(Some(record)) => record,
        Ok(None) => return SessionResumption::None,
        Err(e) => {
            error!("failed to read payjoin session record: {e}");
            return SessionResumption::None;
        }
    };

    let persister = PayjoinSessionPersister::new(db.clone());
    let (session, history) = match replay_event_log(&persister) {
        Ok(pair) => pair,
        Err(e) => {
            warn!("payjoin session replay failed, broadcasting fallback: {e:?}");
            return fallback_from_record(&db, record);
        }
    };

    let fallback_tx = history.fallback_tx();
    match session {
        SendSession::WithReplyKey(sender) => SessionResumption::Resume(Box::new(
            PayjoinActor::new(wallet_addr, persister, sender, fallback_tx),
        )),
        SendSession::PollingForProposal(polling_sender) => SessionResumption::Resume(Box::new(
            PayjoinActor::resume_polling(wallet_addr, persister, polling_sender, fallback_tx),
        )),
        SendSession::Closed(SessionOutcome::Success(proposal_psbt)) => {
            SessionResumption::BroadcastProposal { proposal_psbt, fallback_tx }
        }
        SendSession::Closed(_) => SessionResumption::BroadcastFallback { fallback_tx },
    }
}

/// Recovers the fallback tx stored in the session record when replay is not possible
fn fallback_from_record(db: &WalletDataDb, record: PayjoinSenderSession) -> SessionResumption {
    match consensus::deserialize(&record.fallback_tx) {
        Ok(fallback_tx) => SessionResumption::BroadcastFallback { fallback_tx },
        Err(e) => {
            error!("stored payjoin fallback tx is corrupt, abandoning session: {e}");
            if let Err(e) = db.delete_payjoin_sender_session() {
                warn!("failed to clear corrupt payjoin session record: {e}");
            }
            SessionResumption::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::wallet_data::test_support, wallet::metadata::WalletId};

    fn new_test_persister() -> (PayjoinSessionPersister, WalletDataDb, tempfile::TempDir) {
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

    #[test]
    fn save_event_requires_a_session_record() {
        let (persister, _db, _tmp) = new_test_persister();

        let result = persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt());

        assert!(result.is_err());
    }

    #[test]
    fn events_round_trip_in_order() {
        let (persister, _db, _tmp) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();

        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();
        persister.save_event(PayjoinSessionEvent::Closed(SessionOutcome::Failure)).unwrap();

        let events: Vec<_> = persister.load().unwrap().collect();
        assert_eq!(
            events,
            vec![
                PayjoinSessionEvent::PostedOriginalPsbt(),
                PayjoinSessionEvent::Closed(SessionOutcome::Failure)
            ]
        );
    }

    #[test]
    fn create_session_replaces_previous_session() {
        let (persister, _db, _tmp) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        persister.create_session(&test_fallback_tx()).unwrap();

        assert_eq!(persister.load().unwrap().count(), 0);
    }

    #[test]
    fn close_keeps_the_session_record() {
        let (persister, _db, _tmp) = new_test_persister();
        persister.create_session(&test_fallback_tx()).unwrap();
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        persister.close().unwrap();

        assert_eq!(persister.load().unwrap().count(), 1);
    }

    #[test]
    fn resume_with_no_record_is_none() {
        let (_persister, db, _tmp) = new_test_persister();

        let resumption = resume_session(db, WeakAddr::default());

        assert!(matches!(resumption, SessionResumption::None));
    }

    #[test]
    fn resume_with_unreplayable_log_broadcasts_stored_fallback() {
        let (persister, db, _tmp) = new_test_persister();
        let tx = test_fallback_tx();
        persister.create_session(&tx).unwrap();
        // a log that does not start with a Created event cannot be replayed
        persister.save_event(PayjoinSessionEvent::PostedOriginalPsbt()).unwrap();

        let resumption = resume_session(db, WeakAddr::default());

        match resumption {
            SessionResumption::BroadcastFallback { fallback_tx } => assert_eq!(fallback_tx, tx),
            _ => panic!("expected BroadcastFallback"),
        }
    }

    #[test]
    fn resume_with_corrupt_fallback_clears_the_record() {
        let (_persister, db, _tmp) = new_test_persister();
        let session = PayjoinSenderSession { events: vec![], fallback_tx: vec![0xff] };
        db.set_payjoin_sender_session(session).unwrap();

        let resumption = resume_session(db.clone(), WeakAddr::default());

        assert!(matches!(resumption, SessionResumption::None));
        assert_eq!(db.get_payjoin_sender_session().unwrap(), None);
    }
}

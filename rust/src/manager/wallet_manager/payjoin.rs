use act_zero::*;
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{Amount, FeeRate as BdkFeeRate, Transaction as BdkTransaction, params::Params};
use payjoin::{
    Uri, UriExt,
    persist::{NoopSessionPersister, OptionalTransitionOutcome},
    send::v2::{
        PollingForProposal, Sender as V2Sender, SenderBuilder, SessionEvent as PayjoinSessionEvent,
        WithReplyKey,
    },
};
use rand::seq::SliceRandom as _;
use std::time::Duration;
use tracing::{debug, error, warn};

use super::actor::WalletActor;

// max poll attempts before falling back to original tx; 60 × 5s = 5 minutes
const PAYJOIN_MAX_POLL_ATTEMPTS: u32 = 60;

const PAYJOIN_POLL_INTERVAL: Duration = Duration::from_secs(5);

// send a payjoin HTTP request via reqwest and return the raw response bytes
async fn http_post(client: &reqwest::Client, req: payjoin::Request) -> eyre::Result<Vec<u8>> {
    Ok(client
        .post(&req.url)
        .header("Content-Type", req.content_type)
        .body(req.body)
        .timeout(Duration::from_secs(30))
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

/// Opaque alias so `actor.rs` can name the initial sender state without importing payjoin internals.
pub(super) type PayjoinSender = V2Sender<WithReplyKey>;

/// Builds the v2 sender state machine from a signed PSBT and payjoin endpoint
pub(super) fn build_sender(
    signed_psbt: Psbt,
    endpoint: String,
    network: bitcoin::Network,
) -> eyre::Result<PayjoinSender> {
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

    let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
    let sender = SenderBuilder::new(signed_psbt, pj_uri)
        .always_disable_output_substitution()
        .build_recommended(BdkFeeRate::BROADCAST_MIN)
        .map_err(|e| eyre::eyre!("failed to build payjoin sender: {e:?}"))?
        .save(&persister)
        .expect("NoopSessionPersister cannot fail");

    Ok(sender)
}

/// Tracks which phase of the BIP-77 v2 negotiation the actor is currently in.
enum PayjoinSession {
    /// Initial POST has not been sent yet; holds the sender state machine ready to POST.
    PrePost { sender: V2Sender<WithReplyKey> },
    /// POST was accepted by the directory; now polling for the receiver's proposal.
    Polling { polling_sender: V2Sender<PollingForProposal> },
}

pub(super) struct PayjoinActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    fallback_tx: BdkTransaction,
    session: Option<PayjoinSession>,
    poll_attempt: u32,
}

impl PayjoinActor {
    pub(super) fn new(
        wallet_addr: WeakAddr<WalletActor>,
        sender: V2Sender<WithReplyKey>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            fallback_tx,
            session: Some(PayjoinSession::PrePost { sender }),
            poll_attempt: 0,
        }
    }
}

#[async_trait::async_trait]
impl Actor for PayjoinActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        send!(addr.start_post());
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
                    match http_post(&client, req).await {
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

            let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
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

    /// Stores the polling sender and schedules the first poll
    pub(super) async fn post_succeeded(
        &mut self,
        polling_sender: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.session = Some(PayjoinSession::Polling { polling_sender });
        self.schedule_next_poll();
        Produces::ok(())
    }

    /// Poll the directory for the receiver's proposal PSBT.
    pub(super) async fn begin_poll(&mut self) -> ActorResult<()> {
        if self.poll_attempt >= PAYJOIN_MAX_POLL_ATTEMPTS {
            warn!("payjoin: timed out after {} poll attempts", PAYJOIN_MAX_POLL_ATTEMPTS);
            let fallback_tx = self.fallback_tx.clone();
            send!(self.wallet_addr.handle_payjoin_fallback(fallback_tx));
            return Produces::ok(());
        }

        // clone so the original stays in self.session, allowing retry if all relays fail this tick
        let polling_sender = match &self.session {
            Some(PayjoinSession::Polling { polling_sender }) => polling_sender.clone(),
            _ => {
                warn!("payjoin begin_poll called in unexpected state");
                return Produces::ok(());
            }
        };

        self.poll_attempt += 1;
        let attempt = self.poll_attempt;
        let wallet_addr = self.wallet_addr.clone();
        let fallback_tx = self.fallback_tx.clone();

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin poll attempt {attempt}: failed to create HTTP client: {e:?}");
                    send!(addr.schedule_next_poll_msg());
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
                    match http_post(&client, req).await {
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
                    warn!("payjoin poll attempt {attempt}: all relays failed, retrying: {e}");
                    send!(addr.schedule_next_poll_msg());
                    return;
                }
            };

            let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
            match polling_sender.process_response(&poll_response, poll_ctx).save(&persister) {
                Ok(OptionalTransitionOutcome::Progress(proposal_psbt)) => {
                    send!(wallet_addr.handle_payjoin_success(proposal_psbt, fallback_tx));
                }
                Ok(OptionalTransitionOutcome::Stasis(next)) => {
                    debug!("payjoin poll attempt {attempt}: no proposal yet, continuing");
                    send!(addr.update_polling_sender(next));
                }
                Err(e) => {
                    warn!("payjoin poll attempt {attempt}: fatal error processing response: {e:?}");
                    send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                }
            }
        });

        Produces::ok(())
    }

    /// Schedules the next poll, called from async task context
    pub(super) async fn schedule_next_poll_msg(&mut self) -> ActorResult<()> {
        self.schedule_next_poll();
        Produces::ok(())
    }

    /// Updates the polling sender on Stasis and schedules the next poll
    pub(super) async fn update_polling_sender(
        &mut self,
        next: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.session = Some(PayjoinSession::Polling { polling_sender: next });
        self.schedule_next_poll();
        Produces::ok(())
    }

    fn schedule_next_poll(&mut self) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            tokio::time::sleep(PAYJOIN_POLL_INTERVAL).await;
            send!(addr.begin_poll());
        });
    }
}

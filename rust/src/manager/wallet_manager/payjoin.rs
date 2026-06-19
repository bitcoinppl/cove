use act_zero::*;
use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{Amount, FeeRate as BdkFeeRate, Transaction as BdkTransaction, params::Params};
use payjoin::{
    Uri, UriExt,
    persist::{NoopSessionPersister, OptionalTransitionOutcome},
    send::{
        ResponseError,
        v2::{
            PollingForProposal, Sender as V2Sender, SenderBuilder,
            SessionEvent as PayjoinSessionEvent, WithReplyKey,
        },
    },
};
use rand::seq::SliceRandom as _;
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

use super::actor::WalletActor;

// maximum time to wait for a receiver proposal before broadcasting the fallback transaction;
// create_poll_request does not check session expiry, so this provides a client-side deadline
const PAYJOIN_SESSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
        .error_for_status()
        .map_err(|e| eyre::eyre!("relay returned error status: {e:?}"))?
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

// tries each OHTTP relay in shuffled order, returning the first successful (body, context) pair
async fn try_ohttp_relays<C>(
    client: &reqwest::Client,
    timeout: Duration,
    build_request: impl Fn(&str) -> eyre::Result<(payjoin::Request, C)>,
) -> eyre::Result<(Vec<u8>, C)> {
    let mut last_err = eyre::eyre!("no OHTTP relays configured");
    for relay in ohttp_relays() {
        let (req, ctx) = match build_request(relay) {
            Ok(pair) => pair,
            Err(e) => {
                warn!("payjoin: relay {relay} rejected: {e:?}");
                last_err = e;
                continue;
            }
        };
        match http_post(client, req, timeout).await {
            Ok(body) => return Ok((body, ctx)),
            Err(e) => {
                warn!("payjoin: relay {relay} request failed: {e:?}");
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// Opaque alias so `actor.rs` can name the initial sender state without importing payjoin internals
pub(crate) type PayjoinSender = V2Sender<WithReplyKey>;

// returns the index of the single external (recipient) output, or Err for batch/self-send PSBTs
fn recipient_output_index(psbt: &Psbt) -> eyre::Result<usize> {
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
    endpoint: String,
    network: bitcoin::Network,
) -> eyre::Result<PayjoinSender> {
    // TODO: anti-probing (inputs_seen), verify our inputs have not appeared in a prior session
    // TODO: surface payjoin downgrade to the user when the fallback tx is broadcast instead of the proposal

    // reject v1 endpoints: SenderBuilder::new (v2) panics with unimplemented! on non-OHTTP URIs
    if !endpoint.contains('#') {
        return Err(eyre::eyre!(
            "payjoin endpoint is not a BIP77 v2 OHTTP endpoint (no '#' fragment); \
             v1 endpoints are not supported — broadcast the fallback transaction instead"
        ));
    }

    // '#' is the URI fragment delimiter (carries OHTTP key, reply key, and expiry) and must be
    // percent-encoded as '%23' in the pj= param so it survives BIP21 URI round-trips
    let encoded_endpoint = endpoint.replace('#', "%23");

    let idx = recipient_output_index(&signed_psbt)?;

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

    // use the original PSBT's effective fee rate as the minimum the receiver must match;
    // BROADCAST_MIN is only used as a fallback if the fee cannot be computed
    let fee_rate = signed_psbt
        .fee()
        .ok()
        .and_then(|fee| {
            let wu = signed_psbt.unsigned_tx.weight().to_wu();
            let sat_per_kwu = fee.to_sat() * 1000;
            sat_per_kwu.checked_div(wu).map(BdkFeeRate::from_sat_per_kwu)
        })
        .unwrap_or(BdkFeeRate::BROADCAST_MIN);

    let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
    let sender = SenderBuilder::new(signed_psbt, pj_uri)
        .always_disable_output_substitution()
        .build_recommended(fee_rate)
        .map_err(|e| eyre::eyre!("failed to build payjoin sender: {e:?}"))?
        .save(&persister)
        .expect("NoopSessionPersister cannot fail");

    Ok(sender)
}

/// Tracks which phase of the BIP-77 v2 negotiation the actor is currently in
enum PayjoinSession {
    /// Initial POST has not been sent yet; holds the sender state machine ready to POST
    PrePost { sender: V2Sender<WithReplyKey> },
    /// POST was accepted by the directory; now polling for the receiver's proposal
    Polling { polling_sender: V2Sender<PollingForProposal> },
}

pub(crate) struct PayjoinActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    fallback_tx: BdkTransaction,
    session: Option<PayjoinSession>,
    poll_deadline: Option<Instant>,
}

impl PayjoinActor {
    pub(crate) fn new(
        wallet_addr: WeakAddr<WalletActor>,
        sender: V2Sender<WithReplyKey>,
        fallback_tx: BdkTransaction,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            fallback_tx,
            session: Some(PayjoinSession::PrePost { sender }),
            poll_deadline: None,
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
    pub(crate) async fn start_post(&mut self) -> ActorResult<()> {
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

            let (post_response, post_ctx) =
                match try_ohttp_relays(&client, Duration::from_secs(30), |relay| {
                    sender.create_v2_post_request(relay).map_err(|e| eyre::eyre!("{e:?}"))
                })
                .await
                {
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

    /// Stores the polling sender, sets the session deadline, and begins polling
    pub(crate) async fn post_succeeded(
        &mut self,
        polling_sender: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.poll_deadline = Some(Instant::now() + PAYJOIN_SESSION_TIMEOUT);
        self.session = Some(PayjoinSession::Polling { polling_sender });
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Poll the directory for the receiver's proposal PSBT
    pub(crate) async fn begin_poll(&mut self) -> ActorResult<()> {
        // if the session deadline has passed, broadcast the fallback immediately
        if self.poll_deadline.is_some_and(|d| Instant::now() >= d) {
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

        let wallet_addr = self.wallet_addr.clone();
        let fallback_tx = self.fallback_tx.clone();

        self.addr.send_fut_with(|addr| async move {
            let client = match cove_http::new_client() {
                Ok(c) => c,
                Err(e) => {
                    warn!("payjoin poll: failed to create HTTP client: {e:?}");
                    send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                    return;
                }
            };

            // polls are long-polling: the directory holds the connection open until a
            // proposal arrives or its own timeout fires, so allow slightly more than
            // the server-side timeout to avoid racing it
            let (poll_response, poll_ctx) =
                match try_ohttp_relays(&client, Duration::from_secs(35), |relay| {
                    polling_sender.create_poll_request(relay).map_err(|e| eyre::eyre!("{e:?}"))
                })
                .await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        warn!("payjoin poll: all relays failed, retrying: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        send!(addr.begin_next_poll_msg());
                        return;
                    }
                };

            let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
            match polling_sender.process_response(&poll_response, poll_ctx).save(&persister) {
                Ok(OptionalTransitionOutcome::Progress(proposal_psbt)) => {
                    send!(wallet_addr.handle_payjoin_success(proposal_psbt, fallback_tx));
                }
                Ok(OptionalTransitionOutcome::Stasis(next)) => {
                    debug!("payjoin poll: no proposal yet, continuing");
                    send!(addr.update_polling_sender(next));
                }
                Err(e) => {
                    // WellKnown/Unrecognized mean the receiver explicitly rejected the session;
                    // the public API does not expose fatal/transient classification directly —
                    // other opaque response errors retry until the bounded session deadline fires
                    match e.api_error_ref() {
                        Some(ResponseError::WellKnown(_))
                        | Some(ResponseError::Unrecognized { .. }) => {
                            warn!("payjoin poll: receiver rejected session: {e:?}");
                            send!(wallet_addr.handle_payjoin_fallback(fallback_tx));
                        }
                        _ => {
                            warn!("payjoin poll: transient error, retrying: {e:?}");
                            send!(addr.begin_next_poll_msg());
                        }
                    }
                }
            }
        });

        Produces::ok(())
    }

    /// Updates the polling sender on Stasis and continues polling
    pub(crate) async fn update_polling_sender(
        &mut self,
        next: V2Sender<PollingForProposal>,
    ) -> ActorResult<()> {
        self.session = Some(PayjoinSession::Polling { polling_sender: next });
        self.begin_next_poll();
        Produces::ok(())
    }

    /// Queues the next poll immediately, called from async task context
    pub(crate) async fn begin_next_poll_msg(&mut self) -> ActorResult<()> {
        self.begin_next_poll();
        Produces::ok(())
    }

    fn begin_next_poll(&mut self) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            send!(addr.begin_poll());
        });
    }

    /// Cancels the session and broadcasts the fallback transaction
    pub(crate) async fn cancel_and_fallback(&mut self) -> ActorResult<()> {
        let fallback_tx = self.fallback_tx.clone();
        send!(self.wallet_addr.handle_payjoin_fallback(fallback_tx));
        Produces::ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        ScriptBuf, Transaction, TxOut, psbt::Output as PsbtOutput, transaction::Version,
    };

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

    #[test]
    fn test_endpoint_fragment_encoded() {
        let endpoint = "https://example.com/pj#ohttp-keys".to_string();
        let encoded = endpoint.replace('#', "%23");
        assert_eq!(encoded, "https://example.com/pj%23ohttp-keys");
        assert!(!encoded.contains('#'), "raw '#' must not appear in encoded endpoint");
    }

    #[test]
    fn test_amount_satoshi_precision() {
        let cases = [
            (1u64, "0.00000001"),
            (100, "0.00000100"),
            (1_000_000, "0.01000000"),
            (100_000_000, "1.00000000"),
            (2_100_000_000_000_000u64, "21000000.00000000"),
        ];
        for (sats, expected) in cases {
            let btc = Amount::from_sat(sats).to_btc();
            let formatted = format!("{btc:.8}");
            assert_eq!(formatted, expected, "sats={sats}");
        }
    }
}

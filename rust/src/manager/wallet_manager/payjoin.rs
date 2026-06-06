use bdk_wallet::chain::bitcoin::Psbt;
use bitcoin::{Amount, FeeRate as BdkFeeRate, params::Params};
use payjoin::{
    Uri, UriExt,
    persist::{NoopSessionPersister, OptionalTransitionOutcome},
    send::v2::{SenderBuilder, SessionEvent as PayjoinSessionEvent},
};
use rand::seq::SliceRandom as _;
use std::time::Duration;
use tracing::{debug, warn};

// max poll attempts before falling back; 60 × 5s = 5 minutes
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

// returns OHTTP relay URLs shuffled per call — random order for resilience and privacy
fn ohttp_relays() -> Vec<&'static str> {
    let mut relays = vec![
        "https://relay.payjoin.org",
        "https://ohttp.achow101.com",
        "https://pj.bobspacebkk.com",
    ];
    relays.shuffle(&mut rand::rng());
    relays
}

// POST the signed PSBT to the payjoin directory via OHTTP relay, then poll until
// the receiver returns a proposal PSBT or we time out after 5 minutes
pub(super) async fn payjoin_http_flow(
    signed_psbt: Psbt,
    endpoint: String,
    network: bitcoin::Network,
) -> eyre::Result<Psbt> {
    // TODO: anti-probing (inputs_seen) — before creating a session, verify our inputs
    // haven't appeared in a prior payjoin session with this receiver

    // the endpoint may contain a '#' fragment (the OHTTP key) which must be
    // percent-encoded as '%23' so it survives as a literal value in the pj= query param
    let encoded_endpoint = endpoint.replace('#', "%23");

    // reject batch sends: payjoin requires exactly one external (recipient) output
    let external_count =
        signed_psbt.outputs.iter().filter(|o| o.bip32_derivation.is_empty()).count();
    if external_count > 1 {
        return Err(eyre::eyre!(
            "payjoin not supported for batch sends ({external_count} external outputs)"
        ));
    }

    // identify the recipient output: outputs with no bip32_derivation are
    // external (receiver) outputs — no derivation info in our wallet
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

    // build the v2 sender state machine; NoopSessionPersister skips persistence for now
    let persister = NoopSessionPersister::<PayjoinSessionEvent>::default();
    let sender = SenderBuilder::new(signed_psbt, pj_uri)
        .build_recommended(BdkFeeRate::BROADCAST_MIN)
        .map_err(|e| eyre::eyre!("failed to build payjoin sender: {e:?}"))?
        .save(&persister)
        .map_err(|e| eyre::eyre!("failed to save sender state: {e:?}"))?;

    // each request needs its own HTTP client — cannot share across the async boundary of send_fut
    let client =
        cove_http::new_client().map_err(|e| eyre::eyre!("failed to create HTTP client: {e:?}"))?;

    // POST the original PSBT to the payjoin directory
    // try relays in random order, falling through to the next on any failure
    let (post_response, post_ctx) = {
        let mut last_err: eyre::Error = eyre::eyre!("no OHTTP relays configured");
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
        success.ok_or(last_err)?
    };

    let mut polling_sender = sender
        .process_response(&post_response, post_ctx)
        .save(&persister)
        .map_err(|e| eyre::eyre!("failed to process POST response: {e:?}"))?;

    // poll for the receiver's payjoin proposal via the OHTTP relay
    // each poll is an HTTP POST (BIP77 mandates this); relays reshuffled per attempt
    for attempt in 0..PAYJOIN_MAX_POLL_ATTEMPTS {
        tokio::time::sleep(PAYJOIN_POLL_INTERVAL).await;

        let poll_result = {
            let mut last_err: eyre::Error = eyre::eyre!("no OHTTP relays configured");
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

        // all relays failed this tick; skip and retry on the next tick
        let (poll_response, poll_ctx) = match poll_result {
            Ok(pair) => pair,
            Err(e) => {
                warn!("payjoin poll attempt {attempt}: all relays failed, retrying: {e}");
                continue;
            }
        };

        match polling_sender
            .process_response(&poll_response, poll_ctx)
            .save(&persister)
            .map_err(|e| eyre::eyre!("failed to process poll response: {e:?}"))?
        {
            OptionalTransitionOutcome::Progress(proposal_psbt) => return Ok(proposal_psbt),
            OptionalTransitionOutcome::Stasis(next) => {
                polling_sender = next;
                debug!("payjoin poll attempt {attempt}: no proposal yet, continuing");
            }
        }
    }

    Err(eyre::eyre!("payjoin timed out after {PAYJOIN_MAX_POLL_ATTEMPTS} poll attempts"))
}

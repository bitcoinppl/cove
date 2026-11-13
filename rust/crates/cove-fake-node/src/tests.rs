use bdk_electrum::electrum_client::{self, Client as ElectrumClient, ElectrumApi as _, Param};
use bdk_esplora::esplora_client;
use bitcoin::{
    Address, Amount, CompressedPublicKey, EcdsaSighashType, Network, OutPoint, ScriptBuf, Sequence,
    TapSighashType, Transaction, TxIn, TxOut, Txid, Witness,
    absolute::LockTime,
    ecdsa,
    hashes::{Hash as _, sha256},
    key::{Keypair, TapTweak as _},
    opcodes::all::{OP_CHECKMULTISIG, OP_PUSHNUM_1},
    secp256k1::{Message, Secp256k1, SecretKey},
    sighash::{Prevouts, SighashCache},
    taproot,
    transaction::Version,
};
use serde_json::Value;

use crate::{BroadcastError, FakeNode, Protocol, ReplacementError, TxLocation};

fn test_script(tag: u8) -> ScriptBuf {
    ScriptBuf::from_bytes(vec![
        0x00, 0x14, tag, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    ])
}

/// A transaction spending output 0 of `txid`, which pays to `script`
fn spend(txid: Txid, script: ScriptBuf, amount: Amount) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint { txid, vout: 0 },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut { value: amount, script_pubkey: script }],
    }
}

#[tokio::test]
async fn request_log_records_protocol_and_scripthash() {
    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(1);
    node.chain().pay(&script, Amount::from_sat(1_000));

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    esplora.scripthash_txs(&script, None).await.unwrap();

    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();
    electrum.script_get_history(&script).unwrap();

    let requests = node.history_requests_for(&script);
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests.iter().any(|request| request.protocol == Protocol::Esplora));
    assert!(requests.iter().any(|request| request.protocol == Protocol::Electrum));
    assert!(node.history_requests_for(&test_script(2)).is_empty());

    node.clear_requests();
    assert!(node.requests().is_empty());
}

#[tokio::test]
async fn both_protocols_agree_on_history_and_confirmation() {
    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(3);
    let txid = node.chain().pay(&script, Amount::from_sat(5_000));

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    // pending: in the mempool on both protocols
    let pending = esplora.scripthash_txs(&script, None).await.unwrap();
    assert_eq!((pending.len(), pending[0].txid, pending[0].status.confirmed), (1, txid, false));
    assert_eq!(pending[0].vout[0].value, 5_000);
    assert_eq!(pending[0].fee, 1_000);

    let pending = electrum.script_get_history(&script).unwrap();
    assert_eq!((pending.len(), pending[0].tx_hash, pending[0].height), (1, txid, 0));

    // confirmed: both protocols move to the same block, with a valid merkle proof
    node.chain().mine(2);
    let height = node.chain().confirm(txid);
    assert_eq!(node.chain().location(txid), Some(TxLocation::Confirmed { height }));

    let status = esplora.get_tx_status(&txid).await.unwrap();
    assert_eq!((status.confirmed, status.block_height), (true, Some(height)));
    assert_eq!(
        esplora.get_header_by_hash(&status.block_hash.unwrap()).await.unwrap(),
        node.chain().header(height).unwrap()
    );

    let history = electrum.script_get_history(&script).unwrap();
    assert_eq!(history[0].height, height as i32);

    let header = electrum.block_header(height as usize).unwrap();
    let proof = electrum.transaction_get_merkle(&txid, height as usize).unwrap();
    assert!(electrum_client::utils::validate_merkle_proof(&txid, &header.merkle_root, &proof));

    let esplora_proof = esplora.get_merkle_proof(&txid).await.unwrap().unwrap();
    assert_eq!((esplora_proof.block_height, esplora_proof.pos), (height, proof.pos));

    // evicting a pending transaction removes it and its pending child from both
    let evicted = node.chain().pay(&test_script(4), Amount::from_sat(2_000));
    let child_script = test_script(13);
    let child =
        node.chain().add_to_mempool(spend(evicted, child_script.clone(), Amount::from_sat(1_000)));
    node.chain().evict(evicted);
    assert!(esplora.get_tx_status(&evicted).await.is_err());
    assert!(electrum.transaction_get(&evicted).is_err());

    assert_eq!(node.chain().location(child), None);
    assert!(electrum.script_get_history(&child_script).unwrap().is_empty());
    assert!(esplora.scripthash_txs(&child_script, None).await.unwrap().is_empty());
}

/// Selects a network for one test and restores the previous selection on drop
///
/// `selected_node` follows the selected network, which other tests change
#[tokio::test]
async fn balances_subtract_spent_outputs_on_both_protocols() {
    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(5);
    let address = Address::from_script(&script, Network::Bitcoin).unwrap();
    let funding = node.chain().pay(&script, Amount::from_sat(5_000));
    node.chain().confirm(funding);

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    // pending: the spend takes the confirmed funds out of the unconfirmed balance
    let spending =
        node.chain().add_to_mempool(spend(funding, test_script(6), Amount::from_sat(4_000)));

    let balance = electrum.script_get_balance(&script).unwrap();
    assert_eq!((balance.confirmed, balance.unconfirmed), (5_000, -5_000));

    let stats = esplora.get_address_stats(&address).await.unwrap();
    assert_eq!((stats.chain_stats.funded_txo_sum, stats.chain_stats.spent_txo_sum), (5_000, 0));
    assert_eq!(
        (stats.mempool_stats.spent_txo_count, stats.mempool_stats.spent_txo_sum),
        (1, 5_000)
    );

    // confirmed: nothing is left on either protocol
    node.chain().confirm(spending);

    let balance = electrum.script_get_balance(&script).unwrap();
    assert_eq!((balance.confirmed, balance.unconfirmed), (0, 0));

    let stats = esplora.get_address_stats(&address).await.unwrap();
    assert_eq!((stats.chain_stats.spent_txo_count, stats.chain_stats.spent_txo_sum), (1, 5_000));
    assert_eq!(stats.mempool_stats.spent_txo_count, 0);
}

#[tokio::test]
async fn rebroadcasts_are_accepted_until_the_transaction_confirms() {
    let node = FakeNode::start(Network::Bitcoin);
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let funding = node.chain().pay(&p2wpkh_script(&key), Amount::from_sat(5_000));
    let tx = p2wpkh_spend(&node.chain().tx(funding).unwrap().tx, &key, Amount::from_sat(4_000));
    let txid = tx.compute_txid();

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    // a retry of a pending broadcast succeeds on both protocols
    for _ in 0..2 {
        esplora.broadcast(&tx).await.unwrap();
        assert_eq!(electrum.transaction_broadcast(&tx).unwrap(), txid);
    }

    // the broadcast log keeps the first acceptance only, so retries do not look like new sends
    let logged =
        node.chain().broadcasts().iter().map(Transaction::compute_txid).collect::<Vec<_>>();
    assert_eq!(logged, [txid]);

    // a confirmed transaction is rejected with a reply rather than a dropped connection
    node.chain().confirm_together(&[funding, txid]);

    let esplora_error = esplora.broadcast(&tx).await.unwrap_err().to_string();
    assert!(esplora_error.contains("already in block chain"), "{esplora_error}");

    let electrum_error = electrum.transaction_broadcast(&tx).unwrap_err().to_string();
    assert!(electrum_error.contains("already in block chain"), "{electrum_error}");
}

#[tokio::test]
async fn script_hashes_only_match_in_the_byte_order_each_protocol_uses() {
    let node = FakeNode::start(Network::Bitcoin);
    let script = test_script(10);
    node.chain().pay(&script, Amount::from_sat(1_000));

    let digest = sha256::Hash::hash(script.as_bytes()).to_byte_array();
    let forward = hex::encode(digest);
    let reversed = hex::encode(digest.iter().rev().copied().collect::<Vec<_>>());

    // Electrum takes the digest reversed
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();
    let electrum_history = |hash: &str| {
        let params = [Param::String(hash.to_string())];
        let history = electrum.raw_call("blockchain.scripthash.get_history", params).unwrap();

        history.as_array().expect("history is an array").len()
    };
    assert_eq!(electrum_history(&reversed), 1);
    assert_eq!(electrum_history(&forward), 0);

    // Esplora takes the digest as hashed
    let _ = rustls::crypto::ring::default_provider().install_default();
    let esplora_history = async |hash: &str| {
        let url = format!("{}/scripthash/{hash}/txs", node.esplora_url());
        let history = reqwest::get(url).await.unwrap().json::<Value>().await.unwrap();

        history.as_array().expect("history is an array").len()
    };
    assert_eq!(esplora_history(&forward).await, 1);
    assert_eq!(esplora_history(&reversed).await, 0);
}

#[tokio::test]
async fn pending_children_of_pending_parents_report_height_minus_one() {
    let node = FakeNode::start(Network::Bitcoin);
    let (parent_script, child_script) = (test_script(11), test_script(12));
    let parent = node.chain().pay(&parent_script, Amount::from_sat(5_000));
    let child =
        node.chain().add_to_mempool(spend(parent, child_script.clone(), Amount::from_sat(4_000)));

    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();
    let heights = |script: &ScriptBuf| {
        let history = electrum.script_get_history(script).unwrap();

        history.into_iter().map(|entry| (entry.tx_hash, entry.height)).collect::<Vec<_>>()
    };

    assert_eq!(heights(&parent_script), [(parent, 0), (child, -1)]);
    assert_eq!(heights(&child_script), [(child, -1)]);

    // once the parent confirms, the child no longer depends on a pending transaction
    let height = node.chain().confirm(parent) as i32;
    assert_eq!(heights(&parent_script), [(parent, height), (child, 0)]);
    assert_eq!(heights(&child_script), [(child, 0)]);
}

fn compressed_public(key: &SecretKey) -> CompressedPublicKey {
    let private = bitcoin::PrivateKey::new(*key, Network::Bitcoin);

    CompressedPublicKey::from_private_key(&Secp256k1::new(), &private)
        .expect("the key is compressed")
}

fn p2wpkh_script(key: &SecretKey) -> ScriptBuf {
    ScriptBuf::new_p2wpkh(&compressed_public(key).wpubkey_hash())
}

/// A spend of `funding`'s output 0, which pays to [`p2wpkh_script`] of `key`,
/// signed with `key`
fn p2wpkh_spend(funding: &Transaction, key: &SecretKey, amount: Amount) -> Transaction {
    p2wpkh_spend_to(funding, key, test_script(20), amount)
}

/// Like [`p2wpkh_spend`], paying to `script`
fn p2wpkh_spend_to(
    funding: &Transaction,
    key: &SecretKey,
    script: ScriptBuf,
    amount: Amount,
) -> Transaction {
    p2wpkh_sign(spend(funding.compute_txid(), script, amount), funding, key)
}

/// Signs input 0 of `tx`, which spends `funding`'s output 0, with `key`
fn p2wpkh_sign(mut tx: Transaction, funding: &Transaction, key: &SecretKey) -> Transaction {
    let prevout = &funding.output[0];
    let sighash = SighashCache::new(&tx)
        .p2wpkh_signature_hash(0, &prevout.script_pubkey, prevout.value, EcdsaSighashType::All)
        .expect("output 0 is P2WPKH");
    let message = Message::from_digest(sighash.to_byte_array());
    let signature = ecdsa::Signature::sighash_all(Secp256k1::new().sign_ecdsa(&message, key));
    tx.input[0].witness = Witness::p2wpkh(&signature, &compressed_public(key).0);

    tx
}

#[tokio::test]
async fn broadcasts_must_spend_known_outputs_with_valid_signatures() {
    let node = FakeNode::start(Network::Bitcoin);
    let chain = node.chain();
    let secp = Secp256k1::new();
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let other_key = SecretKey::from_slice(&[8; 32]).unwrap();

    let funding = chain.tx(chain.pay(&p2wpkh_script(&key), Amount::from_sat(10_000))).unwrap().tx;

    // an unsigned spend is refused on both protocols with bitcoind's reject reason
    let unsigned = spend(funding.compute_txid(), test_script(20), Amount::from_sat(9_000));
    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    let esplora_error = esplora.broadcast(&unsigned).await.unwrap_err().to_string();
    assert!(esplora_error.contains("mandatory-script-verify-flag-failed"), "{esplora_error}");

    let electrum_error = electrum.transaction_broadcast(&unsigned).unwrap_err().to_string();
    assert!(electrum_error.contains("mandatory-script-verify-flag-failed"), "{electrum_error}");

    // a signature from the wrong key is refused, the right one is accepted
    let wrong = p2wpkh_spend(&funding, &other_key, Amount::from_sat(9_000));
    assert!(matches!(chain.broadcast(wrong), Err(BroadcastError::InvalidScript { index: 0, .. })));

    let signed = p2wpkh_spend(&funding, &key, Amount::from_sat(9_000));
    esplora.broadcast(&signed).await.expect("a signed spend is accepted");
    assert!(chain.broadcasts().iter().any(|tx| tx.compute_txid() == signed.compute_txid()));

    // taproot key spends are checked too, which libbitcoinconsensus before Core 26 skipped
    let keypair = Keypair::from_secret_key(&secp, &key);
    let (internal, _) = keypair.x_only_public_key();
    let p2tr = ScriptBuf::new_p2tr(&secp, internal, None);
    let taproot_funding = chain.tx(chain.pay(&p2tr, Amount::from_sat(10_000))).unwrap().tx;
    let prevouts = [taproot_funding.output[0].clone()];
    let mut taproot_spend =
        spend(taproot_funding.compute_txid(), test_script(21), Amount::from_sat(9_000));

    let sighash = SighashCache::new(&taproot_spend)
        .taproot_key_spend_signature_hash(0, &Prevouts::All(&prevouts), TapSighashType::Default)
        .unwrap();
    let message = Message::from_digest(sighash.to_byte_array());
    let tweaked = keypair.tap_tweak(&secp, None).to_keypair();

    // signing with the untweaked key gives a well-formed but invalid signature
    let untweaked = secp.sign_schnorr_no_aux_rand(&message, &keypair);
    taproot_spend.input[0].witness =
        Witness::p2tr_key_spend(&taproot::Signature::from_slice(untweaked.as_ref()).unwrap());
    assert!(matches!(
        chain.broadcast(taproot_spend.clone()),
        Err(BroadcastError::InvalidScript { index: 0, .. })
    ));

    let valid = secp.sign_schnorr_no_aux_rand(&message, &tweaked);
    taproot_spend.input[0].witness =
        Witness::p2tr_key_spend(&taproot::Signature::from_slice(valid.as_ref()).unwrap());
    assert_eq!(chain.broadcast(taproot_spend.clone()), Ok(taproot_spend.compute_txid()));

    // an input the chain has never seen is refused before any script runs
    let unknown = OutPoint { txid: Txid::from_byte_array([9; 32]), vout: 0 };
    let orphan = spend(unknown.txid, test_script(22), Amount::from_sat(1_000));
    assert_eq!(chain.broadcast(orphan.clone()), Err(BroadcastError::MissingInput(unknown)));

    let electrum_error = electrum.transaction_broadcast(&orphan).unwrap_err().to_string();
    assert!(electrum_error.contains("bad-txns-inputs-missingorspent"), "{electrum_error}");
}

#[tokio::test]
async fn a_replacement_evicts_the_original_and_its_descendants() {
    let node = FakeNode::start(Network::Bitcoin);
    let chain = node.chain();
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let funding = chain.tx(chain.pay(&p2wpkh_script(&key), Amount::from_sat(10_000))).unwrap().tx;

    // the original pays 1,000 sats without signaling RBF, and a child of it pays 500 more;
    // the funding confirms first, so the original can't inherit its signal
    chain.confirm(funding.compute_txid());
    let mut original = spend(funding.compute_txid(), test_script(19), Amount::from_sat(9_000));
    original.input[0].sequence = Sequence::MAX;
    let original_txid = chain.broadcast(p2wpkh_sign(original, &funding, &key)).unwrap();
    let child =
        chain.add_to_mempool(spend(original_txid, test_script(21), Amount::from_sat(8_500)));

    // 3,000 sats covers both and the incremental relay fee, and full RBF ignores the signal
    let replacement = p2wpkh_spend_to(&funding, &key, test_script(20), Amount::from_sat(7_000));

    let esplora = esplora_client::Builder::new(&node.esplora_url()).build_async().unwrap();
    esplora.broadcast(&replacement).await.expect("the replacement pays enough");

    assert_eq!(chain.location(original_txid), None);
    assert_eq!(chain.location(child), None);
    assert_eq!(chain.location(replacement.compute_txid()), Some(TxLocation::Mempool));
    assert_eq!(
        chain.broadcasts().iter().map(Transaction::compute_txid).collect::<Vec<_>>(),
        [replacement.compute_txid()]
    );

    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();
    let history = electrum.script_get_history(&p2wpkh_script(&key)).unwrap();
    let txids = history.iter().map(|entry| entry.tx_hash).collect::<Vec<_>>();
    assert_eq!(txids, [funding.compute_txid(), replacement.compute_txid()]);
}

#[tokio::test]
async fn replacements_that_underpay_are_refused_with_core_reasons() {
    let node = FakeNode::start(Network::Bitcoin);
    let chain = node.chain();
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let funding = chain.tx(chain.pay(&p2wpkh_script(&key), Amount::from_sat(10_000))).unwrap().tx;

    let original = p2wpkh_spend(&funding, &key, Amount::from_sat(9_000));
    let original_txid = chain.broadcast(original).unwrap();
    let electrum = ElectrumClient::new(&node.electrum_url()).unwrap();

    // a lower fee rate than the transaction it double-spends
    let cheaper = p2wpkh_spend_to(&funding, &key, test_script(22), Amount::from_sat(9_500));
    assert_eq!(
        chain.broadcast(cheaper),
        Err(BroadcastError::Replacement(ReplacementError::FeeRateNotHigher {
            conflict: original_txid
        }))
    );

    // twice the fee is a higher rate per byte, but a bare multisig output counts 20 signature
    // operations, which Core sizes at 400 vB
    let bare_multisig = ScriptBuf::builder()
        .push_opcode(OP_PUSHNUM_1)
        .push_slice(compressed_public(&key).to_bytes())
        .push_opcode(OP_PUSHNUM_1)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script();

    let sigop_heavy = p2wpkh_spend_to(&funding, &key, bare_multisig, Amount::from_sat(8_000));
    assert_eq!(
        chain.broadcast(sigop_heavy),
        Err(BroadcastError::Replacement(ReplacementError::FeeRateNotHigher {
            conflict: original_txid
        }))
    );

    // 50 more sats is a higher rate but does not pay to relay a ~110 vB replacement
    let barely_more = p2wpkh_spend_to(&funding, &key, test_script(22), Amount::from_sat(8_950));
    let required = Amount::from_sat(barely_more.vsize() as u64);
    assert_eq!(
        chain.broadcast(barely_more.clone()),
        Err(BroadcastError::Replacement(ReplacementError::NotEnoughToRelay {
            additional_fee: Amount::from_sat(50),
            required,
        }))
    );

    let error = electrum.transaction_broadcast(&barely_more).unwrap_err().to_string();
    assert!(error.contains("insufficient fee, not enough additional fees to relay"), "{error}");

    // a child's fee counts too: 1,500 beats the original's rate but not 1,000 + 1,000
    chain.add_to_mempool(spend(original_txid, test_script(21), Amount::from_sat(8_000)));
    let below_package = p2wpkh_spend_to(&funding, &key, test_script(22), Amount::from_sat(8_500));
    assert_eq!(
        chain.broadcast(below_package),
        Err(BroadcastError::Replacement(ReplacementError::LessFeeThanReplaced {
            fee: Amount::from_sat(1_500),
            replaced_fees: Amount::from_sat(2_000),
        }))
    );

    // spending more than its inputs is refused before any fee comparison
    let overspends = p2wpkh_spend_to(&funding, &key, test_script(22), Amount::from_sat(20_000));
    assert_eq!(chain.broadcast(overspends), Err(BroadcastError::OutputsExceedInputs));

    // nor may it spend an output of a transaction it would evict
    let mut spends_original = p2wpkh_spend(&funding, &key, Amount::from_sat(5_000));
    spends_original
        .input
        .push(spend(original_txid, test_script(23), Amount::ZERO).input[0].clone());
    assert_eq!(
        chain.broadcast(spends_original),
        Err(BroadcastError::Replacement(ReplacementError::SpendsConflictingTx(original_txid)))
    );

    // a pending output the original never spent from can't be added
    let other_funding = chain.pay(&p2wpkh_script(&key), Amount::from_sat(10_000));
    let mut adds_pending = p2wpkh_spend(&funding, &key, Amount::from_sat(5_000));
    adds_pending.input.push(spend(other_funding, test_script(23), Amount::ZERO).input[0].clone());
    let added = OutPoint { txid: other_funding, vout: 0 };
    assert_eq!(
        chain.broadcast(adds_pending),
        Err(BroadcastError::Replacement(ReplacementError::AddsUnconfirmedInput(added)))
    );

    // nothing above touched the mempool, and once the original confirms its input is spent
    assert_eq!(chain.location(original_txid), Some(TxLocation::Mempool));
    chain.confirm(original_txid);

    let late = p2wpkh_spend_to(&funding, &key, test_script(22), Amount::from_sat(5_000));
    let spent = OutPoint { txid: funding.compute_txid(), vout: 0 };
    assert_eq!(chain.broadcast(late.clone()), Err(BroadcastError::SpentInput(spent)));

    let error = electrum.transaction_broadcast(&late).unwrap_err().to_string();
    assert!(error.contains("bad-txns-inputs-missingorspent"), "{error}");
}

#[tokio::test]
async fn a_replacement_may_evict_at_most_100_transactions() {
    let node = FakeNode::start(Network::Bitcoin);
    let chain = node.chain();

    // 101 pending payments, each spending a coin of its own
    let coins = (0..=100)
        .map(|_| chain.pay(&test_script(30), Amount::from_sat(10_000)))
        .collect::<Vec<_>>();

    for coin in &coins {
        chain.add_to_mempool(spend(*coin, test_script(31), Amount::from_sat(9_000)));
    }

    // one transaction double-spending them all, paying far more than they do together
    let replacing = |coins: &[Txid]| {
        let mut replacement = spend(coins[0], test_script(32), Amount::from_sat(10_000));
        replacement.input = coins
            .iter()
            .map(|coin| spend(*coin, ScriptBuf::new(), Amount::ZERO).input[0].clone())
            .collect();

        replacement
    };

    assert_eq!(
        chain.broadcast(replacing(&coins)),
        Err(BroadcastError::Replacement(ReplacementError::TooManyReplacements { count: 101 }))
    );

    // 100 is allowed, and the unsigned replacement only fails its script checks
    let error = chain.broadcast(replacing(&coins[..100])).unwrap_err();
    assert!(matches!(error, BroadcastError::InvalidScript { index: 0, .. }), "{error:?}");
}

#[tokio::test]
async fn a_replacement_may_spend_from_a_pending_parent_of_a_child_it_evicts() {
    let node = FakeNode::start(Network::Bitcoin);
    let chain = node.chain();
    let key = SecretKey::from_slice(&[7; 32]).unwrap();
    let funding = chain.tx(chain.pay(&p2wpkh_script(&key), Amount::from_sat(10_000))).unwrap().tx;

    // a pending parent with two outputs anyone can spend, so only input 0 needs a signature
    let anyone = ScriptBuf::from(vec![OP_PUSHNUM_1.to_u8()]);
    let parent_funding = chain.pay(&test_script(40), Amount::from_sat(10_000));
    let mut parent = spend(parent_funding, anyone.clone(), Amount::from_sat(4_000));
    parent.output.push(TxOut { value: Amount::from_sat(4_000), script_pubkey: anyone });
    let parent = chain.add_to_mempool(parent);

    // the original pays 1,000 sats, and its child spends the parent's first output
    let original_txid =
        chain.broadcast(p2wpkh_spend(&funding, &key, Amount::from_sat(9_000))).unwrap();
    let mut child = spend(original_txid, test_script(41), Amount::from_sat(12_000));
    child.input.push(spend(parent, ScriptBuf::new(), Amount::ZERO).input[0].clone());
    let child = chain.add_to_mempool(child);

    // the replacement spends the parent's other output: the parent is pending, but an evicted
    // transaction already spent from it, so rule 2 allows it
    let mut replacement = spend(funding.compute_txid(), test_script(42), Amount::from_sat(11_500));
    let mut other_output = spend(parent, ScriptBuf::new(), Amount::ZERO).input[0].clone();
    other_output.previous_output.vout = 1;
    replacement.input.push(other_output);
    let replacement = p2wpkh_sign(replacement, &funding, &key);

    assert_eq!(chain.broadcast(replacement.clone()), Ok(replacement.compute_txid()));
    assert_eq!(chain.location(original_txid), None);
    assert_eq!(chain.location(child), None);
    assert_eq!(chain.location(parent), Some(TxLocation::Mempool));
}

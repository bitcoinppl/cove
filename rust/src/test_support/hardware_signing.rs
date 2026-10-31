//! Fixtures for the watch-only hardware wallet signing round trip
//!
//! The signer side is built from BDK and rust-bitcoin directly, not from Cove's key or
//! descriptor code, so a round trip through it checks Cove against an independent
//! implementation instead of against itself

use std::str::FromStr as _;
use std::sync::Arc;

use bdk_wallet::{
    SignOptions,
    bitcoin::{
        self, Amount as BdkAmount, BlockHash, CompressedPublicKey, OutPoint, Psbt as BdkPsbt,
        ScriptBuf, Transaction, TxOut,
        bip32::{DerivationPath, Xpriv, Xpub},
        hashes::Hash as _,
        secp256k1::{Message, Secp256k1, SecretKey},
        sighash::{EcdsaSighashType, SighashCache},
    },
    chain::BlockId,
    miniscript::{Descriptor, DescriptorPublicKey},
    test_utils::{insert_checkpoint, receive_output_in_latest_block},
};
use bip39::Mnemonic;

use crate::{
    database::{Database, test_support::RestoreWalletTarget},
    manager::wallet_manager::RustWalletManager,
    network::Network,
    transaction::{Amount, FeeRate},
    wallet::{
        Address, Wallet,
        metadata::{WalletId, WalletMode},
    },
};
use cove_types::confirm::ConfirmDetails;

/// Public BIP39 test vector; it must never hold real funds
pub(crate) const SIGNER_MNEMONIC: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

/// A second public BIP39 test vector for a signer Cove has never seen
pub(crate) const UNRELATED_SIGNER_MNEMONIC: &str =
    "letter advice cage absurd amount doctor acoustic avoid letter advice cage above";

/// Amounts of the confirmed outputs that fund the watch-only wallet
pub(crate) const FUNDING_SATS: [u64; 2] = [60_000, 50_000];

/// More than either funding output, so the draft spends both and the signer signs two inputs
pub(crate) const SEND_SATS: u64 = 80_000;

pub(crate) const FEE_RATE_SAT_PER_VB: f32 = 2.0;

/// Resets the process-global database and selects Signet main mode for one test
///
/// Hold the returned guard for the whole test, after the global state lock, so the previous
/// network and mode come back before the lock is released
pub(crate) fn init_signet_globals() -> RestoreWalletTarget {
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    crate::app::reconcile::test_support::init_noop_updater();
    crate::database::test_support::delete_database();

    let restore = RestoreWalletTarget::capture();
    let config = &Database::global().global_config;
    config.set_selected_network(Network::Signet).expect("signet selects");
    config.set_main_mode().expect("main mode selects");

    // the global database stays open across tests, so earlier tests' wallets would otherwise
    // turn this test's import into a duplicate
    for network in [Network::Signet, Network::Bitcoin] {
        for mode in [WalletMode::Main, WalletMode::Decoy] {
            Database::global()
                .wallets
                .save_all_wallets(network, mode, Vec::new())
                .expect("cleared");
        }
    }

    restore
}

/// A BIP84 single-sig software signer holding the keys a watch-only wallet only sees publicly
pub(crate) struct TestSigner {
    wallet: bdk_wallet::Wallet,
    public_descriptor: String,
}

impl TestSigner {
    pub(crate) fn new(words: &str, network: bitcoin::Network) -> Self {
        let secp = Secp256k1::new();
        let mnemonic = Mnemonic::from_str(words).expect("test mnemonic parses");
        let master = Xpriv::new_master(network, &mnemonic.to_seed("")).expect("master key");

        let coin_type = if network == bitcoin::Network::Bitcoin { 0 } else { 1 };
        let account_path =
            DerivationPath::from_str(&format!("m/84'/{coin_type}'/0'")).expect("account path");

        let account_xpub = Xpub::from_priv(
            &secp,
            &master.derive_priv(&secp, &account_path).expect("account key derives"),
        );

        let origin = format!("{}/84h/{coin_type}h/0h", master.fingerprint(&secp));
        let multipath = format!("wpkh([{origin}]{account_xpub}/<0;1>/*)");

        // the checksummed multipath form is what hardware wallets export
        let public_descriptor = Descriptor::<DescriptorPublicKey>::from_str(&multipath)
            .expect("public descriptor parses")
            .to_string();

        let external = format!("wpkh({master}/84'/{coin_type}'/0'/0/*)");
        let internal = format!("wpkh({master}/84'/{coin_type}'/0'/1/*)");
        let wallet = bdk_wallet::Wallet::create(external, internal)
            .network(network)
            .create_wallet_no_persist()
            .expect("signer wallet builds");

        Self { wallet, public_descriptor }
    }

    /// The account descriptor a hardware wallet would export for watch-only import
    pub(crate) fn public_descriptor(&self) -> &str {
        &self.public_descriptor
    }

    /// Signs every input without finalizing, the way hardware signers return a PSBT
    #[allow(deprecated)] // SignOptions is the only signing API bdk_wallet offers
    pub(crate) fn sign(&self, psbt: &BdkPsbt) -> BdkPsbt {
        let mut signed = psbt.clone();
        let options = SignOptions { try_finalize: false, ..Default::default() };
        self.wallet.sign(&mut signed, options).expect("signer signs");

        for (index, input) in signed.inputs.iter().enumerate() {
            assert_eq!(input.partial_sigs.len(), 1, "signer signed input {index}");
        }

        signed
    }

    pub(crate) fn is_mine(&self, script: &ScriptBuf) -> bool {
        self.wallet.is_mine(script.clone())
    }

    /// A signed PSBT this signer builds on its own, which no Cove draft knows about
    pub(crate) fn signed_spend_of_its_own_funds(
        &mut self,
        recipient: &bitcoin::Address,
    ) -> BdkPsbt {
        insert_checkpoint(
            &mut self.wallet,
            BlockId { height: 1, hash: BlockHash::from_byte_array([3; 32]) },
        );
        receive_output_in_latest_block(&mut self.wallet, BdkAmount::from_sat(40_000));

        let mut builder = self.wallet.build_tx();
        builder.add_recipient(recipient.script_pubkey(), BdkAmount::from_sat(10_000));
        let psbt = builder.finish().expect("signer builds its own spend");

        self.sign(&psbt)
    }
}

/// A P2WPKH address that belongs to neither test signer
pub(crate) fn external_address(network: bitcoin::Network) -> bitcoin::Address {
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[7; 32]).expect("valid secret key");
    let pubkey = CompressedPublicKey(secret.public_key(&secp));

    bitcoin::Address::p2wpkh(&pubkey, network)
}

/// The Signet recipient of the hardware draft
pub(crate) fn external_recipient() -> Address {
    Address::new(external_address(bitcoin::Network::Signet))
}

/// Funds a persisted wallet with one confirmed output per amount, without a node
///
/// Spend-side wallet calls refuse to run before the first full scan, so this also records
/// that scan as done, which is what a real scan of the same chain would leave behind
pub(crate) fn fund_offline(id: &WalletId, amounts_sats: &[u64]) {
    let mut wallet = Wallet::try_load_persisted(id.clone()).expect("wallet loads");
    insert_checkpoint(
        &mut wallet.bdk,
        BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
    );

    for amount in amounts_sats {
        receive_output_in_latest_block(&mut wallet.bdk, BdkAmount::from_sat(*amount));
    }

    wallet.persist().expect("funded wallet persists");

    let mut metadata = wallet.metadata.clone();
    metadata.internal.performed_full_scan_at = Some(1);
    Database::global().wallets.replace_wallet_metadata(metadata).expect("metadata saves");
}

/// Builds and saves an unsigned draft the way the send flow does for a hardware wallet
///
/// `finalize_and_go_to_next_screen` calls `confirm_txn`, then saves the draft for cold
/// wallets before routing to the hardware export screen
pub(crate) async fn save_hardware_draft(
    manager: &RustWalletManager,
    recipient: &Address,
) -> ConfirmDetails {
    let details = manager
        .confirm_txn(
            Amount::from_sat(SEND_SATS),
            Arc::new(recipient.clone()),
            FeeRate::from_sat_per_vb(FEE_RATE_SAT_PER_VB),
        )
        .await
        .expect("draft builds");

    // the process-global drafts table outlives earlier tests, and an earlier draft with the
    // same txid would make this save a no-op that leaves the old wallet's record in place
    let drafts = Database::global().unsigned_transactions();
    if drafts.get_tx(&details.psbt.tx_id()).expect("draft lookup reads").is_some() {
        drafts.delete_tx(&details.psbt.tx_id()).expect("stale draft deletes");
    }

    manager.save_unsigned_transaction(Arc::new(details.clone())).expect("draft saves");

    details
}

/// Checks that a finalized transaction is exactly the reviewed draft, fully and validly signed
///
/// `owns_change` decides which outputs are change; it should come from the signer side so
/// the check does not trust Cove's own view of which scripts it owns
pub(crate) fn assert_matches_draft(
    transaction: &Transaction,
    details: &ConfirmDetails,
    recipient: &Address,
    owns_change: impl Fn(&ScriptBuf) -> bool,
) {
    let draft = &details.psbt.0;
    let draft_tx = &draft.unsigned_tx;

    let spent: Vec<OutPoint> =
        transaction.input.iter().map(|input| input.previous_output).collect();
    let drafted: Vec<OutPoint> = draft_tx.input.iter().map(|input| input.previous_output).collect();
    assert_eq!(spent, drafted, "finalized transaction spends the drafted inputs");
    assert_eq!(spent.len(), FUNDING_SATS.len(), "draft spends every funding output");

    assert_eq!(transaction.output, draft_tx.output, "finalized outputs are the drafted outputs");
    assert_eq!(transaction.compute_txid(), draft_tx.compute_txid(), "segwit txid is stable");

    let recipient_script = recipient.script_pubkey();
    let to_recipient: Vec<&TxOut> = transaction
        .output
        .iter()
        .filter(|output| output.script_pubkey == recipient_script)
        .collect();
    assert_eq!(to_recipient.len(), 1, "one output pays the recipient");
    assert_eq!(to_recipient[0].value.to_sat(), SEND_SATS);
    assert_eq!(details.sending_amount.as_sats(), SEND_SATS);

    let change: Vec<&TxOut> = transaction
        .output
        .iter()
        .filter(|output| output.script_pubkey != recipient_script)
        .collect();
    assert_eq!(change.len(), 1, "one change output");
    assert!(owns_change(&change[0].script_pubkey), "change returns to the signer's wallet");

    let prevouts: Vec<TxOut> = draft
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().expect("draft carries witness utxos"))
        .collect();

    let input_total: u64 = prevouts.iter().map(|prevout| prevout.value.to_sat()).sum();
    let output_total: u64 = transaction.output.iter().map(|output| output.value.to_sat()).sum();
    assert_eq!(input_total, FUNDING_SATS.iter().sum::<u64>());
    assert_eq!(input_total - output_total, details.fee_total.as_sats(), "fee is the reviewed fee");
    assert_eq!(change[0].value.to_sat(), input_total - SEND_SATS - details.fee_total.as_sats());

    assert_p2wpkh_witnesses_verify(transaction, &prevouts);
}

/// Verifies each input's P2WPKH signature against its sighash, so a finalized transaction with
/// a missing, malformed or wrong-key signature fails here instead of at broadcast
pub(crate) fn assert_p2wpkh_witnesses_verify(transaction: &Transaction, prevouts: &[TxOut]) {
    let secp = Secp256k1::verification_only();
    let mut cache = SighashCache::new(transaction);

    for (index, prevout) in prevouts.iter().enumerate() {
        let witness = &transaction.input[index].witness;
        assert_eq!(witness.len(), 2, "input {index} has a signature and a pubkey");

        let signature = bitcoin::ecdsa::Signature::from_slice(&witness[0])
            .unwrap_or_else(|error| panic!("input {index} signature parses: {error}"));
        let pubkey = CompressedPublicKey::from_slice(&witness[1])
            .unwrap_or_else(|error| panic!("input {index} pubkey parses: {error}"));

        assert_eq!(signature.sighash_type, EcdsaSighashType::All);
        assert_eq!(
            prevout.script_pubkey,
            ScriptBuf::new_p2wpkh(&pubkey.wpubkey_hash()),
            "input {index} pubkey owns the spent output"
        );

        let sighash = cache
            .p2wpkh_signature_hash(
                index,
                &prevout.script_pubkey,
                prevout.value,
                EcdsaSighashType::All,
            )
            .expect("sighash computes");

        secp.verify_ecdsa(&Message::from(sighash), &signature.signature, &pubkey.0)
            .unwrap_or_else(|error| panic!("input {index} signature verifies: {error}"));
    }
}

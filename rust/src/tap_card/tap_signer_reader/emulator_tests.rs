//! TAPSIGNER flows run against Coinkite's card emulator
//!
//! Every command goes through Cove's [`TapSignerReader`] and the [`TapcardTransportProtocol`]
//! callback the iOS and Android NFC layers implement, so these exercise Cove's setup, PIN,
//! derive and sign logic over real card APDUs. The tests need the emulator, so they are
//! ignored by default; `just test-tapsigner-emulator` installs it and runs them, and fails
//! with a message naming the missing piece when it is absent
//!
//! Each test starts its own emulated card. The emulator implements neither the 429 rate limit
//! nor `wait`, so the auth-delay paths are covered by the mock-transport tests in the parent
//! module instead: `connect_waits_out_auth_delay_before_any_command`,
//! `cvc_probe_waits_out_auth_delay_then_authenticates` and
//! `rate_limited_cvc_is_not_treated_as_a_wrong_cvc`

use bitcoin::{
    CompressedPublicKey, ScriptBuf,
    bip32::{ChildNumber, Xpub},
    secp256k1::Secp256k1,
};
use rust_cktap::shared::Nfc as _;

use super::*;
use crate::{
    database::Database,
    manager::wallet_manager::RustWalletManager,
    test_support::{
        hardware_signing::{
            FUNDING_SATS, assert_matches_draft, external_recipient, fund_offline,
            init_signet_globals, save_hardware_draft,
        },
        tap_signer_emulator::{EmulatedCard, FACTORY_CVC},
    },
    wallet::{Wallet, fingerprint::Fingerprint as WalletFingerprint, metadata::WalletId},
};

const NEW_CVC: &str = "246813";
const WRONG_CVC: &str = "000000";

/// What the user types into the advanced chain code field
const ADVANCED_CHAIN_CODE: [u8; 32] = *b"cove tapsigner advanced chaincod";

fn cvc(value: &str) -> Arc<TapSignerCvc> {
    Arc::new(TapSignerCvc::try_new(value.to_string()).expect("test CVC is valid"))
}

impl TapSignerReader {
    /// Connects the way [`TapSignerReader::new`] does, minus the factory certificate check,
    /// which the emulator's self-signed chain cannot pass
    async fn connect_emulated(card: &EmulatedCard, cmd: TapSignerCmd) -> Result<Self> {
        let operation = TapSignerOperation::try_from(cmd)?;
        let transport = TapcardTransport(Arc::new(card.nfc_transport().await));

        let card = match rust_cktap::shared::to_cktap(Arc::new(transport.clone()))
            .await
            .map_err(TransportError::from)?
        {
            CkTapCard::TapSigner(card) => VerifiedTapSigner(card),
            _ => panic!("the emulator was started as a TAPSIGNER"),
        };

        Self::with_card(card, transport, Some(operation)).await
    }
}

/// One NFC tap that runs one command, like each card prompt in the apps
async fn tap(card: &EmulatedCard, cmd: TapSignerCmd) -> Result<TapSignerResponse> {
    TapSignerReader::connect_emulated(card, cmd).await?.run().await
}

async fn set_up(card: &EmulatedCard, cmd: SetupCmd) -> TapSignerSetupComplete {
    match tap(card, TapSignerCmd::Setup(Arc::new(cmd))).await {
        Ok(TapSignerResponse::Setup(SetupCmdResponse::Complete(complete))) => complete,
        other => panic!("setup should complete: {other:?}"),
    }
}

fn assert_wrong_pin<T: fmt::Debug>(result: Result<T>) {
    match result {
        Err(error) if error.is_auth_error() => {}
        other => panic!("a wrong CVC should fail as a wrong PIN: {other:?}"),
    }
}

/// The card's master and current xpubs, read with rust-cktap alone so the check does not
/// rely on Cove's derive code
async fn card_xpubs(card: &EmulatedCard, cvc: &str) -> (Xpub, Xpub) {
    let cvc = rust_cktap::Cvc::try_from(cvc).expect("test CVC is valid");
    let mut session = card.tap().await;

    let master = session.xpub(true, &cvc).await.expect("master xpub reads");
    let current = session.xpub(false, &cvc).await.expect("current xpub reads");

    (master, current)
}

fn p2wpkh_at(account: &Xpub, keychain: u32, index: u32) -> ScriptBuf {
    let path = [ChildNumber::from_normal_idx(keychain), ChildNumber::from_normal_idx(index)]
        .map(|child| child.expect("normal child index"));

    let child = account.derive_pub(&Secp256k1::new(), &path).expect("child derives");

    ScriptBuf::new_p2wpkh(&CompressedPublicKey(child.public_key).wpubkey_hash())
}

/// Imports the card as a wallet the way the setup screens do: identify it from its NFC URL,
/// then build the wallet from the setup result
async fn import_wallet(card: &EmulatedCard, setup: &TapSignerSetupComplete) -> WalletId {
    let url = card.tap().await.nfc().await.expect("card returns its NFC URL");
    let cove_tap_card::TapCard::TapSigner(identity) =
        cove_tap_card::TapCard::parse(&format!("https://{url}")).expect("NFC URL parses")
    else {
        panic!("the emulator identifies as a TAPSIGNER: {url}");
    };

    let wallet = Wallet::try_new_persisted_from_tap_signer(
        identity,
        setup.derive_info.clone(),
        Some(setup.backup.clone()),
        Some(setup.birthday),
    )
    .expect("TAPSIGNER wallet imports");

    wallet.id
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs Coinkite's TAPSIGNER emulator; run `just test-tapsigner-emulator`"]
async fn setup_with_default_and_advanced_chain_codes_imports_the_card_account() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let _restore = init_signet_globals();

    let cases =
        [("default chain code", 1, None), ("advanced chain code", 2, Some(ADVANCED_CHAIN_CODE))];

    for (label, rng_seed, chain_code) in cases {
        let card = EmulatedCard::start_unset(rng_seed);
        let chain_code = chain_code.map(|chain_code| chain_code.to_vec());

        let cmd = SetupCmd::try_new(cvc(FACTORY_CVC), cvc(NEW_CVC), chain_code.clone())
            .expect("setup command builds");

        if let Some(chain_code) = &chain_code {
            assert_eq!(cmd.chain_code.as_slice(), chain_code.as_slice(), "{label}");
        }

        let setup = set_up(&card, cmd.clone()).await;
        let (master, account) = card_xpubs(&card, NEW_CVC).await;

        // the xpub and setup's report both reflect wherever the card's path points, so read the
        // path from the card itself to prove setup moved it off the factory m/84'/0'/0'
        let hardened = |index: u32| index | (1 << 31);
        let card_path = card.tap().await.path;
        assert_eq!(card_path, Some(vec![hardened(84), hardened(1), hardened(0)]), "{label}");

        // the card's master key carries exactly the chain code setup sent
        assert_eq!(master.chain_code.to_bytes(), cmd.chain_code, "{label}");
        assert_eq!(setup.derive_info.master_pubkey, master.public_key.serialize(), "{label}");
        assert_eq!(setup.derive_info.path, vec![84, 1, 0], "{label}: Signet account path");
        assert_eq!(setup.derive_info.pubkey, account.public_key.serialize(), "{label}");
        assert_eq!(setup.derive_info.chain_code, account.chain_code.to_bytes(), "{label}");
        assert!(!setup.backup.is_empty(), "{label}: setup returns the card backup");

        // setup finished by replacing the factory CVC
        assert_wrong_pin(tap(&card, TapSignerCmd::Derive { cvc: cvc(FACTORY_CVC) }).await);

        let id = import_wallet(&card, &setup).await;
        let wallet = Wallet::try_load_persisted(id).expect("imported wallet loads");

        assert_eq!(
            wallet.metadata.master_fingerprint.as_deref(),
            Some(&WalletFingerprint::from(master.fingerprint())),
            "{label}"
        );

        for index in 0..3 {
            let receive = wallet.bdk.peek_address(bdk_wallet::KeychainKind::External, index);
            let change = wallet.bdk.peek_address(bdk_wallet::KeychainKind::Internal, index);

            assert_eq!(receive.script_pubkey(), p2wpkh_at(&account, 0, index), "{label}");
            assert_eq!(change.script_pubkey(), p2wpkh_at(&account, 1, index), "{label}");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs Coinkite's TAPSIGNER emulator; run `just test-tapsigner-emulator`"]
async fn a_wrong_pin_changes_nothing_and_the_retry_succeeds() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let _restore = init_signet_globals();
    let card = EmulatedCard::start_unset(1);

    // the production reader refuses a card that does not chain to Coinkite's factory root
    let unverified = create_tap_signer_reader(card.nfc_transport().await, None).await;
    assert!(
        matches!(
            unverified,
            Err(TapSignerReaderError::TapSignerError(TransportError::IncorrectSignature(_)))
        ),
        "an emulated card is not a genuine TAPSIGNER: {unverified:?}"
    );

    // the emulator skips the CVC check on `new` for slot 0, so a wrong factory CVC picks the
    // key and then fails at the backup that follows; the retry below must then resume setup on
    // an initialized card instead of starting over
    let wrong = SetupCmd::try_new(cvc(WRONG_CVC), cvc(NEW_CVC), None).expect("command builds");
    assert_wrong_pin(tap(&card, TapSignerCmd::Setup(Arc::new(wrong))).await);
    assert!(card.tap().await.path.is_some(), "the emulator picked a key before the CVC failed");

    let cmd = SetupCmd::try_new(cvc(FACTORY_CVC), cvc(NEW_CVC), None).expect("command builds");
    let setup = set_up(&card, cmd).await;

    // a wrong PIN on a set-up card fails each command it guards, then the right PIN works
    assert_wrong_pin(tap(&card, TapSignerCmd::Derive { cvc: cvc(WRONG_CVC) }).await);
    assert_wrong_pin(tap(&card, TapSignerCmd::Backup { cvc: cvc(WRONG_CVC) }).await);

    match tap(&card, TapSignerCmd::Derive { cvc: cvc(NEW_CVC) }).await {
        Ok(TapSignerResponse::Import(derive_info)) => assert_eq!(derive_info, setup.derive_info),
        other => panic!("derive with the right CVC should import: {other:?}"),
    }

    match tap(&card, TapSignerCmd::Backup { cvc: cvc(NEW_CVC) }).await {
        Ok(TapSignerResponse::Backup(backup)) => assert_eq!(backup.len(), setup.backup.len()),
        other => panic!("backup with the right CVC should succeed: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs Coinkite's TAPSIGNER emulator; run `just test-tapsigner-emulator`"]
async fn signs_a_hardware_draft_that_then_finalizes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let _restore = init_signet_globals();
    let card = EmulatedCard::start_unset(1);

    let cmd = SetupCmd::try_new(cvc(FACTORY_CVC), cvc(NEW_CVC), None).expect("command builds");
    let setup = set_up(&card, cmd).await;
    let (_, account) = card_xpubs(&card, NEW_CVC).await;

    let id = import_wallet(&card, &setup).await;
    fund_offline(&id, &FUNDING_SATS);

    let manager = RustWalletManager::try_new(id).expect("wallet manager loads");
    let recipient = external_recipient();
    let details = save_hardware_draft(&manager, &recipient).await;
    let draft = Arc::new(details.psbt.clone());

    assert_wrong_pin(
        tap(&card, TapSignerCmd::Sign { psbt: draft.clone(), cvc: cvc(WRONG_CVC) }).await,
    );

    let signed = match tap(&card, TapSignerCmd::Sign { psbt: draft, cvc: cvc(NEW_CVC) }).await {
        Ok(TapSignerResponse::Sign(signed)) => signed,
        other => panic!("signing with the right CVC should return the signed PSBT: {other:?}"),
    };

    for (index, input) in signed.0.inputs.iter().enumerate() {
        assert_eq!(input.partial_sigs.len(), 1, "card signed input {index}");
    }

    // the signed PSBT returns through the same draft lookup and finalize as a QR import
    let record = Database::global()
        .unsigned_transactions()
        .get_tx(&signed.tx_id())
        .expect("draft lookup reads")
        .expect("signed PSBT matches the saved draft");

    assert_eq!(record.wallet_id, manager.id);

    let transaction = manager.finalize_psbt(signed).await.expect("signed draft finalizes");
    assert_matches_draft(&transaction.0, &details, &recipient, |script| {
        (0..10).any(|index| *script == p2wpkh_at(&account, 1, index))
    });
}

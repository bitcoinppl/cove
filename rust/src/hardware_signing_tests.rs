//! End-to-end tests of the watch-only hardware wallet PSBT loop over animated QR codes
//!
//! Cove builds and saves a draft, exports it as UR and BBQr frames, and an independent signer
//! decodes, signs and re-encodes it. The signed frames then come back one at a time through
//! the scanner, the draft lookup and the finalize call the hardware send screen uses, so a
//! regression anywhere in that loop fails here instead of in a camera session

use std::sync::Arc;

use bbqr::{
    encode::Encoding,
    file_type::FileType,
    join::Joined,
    qr::Version,
    split::{Split, SplitOptions},
};
use bdk_wallet::bitcoin::{self, Psbt as BdkPsbt};
use foundation_ur::{Decoder as UrDecoder, Encoder as UrEncoder, UR};

use crate::{
    database::{
        Database, test_support::RestoreWalletTarget,
        unsigned_transactions::UnsignedTransactionsTableError,
    },
    manager::wallet_manager::{RustWalletManager, WalletManagerError},
    multi_format::{MultiFormat, StringOrData},
    qr_scanner::{QrScannerFFI, ScanProgress, ScanResult},
    test_support::hardware_signing::{
        FUNDING_SATS, SIGNER_MNEMONIC, TestSigner, UNRELATED_SIGNER_MNEMONIC, assert_matches_draft,
        external_address, external_recipient, fund_offline, init_signet_globals,
        save_hardware_draft,
    },
    wallet::{Address, Wallet, metadata::WalletType},
};
use cove_types::{
    confirm::{ConfirmDetails, QrDensity},
    psbt::Psbt,
};

/// Fragment length for the signer's UR frames, small enough to need many frames
const SIGNED_UR_FRAGMENT_LEN: usize = 60;

struct Fixture {
    manager: RustWalletManager,
    signer: TestSigner,
    recipient: Address,
    details: ConfirmDetails,
    // dropped last so the network is restored after the manager shuts down
    _restore: RestoreWalletTarget,
}

/// A funded Signet watch-only wallet imported from the signer's public descriptor, with one
/// saved hardware draft
async fn hardware_wallet_with_draft() -> Fixture {
    let restore = init_signet_globals();
    let signer = TestSigner::new(SIGNER_MNEMONIC, bitcoin::Network::Signet);
    let id = {
        let wallet = Wallet::try_new_persisted_from_xpub(signer.public_descriptor().to_string())
            .expect("watch-only wallet imports");

        // the hardware send screen only saves drafts for cold and xpub-only wallets
        assert_eq!(wallet.metadata.wallet_type, WalletType::Cold);
        wallet.id.clone()
    };

    fund_offline(&id, &FUNDING_SATS);

    let manager = RustWalletManager::try_new(id).expect("wallet manager loads");
    let recipient = external_recipient();
    let details = save_hardware_draft(&manager, &recipient).await;

    Fixture { manager, signer, recipient, details, _restore: restore }
}

/// Smaller QR codes than the default, as a user picks on the export screen, so both
/// formats animate over several frames
fn low_density() -> QrDensity {
    (0..5).fold(QrDensity::new(), |density, _| density.decrease())
}

/// What a signer reads from Cove's animated UR export, decoded without Cove's UR code
fn read_cove_ur_export(frames: &[String]) -> BdkPsbt {
    assert!(frames.len() > 2, "export animates over several frames, got {}", frames.len());

    let mut decoder = UrDecoder::default();
    for frame in frames {
        assert!(frame.starts_with("ur:crypto-psbt/"), "frame is a crypto-psbt UR: {frame}");
        decoder.receive(UR::parse(frame).expect("frame parses")).expect("frame decodes");
    }

    assert!(decoder.is_complete(), "every exported frame together completes the message");
    let cbor = decoder.message().expect("message decodes").expect("message is complete");

    // crypto-psbt is an untagged CBOR byte string holding the PSBT
    let ciborium::Value::Bytes(psbt) = ciborium::from_reader(cbor).expect("CBOR decodes") else {
        panic!("crypto-psbt payload is a CBOR byte string");
    };

    BdkPsbt::deserialize(&psbt).expect("PSBT parses")
}

/// What a signer reads from Cove's animated BBQr export, joined by the bbqr crate alone
fn read_cove_bbqr_export(frames: &[String]) -> BdkPsbt {
    assert!(frames.len() > 2, "export animates over several frames, got {}", frames.len());

    let joined = Joined::try_from_parts(frames.to_vec()).expect("BBQr frames join");
    assert_eq!(joined.file_type, FileType::Psbt);

    BdkPsbt::deserialize(&joined.data).expect("PSBT parses")
}

/// The signer's UR frames: every pure part, then fountain parts a scanner can use to fill gaps
fn signed_ur_frames(psbt: &BdkPsbt, fountain_parts: usize) -> (Vec<String>, Vec<String>) {
    let mut cbor = Vec::new();
    ciborium::into_writer(&ciborium::Value::Bytes(psbt.serialize()), &mut cbor)
        .expect("CBOR encodes");

    let mut encoder = UrEncoder::new();
    encoder.start("crypto-psbt", &cbor, SIGNED_UR_FRAGMENT_LEN);

    let pure_count = encoder.sequence_count() as usize;
    let pure = (0..pure_count).map(|_| encoder.next_part().to_string()).collect();
    let fountain = (0..fountain_parts).map(|_| encoder.next_part().to_string()).collect();

    (pure, fountain)
}

/// The signer's BBQr frames, split by the bbqr crate at the smallest common QR size
fn signed_bbqr_frames(psbt: &BdkPsbt) -> Vec<String> {
    let options = SplitOptions {
        encoding: Encoding::Zlib,
        min_split_number: 1,
        max_split_number: 100,
        min_version: Version::V01,
        max_version: Version::V05,
    };

    Split::try_from_data(&psbt.serialize(), FileType::Psbt, options).expect("BBQr splits").parts
}

/// Reversed, with frames the camera saw twice, as a scanner meets a looping animation
/// joined midway
fn camera_order(frames: &[String]) -> Vec<String> {
    let mut order: Vec<String> = frames.iter().rev().cloned().collect();
    order.insert(1, order[0].clone());
    order.insert(4, order[2].clone());

    order
}

/// Feeds frames one at a time and returns the scanned data, checking that the scan completes
/// exactly on the last frame and reports progress, not an error, on every earlier one
fn scan_frames(scanner: &QrScannerFFI, frames: &[String]) -> MultiFormat {
    let (last, earlier) = frames.split_last().expect("at least one frame");

    for (index, frame) in earlier.iter().enumerate() {
        match scanner.scan(StringOrData::String(frame.clone())) {
            Ok(ScanResult::InProgress { .. }) => {}
            other => panic!("frame {index} of {} should be in progress: {other:?}", frames.len()),
        }
    }

    match scanner.scan(StringOrData::String(last.clone())) {
        Ok(ScanResult::Complete { data, .. }) => data,
        other => panic!("last frame should complete the scan: {other:?}"),
    }
}

/// Feeds frames that never add up to the whole message and checks the scan stays incomplete
fn assert_scan_stays_incomplete(scanner: &QrScannerFFI, frames: &[String]) -> ScanProgress {
    let mut progress = None;

    for (index, frame) in frames.iter().enumerate() {
        match scanner.scan(StringOrData::String(frame.clone())) {
            Ok(ScanResult::InProgress { progress: current, .. }) => progress = Some(current),
            other => panic!("truncated frame {index} must not complete the scan: {other:?}"),
        }
    }

    progress.expect("at least one frame")
}

fn scanned_psbt(data: MultiFormat) -> Psbt {
    match data {
        MultiFormat::SignedPsbt(psbt) => Arc::unwrap_or_clone(psbt),
        other => panic!("scan should produce a signed PSBT, got {other:?}"),
    }
}

/// Runs the import half of the hardware send screen: find the saved draft for the scanned
/// PSBT, then finalize it through the draft's wallet
async fn import_and_finalize(fixture: &Fixture, scanned: Psbt) -> bitcoin::Transaction {
    let record = Database::global()
        .unsigned_transactions()
        .get_tx(&scanned.tx_id())
        .expect("draft lookup reads")
        .expect("scanned PSBT matches the saved draft");

    assert_eq!(record.wallet_id, fixture.manager.id);
    assert_eq!(record.confirm_details, fixture.details, "draft shows what the user reviewed");

    let transaction =
        fixture.manager.finalize_psbt(Arc::new(scanned)).await.expect("signed draft finalizes");

    transaction.0
}

#[tokio::test(flavor = "current_thread")]
async fn signed_animated_ur_and_bbqr_from_an_external_signer_finalize_the_saved_draft() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let fixture = hardware_wallet_with_draft().await;
    let density = low_density();

    let from_ur = read_cove_ur_export(
        &fixture.details.psbt_to_ur_with_density(&density).expect("UR export encodes"),
    );
    let from_bbqr = read_cove_bbqr_export(
        &fixture.details.psbt_to_bbqr_with_density(&density).expect("BBQr export encodes"),
    );

    assert_eq!(from_ur, fixture.details.psbt.0, "UR export carries the draft PSBT");
    assert_eq!(from_bbqr, fixture.details.psbt.0, "BBQr export carries the draft PSBT");

    let signed = fixture.signer.sign(&from_ur);
    let scanner = QrScannerFFI::new();

    // UR: every pure part out of order, with repeats
    let (pure, fountain) = signed_ur_frames(&signed, 0);
    assert!(pure.len() > 3, "signed PSBT animates over several UR frames");

    let scanned = scanned_psbt(scan_frames(&scanner, &camera_order(&pure)));
    assert_eq!(scanned.0, signed, "UR scan returns the signed PSBT unchanged");
    assert!(fountain.is_empty());

    let transaction = import_and_finalize(&fixture, scanned).await;
    assert_matches_draft(&transaction, &fixture.details, &fixture.recipient, |script| {
        fixture.signer.is_mine(script)
    });

    // UR: a missed frame stays incomplete until fountain parts make up for it
    scanner.reset();
    let (pure, fountain) = signed_ur_frames(&signed, 2 * pure.len());
    let missing_one: Vec<String> = pure.iter().skip(1).cloned().collect();

    let ScanProgress::Ur { percentage } = assert_scan_stays_incomplete(&scanner, &missing_one)
    else {
        panic!("UR scan reports UR progress");
    };
    assert!(percentage < 1.0, "a scan missing a frame is not complete: {percentage}");

    let completing = fountain
        .iter()
        .position(|frame| {
            matches!(
                scanner.scan(StringOrData::String(frame.clone())),
                Ok(ScanResult::Complete { .. })
            )
        })
        .expect("fountain parts complete the scan");

    let Ok(ScanResult::Complete { data, .. }) =
        scanner.scan(StringOrData::String(fountain[completing].clone()))
    else {
        panic!("a completed scan keeps returning its result until reset");
    };
    assert_eq!(scanned_psbt(data).0, signed, "fountain-recovered scan returns the signed PSBT");

    // BBQr: out of order, with repeats
    scanner.reset();
    let bbqr = signed_bbqr_frames(&fixture.signer.sign(&from_bbqr));
    assert!(bbqr.len() > 3, "signed PSBT animates over several BBQr frames");

    let scanned = scanned_psbt(scan_frames(&scanner, &camera_order(&bbqr)));
    assert_eq!(scanned.0, signed, "BBQr scan returns the signed PSBT unchanged");

    let transaction = import_and_finalize(&fixture, scanned).await;
    assert_matches_draft(&transaction, &fixture.details, &fixture.recipient, |script| {
        fixture.signer.is_mine(script)
    });
}

#[tokio::test(flavor = "current_thread")]
async fn signed_psbts_that_match_no_saved_draft_are_rejected() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let fixture = hardware_wallet_with_draft().await;
    let drafts = Database::global().unsigned_transactions();

    // truncated BBQr: one frame short never yields a PSBT
    let signed = fixture.signer.sign(&fixture.details.psbt.0);
    let bbqr = signed_bbqr_frames(&signed);
    let scanner = QrScannerFFI::new();

    let ScanProgress::Bbqr { scanned, total } =
        assert_scan_stays_incomplete(&scanner, &bbqr[..bbqr.len() - 1])
    else {
        panic!("BBQr scan reports BBQr progress");
    };
    assert_eq!((scanned, total), (bbqr.len() as u32 - 1, bbqr.len() as u32));

    // another signer's PSBT, and a PSBT the same seed signed on mainnet, match no draft and
    // cannot be finalized by this wallet
    let foreign = [
        ("unrelated signer", UNRELATED_SIGNER_MNEMONIC, bitcoin::Network::Signet),
        ("mainnet signer", SIGNER_MNEMONIC, bitcoin::Network::Bitcoin),
    ];

    for (label, words, network) in foreign {
        let mut signer = TestSigner::new(words, network);
        let signed = signer.signed_spend_of_its_own_funds(&external_address(network));
        let (pure, _) = signed_ur_frames(&signed, 0);

        scanner.reset();
        let scanned = scanned_psbt(scan_frames(&scanner, &pure));

        assert_eq!(drafts.get_tx(&scanned.tx_id()).expect("lookup reads"), None, "{label}");
        assert_eq!(
            drafts._get_tx_throw(Arc::new(scanned.tx_id())).err(),
            Some(UnsignedTransactionsTableError::NoRecordFound),
            "{label}: file, paste and NFC imports report the missing draft"
        );

        let finalized = fixture.manager.finalize_psbt(Arc::new(scanned)).await;
        assert!(
            matches!(finalized, Err(WalletManagerError::PsbtFinalizeError(_))),
            "{label}: this wallet must not finalize a PSBT it did not draft: {finalized:?}"
        );
    }

    let draft_id = fixture.details.psbt.tx_id();

    // a correctly signed PSBT for a draft the user deleted
    fixture.manager.delete_unsigned_transaction(Arc::new(draft_id)).expect("draft deletes");
    assert!(fixture.manager.get_unsigned_transactions().expect("drafts list").is_empty());

    scanner.reset();
    let scanned = scanned_psbt(scan_frames(&scanner, &bbqr));
    assert_eq!(scanned.tx_id(), draft_id, "the scan itself still succeeds");

    assert_eq!(drafts.get_tx(&draft_id).expect("lookup reads"), None);
    assert_eq!(
        drafts._get_tx_throw(Arc::new(draft_id)).err(),
        Some(UnsignedTransactionsTableError::NoRecordFound),
        "importing a signature for a deleted draft reports the missing draft"
    );
}

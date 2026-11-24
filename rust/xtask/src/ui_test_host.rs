//! Host services for UI tests that sign with a hardware wallet
//!
//! Debug builds of the app have two test seams: the QR scanner can read frames
//! from a script file instead of the camera, and TAPSIGNER commands can go to a
//! TCP bridge instead of an NFC session. This module serves the host side of
//! both on the run's block of [`UiTestPorts`], reachable from the iOS simulator
//! over the shared loopback and from Android through `adb reverse`:
//!
//! - [`UiTestPorts::signer_control`]: a JSON API. `POST /sign-psbt` signs a PSBT the way a
//!   hardware wallet would and returns it as UR and BBQr frames for the QR
//!   seam, `/unrelated-psbt` signs one that matches no draft, and
//!   `/watch-only-descriptor` returns the account a hardware wallet exports.
//!   `/join-export-frames` joins the UR or BBQr frames a test read off the
//!   app's PSBT export back into the PSBT ([`export_frames`]).
//!   `/public-key-export` returns an account's bare SLIP-132 public key, as a
//!   plain QR and as a single-part BBQr, a native segwit account also as a
//!   single-frame crypto-hdkey UR, with the first receive address it should
//!   import as. `/simulator-pasteboard` reads an iOS simulator's
//!   pasteboard, `/simulator-appearance` switches it between light and
//!   dark mode, and `/simulator-content-size` reads its text size and, given
//!   one, changes it while the app runs. The `/tapsigner/...` routes start
//!   the card emulator, set up or factory fresh, and inject dropped
//!   sessions. The `/keyteleport/...` routes play the other phone in a
//!   KeyTeleport transfer ([`key_teleport`]), and the `/payjoin/...` routes
//!   play the receiver of a Payjoin payment ([`payjoin`])
//! - [`UiTestPorts::tapsigner_bridge`]: the TAPSIGNER bridge. Each APDU arrives as a frame with a
//!   4-byte big-endian length, goes to Coinkite's `ecard.py` emulator, and the
//!   card's response goes back the same way. Closing the connection is how a
//!   dropped NFC session looks to the app
//! - [`UiTestPorts::payjoin_relay`]: the OHTTP relay in front of a BIP 77
//!   directory on this machine, which the app's debug-only
//!   `COVE_UI_TEST_PAYJOIN_RELAY` seam sends Payjoin requests through

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    process::Command,
    sync::Arc,
    thread,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use bdk_wallet::KeychainKind;
use bip39::Mnemonic;
use bitcoin::{
    absolute::LockTime,
    base58,
    bip32::{DerivationPath, Fingerprint, Xpriv, Xpub},
    hashes::Hash,
    secp256k1::Secp256k1,
    transaction, Address, Amount, CompressedPublicKey, FeeRate, Network, NetworkKind, OutPoint,
    Psbt, Transaction, TxIn, TxOut, Txid,
};
use clap::Subcommand;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};

mod card_emulator;
mod export_frames;
mod key_teleport;
mod payjoin;

use self::card_emulator::{CardKind, Cards};
use crate::{
    android_device::AndroidDevice,
    common::{print_info, print_success, print_warning},
    fake_node::{AndroidPorts, AndroidTarget, FakeNodeGuard, ReversedPorts},
    shutdown::{self, Registration, Stop},
    ui_test_ports::{PortReservation, UiTestPorts},
    util,
};

// rust-cktap opens every session with this applet SELECT; the emulator has no
// applet, so the bridge answers it with a status command as a real card does
const SELECT_APDU: &[u8] = b"\x00\xa4\x04\x00\x0f\xf0CoinkiteCARDv1";
const STATUS_REQUEST: &[u8] = b"\xa1\x63cmd\x66status";
const CBOR_APDU_HEADER: &[u8] = &[0x00, 0xcb, 0x00, 0x00];
const STATUS_OK: [u8; 2] = [0x90, 0x00];
const INSTRUCTION_NOT_SUPPORTED: [u8; 2] = [0x6d, 0x00];

#[derive(Subcommand)]
pub enum UiTestHostCommand {
    /// Start the fake node and the hardware signer services on the first free port block, run a command with COVE_UI_TEST_PORT_BASE set to the block, then stop them
    Run {
        /// Forward the ports with adb reverse to an emulator, the one ANDROID_SERIAL names or else the only connected one, and fail when there is none. Without it, ports go to such an emulator only if one is connected. Physical devices are never used, and the command runs with ANDROID_SERIAL set to the emulator
        #[arg(long)]
        android: bool,

        /// Command and arguments to run after --, from the current directory
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

pub fn run(command: UiTestHostCommand) -> Result<()> {
    match command {
        UiTestHostCommand::Run { android, command } => {
            let android = if android { AndroidPorts::Required } else { AndroidPorts::IfConnected };
            let host = UiTestHost::start(android)?;

            crate::fake_node::run_command(&command, host.ports(), host.android_device())
        }
    }
}

/// Everything the full-launch UI tests reach on the host; dropping it stops them
pub struct UiTestHost {
    // dropped in this order: the signer and its forwards, the node and its forwards, then the
    // device lock and the port block, so the next run on either starts after these have stopped
    _signer: HardwareSignerGuard,
    _node: FakeNodeGuard,
    android: Option<AndroidTarget>,
    ports: PortReservation,
}

impl UiTestHost {
    /// Locks the Android device first, as scripts take the emulator lock before the UI lock,
    /// then reserves a port block and starts both services on it
    pub fn start(android: AndroidPorts) -> Result<Self> {
        let android = android.target()?;
        let ports = PortReservation::take()?;
        let device = android.as_ref().map(AndroidTarget::device);
        let node = FakeNodeGuard::start(ports.ports(), device)?;
        let signer = HardwareSignerGuard::start(ports.ports(), device)?;

        Ok(Self { _signer: signer, _node: node, android, ports })
    }

    pub const fn ports(&self) -> UiTestPorts {
        self.ports.ports()
    }

    /// The device the ports are forwarded to, if any
    pub fn android_device(&self) -> Option<&AndroidDevice> {
        self.android.as_ref().map(AndroidTarget::device)
    }
}

/// The control API and TAPSIGNER bridge; dropping it, or a signal, removes the port forwards and
/// stops the card emulator
///
/// The servers run on threads of this process, which ends with the test run
pub struct HardwareSignerGuard {
    signer: Arc<HardwareSigner>,
    _registration: Registration,
}

struct HardwareSigner {
    state: Arc<SignerState>,
    reversed: ReversedPorts,
}

impl HardwareSignerGuard {
    pub fn start(ports: UiTestPorts, android: Option<&AndroidDevice>) -> Result<Self> {
        let listen = |port: u16| {
            TcpListener::bind(("127.0.0.1", port)).wrap_err_with(|| {
                format!("Failed to listen on {port}; is another program using it?")
            })
        };
        let control = listen(ports.signer_control())?;
        let bridge = listen(ports.tapsigner_bridge())?;

        // logs named for the port block, so concurrent runs from one worktree keep their own
        let cards = Cards::prepare(ports.base())?;
        let payjoin = payjoin::PayjoinHost::start(ports)?;
        let state = Arc::new(SignerState::new(cards).with_payjoin(payjoin));

        let signer =
            Arc::new(HardwareSigner { state: state.clone(), reversed: ReversedPorts::default() });
        let guard = Self { _registration: shutdown::register(signer.clone()), signer };

        spawn_server(control, state.clone(), serve_control);
        spawn_server(bridge, state, serve_bridge);
        print_success(&format!(
            "Hardware signer up: control http://127.0.0.1:{}, TAPSIGNER bridge 127.0.0.1:{}",
            ports.signer_control(),
            ports.tapsigner_bridge()
        ));

        if let Some(device) = android {
            guard.signer.forward(device, ports)?;
        }

        Ok(guard)
    }
}

impl HardwareSigner {
    fn forward(&self, device: &AndroidDevice, ports: UiTestPorts) -> Result<()> {
        self.reversed.add(device, UiTestPorts::DEVICE.signer().into_iter().zip(ports.signer()))?;
        print_info(&format!("Forwarded hardware signer ports to {}", device.description()));
        Ok(())
    }
}

impl Stop for HardwareSigner {
    fn stop(&self) {
        self.reversed.remove(UiTestPorts::DEVICE.signer());
        self.state.cards.stop();
    }
}

impl Drop for HardwareSignerGuard {
    fn drop(&mut self) {
        self.signer.stop();
    }
}

struct SignerState {
    cards: Cards,
    // the next bridge session closes after this many exchanges
    drop_after: Mutex<Option<u32>>,
    key_teleport_receivers: key_teleport::Receivers,
    // none in the card emulator's own tests, which serve no Payjoin routes
    payjoin: Option<payjoin::PayjoinHost>,
}

impl SignerState {
    fn new(cards: Cards) -> Self {
        Self {
            cards,
            drop_after: Mutex::new(None),
            key_teleport_receivers: key_teleport::Receivers::default(),
            payjoin: None,
        }
    }

    fn with_payjoin(self, payjoin: payjoin::PayjoinHost) -> Self {
        Self { payjoin: Some(payjoin), ..self }
    }

    fn payjoin(&self) -> Result<&payjoin::PayjoinHost> {
        self.payjoin.as_ref().ok_or_else(|| eyre!("The Payjoin services aren't running"))
    }
}

fn spawn_server(
    listener: TcpListener,
    state: Arc<SignerState>,
    serve: fn(TcpStream, &SignerState) -> Result<()>,
) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let state = state.clone();

            thread::spawn(move || {
                if let Err(error) = serve(stream, &state) {
                    print_warning(&format!("Hardware signer: {error:#}"));
                }
            });
        }
    });
}

// ----------------------------------------------------------------------------
// Control API
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignPsbtRequest {
    /// The unsigned PSBT as base64 or hex
    psbt: String,
    mnemonic: String,
    #[serde(default = "default_network")]
    network: String,
    /// Split the signed PSBT into at least this many UR and BBQr frames, so a
    /// test can scan an animated QR
    #[serde(default = "default_min_parts")]
    min_parts: usize,
}

fn default_network() -> String {
    "signet".to_string()
}

fn default_min_parts() -> usize {
    1
}

/// A signed PSBT that spends a made-up output, so it matches no transaction
/// the app drafted
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnrelatedPsbtRequest {
    mnemonic: String,
    #[serde(default = "default_network")]
    network: String,
    #[serde(default = "default_min_parts")]
    min_parts: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WalletRequest {
    mnemonic: String,
    #[serde(default = "default_network")]
    network: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKeyRequest {
    mnemonic: String,
    #[serde(default = "default_network")]
    network: String,
    address_type: AddressType,
}

/// The single-signature account types a bare SLIP-132 key names
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum AddressType {
    /// BIP84 P2WPKH, a zpub or vpub
    NativeSegwit,
    /// BIP49 P2SH-P2WPKH, a ypub or upub
    WrappedSegwit,
}

impl AddressType {
    const fn purpose(self) -> u32 {
        match self {
            Self::NativeSegwit => 84,
            Self::WrappedSegwit => 49,
        }
    }

    /// SLIP-132 version bytes of the account's extended public key
    const fn slip132_version(self, network: NetworkKind) -> [u8; 4] {
        match (self, network) {
            (Self::NativeSegwit, NetworkKind::Main) => [0x04, 0xb2, 0x47, 0x46],
            (Self::NativeSegwit, NetworkKind::Test) => [0x04, 0x5f, 0x1c, 0xf6],
            (Self::WrappedSegwit, NetworkKind::Main) => [0x04, 0x9d, 0x7c, 0xb2],
            (Self::WrappedSegwit, NetworkKind::Test) => [0x04, 0x4a, 0x52, 0x62],
        }
    }

    fn address(self, key: CompressedPublicKey, network: Network) -> Address {
        match self {
            Self::NativeSegwit => Address::p2wpkh(&key, network),
            Self::WrappedSegwit => Address::p2shwpkh(&key, network),
        }
    }
}

/// Starts the emulated TAPSIGNER; an empty body starts one that is set up
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TapSignerStartRequest {
    /// Start a factory-fresh card with no key yet, which the app sets up
    #[serde(default)]
    needs_setup: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteboardRequest {
    /// The simulator's UDID, which the XCUITest runner reads from `SIMULATOR_UDID`
    udid: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppearanceRequest {
    udid: String,
    appearance: Appearance,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Appearance {
    Light,
    Dark,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentSizeRequest {
    udid: String,
    /// The size to set; without it the request only reads the current one
    #[serde(default)]
    content_size: Option<ContentSize>,
}

/// The preferred content size categories, named as `simctl ui content_size` takes and prints them
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum ContentSize {
    ExtraSmall,
    Small,
    Medium,
    Large,
    ExtraLarge,
    ExtraExtraLarge,
    ExtraExtraExtraLarge,
    AccessibilityMedium,
    AccessibilityLarge,
    AccessibilityExtraLarge,
    AccessibilityExtraExtraLarge,
    AccessibilityExtraExtraExtraLarge,
}

impl ContentSize {
    fn name(self) -> &'static str {
        match self {
            Self::ExtraSmall => "extra-small",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::ExtraLarge => "extra-large",
            Self::ExtraExtraLarge => "extra-extra-large",
            Self::ExtraExtraExtraLarge => "extra-extra-extra-large",
            Self::AccessibilityMedium => "accessibility-medium",
            Self::AccessibilityLarge => "accessibility-large",
            Self::AccessibilityExtraLarge => "accessibility-extra-large",
            Self::AccessibilityExtraExtraLarge => "accessibility-extra-extra-large",
            Self::AccessibilityExtraExtraExtraLarge => "accessibility-extra-extra-extra-large",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BiometricsRequest {
    udid: String,
    action: BiometricsAction,
}

/// What the Simulator app's Features > Face ID menu does, as `simctl` can do it from the host
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum BiometricsAction {
    /// Enrolls a face, so `LAContext` reports Face ID as available
    Enroll,
    /// Removes the enrolled face
    Unenroll,
    /// Answers the Face ID prompt on screen with the enrolled face
    Match,
    /// Answers the Face ID prompt on screen with a face that isn't enrolled
    Nomatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DropRequest {
    after_exchanges: u32,
}

fn serve_control(stream: TcpStream, state: &SignerState) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let (method, path, body) = read_http_request(&mut reader)?;

    let (status, body) = match route(state, &method, &path, &body) {
        Ok(value) => (200, value),
        Err(error) => (400, json!({ "error": format!("{error:#}") })),
    };

    write_http_response(stream, status, &body)
}

fn route(state: &SignerState, method: &str, path: &str, body: &[u8]) -> Result<Value> {
    match (method, path) {
        ("GET", "/health") => Ok(json!({ "ok": true })),
        ("POST", "/sign-psbt") => sign_psbt(serde_json::from_slice(body)?),
        ("POST", "/unrelated-psbt") => unrelated_psbt(serde_json::from_slice(body)?),
        ("POST", "/join-export-frames") => export_frames::join(serde_json::from_slice(body)?),
        ("POST", "/watch-only-descriptor") => watch_only_descriptor(serde_json::from_slice(body)?),
        ("POST", "/public-key-export") => public_key_export(serde_json::from_slice(body)?),
        ("POST", "/simulator-pasteboard") => simulator_pasteboard(serde_json::from_slice(body)?),
        ("POST", "/simulator-appearance") => simulator_appearance(serde_json::from_slice(body)?),
        ("POST", "/simulator-content-size") => {
            simulator_content_size(serde_json::from_slice(body)?)
        }
        ("POST", "/simulator-biometrics") => simulator_biometrics(serde_json::from_slice(body)?),

        ("POST", "/tapsigner/start") => {
            let request: TapSignerStartRequest = if body.is_empty() {
                TapSignerStartRequest::default()
            } else {
                serde_json::from_slice(body)?
            };

            let kind = if request.needs_setup { CardKind::NeedsSetup } else { CardKind::SetUp };
            *state.drop_after.lock() = None;
            state.cards.start(kind)?;

            Ok(json!({ "ok": true }))
        }
        ("POST", "/tapsigner/drop") => {
            let request: DropRequest = serde_json::from_slice(body)?;
            *state.drop_after.lock() = Some(request.after_exchanges);
            Ok(json!({ "ok": true }))
        }
        ("POST", "/keyteleport/send") => key_teleport::send(serde_json::from_slice(body)?),
        ("POST", "/keyteleport/receive-request") => state.key_teleport_receivers.receive_request(),
        ("POST", "/keyteleport/receive-response") => {
            state.key_teleport_receivers.receive_response(serde_json::from_slice(body)?)
        }
        ("POST", "/payjoin/receiver") => {
            state.payjoin()?.open_receiver(serde_json::from_slice(body)?)
        }
        ("POST", "/payjoin/original") => state.payjoin()?.wait_for_original(),
        ("POST", "/payjoin/respond") => state.payjoin()?.respond(),
        _ => Err(eyre!("No route for {method} {path}")),
    }
}

fn sign_psbt(request: SignPsbtRequest) -> Result<Value> {
    let network = util::parse_network(&request.network)?;
    let mnemonic = Mnemonic::parse(&request.mnemonic).wrap_err("Invalid mnemonic")?;

    let input = request.psbt.trim();
    let bytes = match hex::decode(input) {
        Ok(bytes) => bytes,
        Err(_) => STANDARD.decode(input).wrap_err("The PSBT is neither hex nor base64")?,
    };

    let mut psbt = Psbt::deserialize(&bytes).wrap_err("Failed to parse the PSBT")?;
    let wallet = util::bip84_signing_wallet(&mnemonic, network)?;
    if !util::sign_without_finalizing(&wallet, &mut psbt)? {
        bail!("The mnemonic owns none of the PSBT's inputs");
    }

    signed_frames(&psbt, request.min_parts)
}

fn unrelated_psbt(request: UnrelatedPsbtRequest) -> Result<Value> {
    let network = util::parse_network(&request.network)?;
    let mnemonic = Mnemonic::parse(&request.mnemonic).wrap_err("Invalid mnemonic")?;
    let mut wallet = util::bip84_signing_wallet(&mnemonic, network)?;
    let address = wallet.reveal_next_address(KeychainKind::External).address;

    // no node has seen this funding transaction, so no app draft can spend it
    let funding = Transaction {
        version: transaction::Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array([0x42; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_000),
            script_pubkey: address.script_pubkey(),
        }],
    };
    wallet.apply_unconfirmed_txs([(funding, 0)]);

    let mut builder = wallet.build_tx();
    builder
        .drain_wallet()
        .drain_to(address.script_pubkey())
        .fee_rate(FeeRate::from_sat_per_vb_u32(2));
    let mut psbt = builder.finish().wrap_err("Failed to build the unrelated PSBT")?;

    if !util::sign_without_finalizing(&wallet, &mut psbt)? {
        bail!("Failed to sign the unrelated PSBT");
    }

    signed_frames(&psbt, request.min_parts)
}

/// The account a hardware wallet exports for the mnemonic, with key origin
fn watch_only_descriptor(request: WalletRequest) -> Result<Value> {
    let network = util::parse_network(&request.network)?;
    let mnemonic = Mnemonic::parse(&request.mnemonic).wrap_err("Invalid mnemonic")?;
    let wallet = util::bip84_signing_wallet(&mnemonic, network)?;

    Ok(json!({
        "external": wallet.public_descriptor(KeychainKind::External).to_string(),
        "internal": wallet.public_descriptor(KeychainKind::Internal).to_string(),
    }))
}

/// An account's bare SLIP-132 public key, as a hardware wallet shows it in a
/// plain QR and in a single-part BBQr, for a native segwit account the same
/// account as a single-frame crypto-hdkey UR, and the first receive address an
/// import of it should show
///
/// The address comes straight from the mnemonic's key at `/0/0`, not from a
/// descriptor, so it checks the address type the app infers from the key
fn public_key_export(request: PublicKeyRequest) -> Result<Value> {
    let network = util::parse_network(&request.network)?;
    let mnemonic = Mnemonic::parse(&request.mnemonic).wrap_err("Invalid mnemonic")?;
    let address_type = request.address_type;

    let secp = Secp256k1::new();
    let master = Xpriv::new_master(network, &mnemonic.to_seed(""))?;
    let coin_type = if network == Network::Bitcoin { 0 } else { 1 };
    let account_path: DerivationPath =
        format!("m/{}'/{coin_type}'/0'", address_type.purpose()).parse()?;
    let account = Xpub::from_priv(&secp, &master.derive_priv(&secp, &account_path)?);

    // the app reads a crypto-hdkey as a bare xpub, which imports as native segwit
    // whatever its origin says, so a wrapped segwit UR wouldn't match its address
    let ur = match address_type {
        AddressType::NativeSegwit => Some(single_frame_hdkey_ur(
            &account,
            &account_path,
            master.fingerprint(&secp),
            network,
        )?),
        AddressType::WrappedSegwit => None,
    };

    let mut encoded = account.encode();
    encoded[..4].copy_from_slice(&address_type.slip132_version(network.into()));
    let key = base58::encode_check(&encoded);

    let receive_path: DerivationPath = "m/0/0".parse()?;
    let first = account.derive_pub(&secp, &receive_path)?.to_pub();

    Ok(json!({
        "key": key,
        "bbqr": single_part_bbqr(&key)?,
        "ur": ur,
        "first_address": address_type.address(first, network).to_string(),
        "first_derivation_path": format!("{account_path}/0/0"),
    }))
}

/// An account key as one crypto-hdkey UR frame in the multi-part `1-1` form,
/// as Jade shows its export, with the origin and coin info it includes
///
/// The camera reports an unchanged code once, so a scanner that waits for a
/// second frame to finish this one never completes
fn single_frame_hdkey_ur(
    account: &Xpub,
    account_path: &DerivationPath,
    master_fingerprint: Fingerprint,
    network: Network,
) -> Result<String> {
    use cove_ur::{CryptoCoinInfo, CryptoHdkey, CryptoKeypath};
    use foundation_ur::Encoder;

    let components = account_path.into_iter().map(|child| u32::from(*child)).collect();
    let coin_network = if network == Network::Bitcoin { 0 } else { 1 };

    let mut hdkey = CryptoHdkey::from_xpub(account);
    hdkey.use_info = Some(CryptoCoinInfo::new(Some(0), Some(coin_network)));
    hdkey.origin = Some(CryptoKeypath::new(
        components,
        Some(master_fingerprint.to_bytes()),
        Some(account.depth.into()),
    ));

    let cbor = hdkey.encode().map_err(|error| eyre!("crypto-hdkey encoding failed: {error}"))?;

    // a fragment longer than the payload keeps it to one part
    let mut encoder = Encoder::new();
    encoder.start("crypto-hdkey", &cbor, cbor.len() + 1);
    if encoder.sequence_count() != 1 {
        bail!("crypto-hdkey UR took {} frames, expected one", encoder.sequence_count());
    }

    Ok(encoder.next_part().to_string())
}

/// A text payload as one BBQr frame, as a hardware wallet shows a short export
fn single_part_bbqr(text: &str) -> Result<String> {
    use bbqr::{
        encode::Encoding,
        file_type::FileType,
        qr::Version,
        split::{Split, SplitOptions},
    };

    let split = Split::try_from_data(
        text.as_bytes(),
        FileType::UnicodeText,
        SplitOptions {
            encoding: Encoding::Zlib,
            min_split_number: 1,
            max_split_number: 1,
            min_version: Version::V01,
            max_version: Version::V40,
        },
    )
    .map_err(|error| eyre!("BBQr encoding failed: {error}"))?;

    split.parts.into_iter().next().ok_or_else(|| eyre!("BBQr encoding produced no frame"))
}

/// Reads the simulator's pasteboard from the host, so the XCUITest runner
/// doesn't trigger the paste permission prompt by reading it itself
fn simulator_pasteboard(request: PasteboardRequest) -> Result<Value> {
    let output = Command::new("xcrun")
        .args(["simctl", "pbpaste", &request.udid])
        .output()
        .wrap_err("Failed to run simctl pbpaste")?;

    if !output.status.success() {
        bail!("simctl pbpaste failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(json!({ "text": String::from_utf8(output.stdout).wrap_err("The pasteboard is not UTF-8")? }))
}

/// Switches an iOS simulator between light and dark mode, which an XCUITest
/// can't do from inside the simulator
fn simulator_appearance(request: AppearanceRequest) -> Result<Value> {
    let appearance = match request.appearance {
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    };

    simctl_ui(&request.udid, &["appearance", appearance])?;
    Ok(json!({ "ok": true }))
}

/// Reads an iOS simulator's text size and, when the request names one, sets
/// it, which reaches a running app as a live
/// `UIContentSizeCategory.didChangeNotification`, unlike the launch argument
/// a test sets per launch. Answers with the size the simulator had before
fn simulator_content_size(request: ContentSizeRequest) -> Result<Value> {
    let printed = simctl_ui(&request.udid, &["content_size"])?;
    let printed = printed.trim();

    // simctl prints `unknown` or `unsupported` when it can't tell, and a test could then only
    // restore a guess, so refuse before changing anything
    let previous = serde_json::from_value::<ContentSize>(json!(printed))
        .map_err(|_| eyre!("simctl ui content_size reported `{printed}`, not a text size"))?;

    if let Some(size) = request.content_size {
        simctl_ui(&request.udid, &["content_size", size.name()])?;
    }

    Ok(json!({ "previous": previous.name() }))
}

/// Enrolls or removes the simulator's Face ID, or answers the Face ID prompt on screen, which an
/// XCUITest can't do from inside the simulator
///
/// These are the Darwin notifications the Simulator app's Features > Face ID menu posts, which the
/// simulator's BiometricKit listens for
fn simulator_biometrics(request: BiometricsRequest) -> Result<Value> {
    const ENROLLMENT: &str = "com.apple.BiometricKit.enrollmentChanged";

    let udid = request.udid.as_str();
    match request.action {
        BiometricsAction::Enroll => {
            simctl_notifyutil(udid, &["-s", ENROLLMENT, "1"])?;
            simctl_notifyutil(udid, &["-p", ENROLLMENT])?;
        }

        BiometricsAction::Unenroll => {
            simctl_notifyutil(udid, &["-s", ENROLLMENT, "0"])?;
            simctl_notifyutil(udid, &["-p", ENROLLMENT])?;
        }

        BiometricsAction::Match => {
            simctl_notifyutil(udid, &["-p", "com.apple.BiometricKit_Sim.pearl.match"])?;
        }

        BiometricsAction::Nomatch => {
            simctl_notifyutil(udid, &["-p", "com.apple.BiometricKit_Sim.pearl.nomatch"])?;
        }
    }

    Ok(json!({ "ok": true }))
}

/// Runs `notifyutil` inside the simulator
fn simctl_notifyutil(udid: &str, args: &[&str]) -> Result<()> {
    let output = Command::new("xcrun")
        .args(["simctl", "spawn", udid, "notifyutil"])
        .args(args)
        .output()
        .wrap_err("Failed to run simctl spawn notifyutil")?;

    if !output.status.success() {
        bail!(
            "notifyutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

/// Runs `simctl ui <udid> <args>` and returns what it printed
fn simctl_ui(udid: &str, args: &[&str]) -> Result<String> {
    let option = args.first().copied().unwrap_or_default();
    let output = Command::new("xcrun")
        .args(["simctl", "ui", udid])
        .args(args)
        .output()
        .wrap_err_with(|| format!("Failed to run simctl ui {option}"))?;

    if !output.status.success() {
        bail!("simctl ui {option} failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    String::from_utf8(output.stdout)
        .wrap_err_with(|| format!("simctl ui {option} printed non-UTF-8"))
}

fn signed_frames(psbt: &Psbt, min_parts: usize) -> Result<Value> {
    let signed = psbt.serialize();
    Ok(json!({
        "psbt": STANDARD.encode(&signed),
        "ur": util::ur_parts(&signed, min_parts)?,
        "bbqr": util::bbqr_parts(&signed, min_parts)?,
    }))
}

fn read_http_request(reader: &mut impl BufRead) -> Result<(String, String, Vec<u8>)> {
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or_else(|| eyre!("Empty HTTP request"))?.to_string();
    let path = parts.next().ok_or_else(|| eyre!("HTTP request has no path"))?.to_string();

    let mut content_length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;

        let header = header.trim_end();
        if header.is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().wrap_err("Invalid Content-Length")?;
            }
        }
    }

    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;

    Ok((method, path, body))
}

fn write_http_response(mut stream: TcpStream, status: u16, body: &Value) -> Result<()> {
    let body = body.to_string();
    let reason = if status == 200 { "OK" } else { "Bad Request" };

    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;

    Ok(stream.flush()?)
}

// ----------------------------------------------------------------------------
// TAPSIGNER bridge
// ----------------------------------------------------------------------------

/// One app NFC session: APDU frames in, card responses out, until the app closes it
fn serve_bridge(mut stream: TcpStream, state: &SignerState) -> Result<()> {
    let drop_after = state.drop_after.lock().take();
    let mut exchanges = 0;

    while let Some(apdu) = read_frame(&mut stream)? {
        if drop_after == Some(exchanges) {
            print_info(&format!(
                "TAPSIGNER bridge: dropping the session after {exchanges} exchanges"
            ));
            stream.shutdown(Shutdown::Both)?;
            return Ok(());
        }

        let response = exchange(state, &apdu)?;
        write_frame(&mut stream, &response)?;
        exchanges += 1;
    }

    Ok(())
}

/// Send one APDU to the emulator and return the response with its status word
fn exchange(state: &SignerState, apdu: &[u8]) -> Result<Vec<u8>> {
    let request = if apdu == SELECT_APDU {
        STATUS_REQUEST
    } else if apdu.starts_with(CBOR_APDU_HEADER)
        && apdu.len() > 5
        && usize::from(apdu[4]) == apdu.len() - 5
    {
        &apdu[5..]
    } else {
        return Ok(INSTRUCTION_NOT_SUPPORTED.to_vec());
    };

    // ecard.py serves one connection at a time, so its commands run one after another, as one
    // card's would
    let mut response = state.cards.exchange(request)?;
    response.extend_from_slice(&STATUS_OK);
    Ok(response)
}

fn read_frame(stream: &mut TcpStream) -> Result<Option<Vec<u8>>> {
    let mut length = [0; 4];
    match stream.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }

    let mut frame = vec![0; u32::from_be_bytes(length) as usize];
    stream.read_exact(&mut frame)?;

    Ok(Some(frame))
}

fn write_frame(stream: &mut TcpStream, frame: &[u8]) -> Result<()> {
    let length = u32::try_from(frame.len()).wrap_err("Frame is too large")?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(frame)?;

    Ok(stream.flush()?)
}

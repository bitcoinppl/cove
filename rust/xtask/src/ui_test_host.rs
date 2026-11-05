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
//!   `/public-key-export` returns an account's bare SLIP-132 public key, as a
//!   plain QR and as a single-part BBQr, with the first receive address it
//!   should import as. `/simulator-pasteboard` reads an iOS simulator's
//!   pasteboard and `/simulator-appearance` switches it between light and
//!   dark mode. The `/tapsigner/...` routes start the card emulator, set up or
//!   factory fresh, and inject dropped sessions. The `/keyteleport/...` routes
//!   play the other phone in a KeyTeleport transfer ([`key_teleport`])
//! - [`UiTestPorts::tapsigner_bridge`]: the TAPSIGNER bridge. Each APDU arrives as a frame with a
//!   4-byte big-endian length, goes to Coinkite's `ecard.py` emulator, and the
//!   card's response goes back the same way. Closing the connection is how a
//!   dropped NFC session looks to the app

use std::{
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use bdk_wallet::KeychainKind;
use bip39::Mnemonic;
use bitcoin::{
    absolute::LockTime,
    base58,
    bip32::{DerivationPath, Xpriv, Xpub},
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
use xshell::Shell;

mod key_teleport;

use crate::{
    android_device::AndroidDevice,
    common::{log_tail, print_info, print_success, print_warning},
    fake_node::{AndroidPorts, AndroidTarget, FakeNodeGuard, ReversedPorts},
    shutdown::{self, Registration, Stop},
    tapsigner_emulator::{emulator_script, EmulatorPaths},
    ui_test_ports::{PortReservation, UiTestPorts},
    util,
};

/// How long stopping the signer waits for an exchange with the card emulator to finish
const EMULATOR_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// The emulator derives every key from this seed, so the card and its wallet
/// are the same on every run
const EMULATOR_RNG_SEED: &str = "1337";

// ecard.py starts in well under a second, but on a busy macOS runner a start that overlaps a
// simulator boot and app install has spent more than 20 seconds in its first imports. This stays
// under the iOS tests' 60 second request timeout, so a start that still fails reports its log
const EMULATOR_START_TIMEOUT: Duration = Duration::from_secs(45);

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

        // one log per port block, so concurrent runs from one worktree keep their own
        let state = Arc::new(SignerState {
            emulator_paths: prepare_emulator(),
            emulator_log: host_dir()?.join(format!("ecard-{}.log", ports.base())),
            emulator: Mutex::new(None),
            drop_after: Mutex::new(None),
            key_teleport_receivers: key_teleport::Receivers::default(),
        });

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

        // bounded, since an exchange with a card that stopped replying holds the lock; after a
        // signal, the card then stops with the run's other commands
        match self.state.emulator.try_lock_for(EMULATOR_LOCK_TIMEOUT) {
            Some(mut emulator) => {
                if let Some(mut emulator) = emulator.take() {
                    emulator.stop();
                }
            }
            None => print_warning("The card emulator is stuck in an exchange and was not stopped"),
        }
    }
}

impl Drop for HardwareSignerGuard {
    fn drop(&mut self) {
        self.signer.stop();
    }
}

struct SignerState {
    // the error is a string because each /tapsigner/start reports it again
    emulator_paths: Result<EmulatorPaths, String>,
    emulator_log: PathBuf,
    emulator: Mutex<Option<CardEmulator>>,
    // the next bridge session closes after this many exchanges
    drop_after: Mutex<Option<u32>>,
    key_teleport_receivers: key_teleport::Receivers,
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
        ("POST", "/watch-only-descriptor") => watch_only_descriptor(serde_json::from_slice(body)?),
        ("POST", "/public-key-export") => public_key_export(serde_json::from_slice(body)?),
        ("POST", "/simulator-pasteboard") => simulator_pasteboard(serde_json::from_slice(body)?),
        ("POST", "/simulator-appearance") => simulator_appearance(serde_json::from_slice(body)?),
        ("POST", "/tapsigner/start") => {
            let request: TapSignerStartRequest = if body.is_empty() {
                TapSignerStartRequest::default()
            } else {
                serde_json::from_slice(body)?
            };

            let mut emulator = state.emulator.lock();
            if let Some(mut old) = emulator.take() {
                old.stop();
            }

            let paths = state.emulator_paths.as_ref().map_err(|error| {
                eyre!("The TAPSIGNER emulator could not be prepared when the host started: {error}")
            })?;

            *state.drop_after.lock() = None;
            *emulator = Some(CardEmulator::start(paths, &state.emulator_log, request.needs_setup)?);
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
/// plain QR and in a single-part BBQr, and the first receive address an import
/// of it should show
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

    let mut encoded = account.encode();
    encoded[..4].copy_from_slice(&address_type.slip132_version(network.into()));
    let key = base58::encode_check(&encoded);

    let receive_path: DerivationPath = "m/0/0".parse()?;
    let first = account.derive_pub(&secp, &receive_path)?.to_pub();

    Ok(json!({
        "key": key,
        "bbqr": single_part_bbqr(&key)?,
        "first_address": address_type.address(first, network).to_string(),
        "first_derivation_path": format!("{account_path}/0/0"),
    }))
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

    let output = Command::new("xcrun")
        .args(["simctl", "ui", &request.udid, "appearance", appearance])
        .output()
        .wrap_err("Failed to run simctl ui appearance")?;

    if !output.status.success() {
        bail!("simctl ui appearance failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(json!({ "ok": true }))
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

    // holding the lock serializes card commands, as one card would
    let emulator = state.emulator.lock();
    let emulator = emulator
        .as_ref()
        .ok_or_else(|| eyre!("The test sent a TAPSIGNER command before POST /tapsigner/start"))?;

    let mut card = UnixStream::connect(&emulator.socket).wrap_err_with(|| {
        format!("Failed to reach the emulator at {}", emulator.socket.display())
    })?;
    card.write_all(request)?;

    // the emulator answers each request with one CBOR value and closes the
    // connection once our side is done writing, so the response ends at EOF
    card.shutdown(Shutdown::Write)?;
    let mut response = Vec::new();
    card.read_to_end(&mut response)?;

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

// ----------------------------------------------------------------------------
// Card emulator
// ----------------------------------------------------------------------------

/// A running `ecard.py` TAPSIGNER with factory CVC 123456, set up with a key
/// unless it was started factory fresh
struct CardEmulator {
    child: Child,
    socket: PathBuf,
}

impl CardEmulator {
    fn start(paths: &EmulatorPaths, log_path: &Path, needs_setup: bool) -> Result<Self> {
        // Unix socket paths are limited to about 100 bytes, which rules out the
        // per-user temporary directory on macOS
        let socket = PathBuf::from(format!("/tmp/cove-ecard-{}.sock", std::process::id()));
        if socket.exists() {
            fs::remove_file(&socket)?;
        }

        let log = File::create(log_path)?;
        // --no-init leaves the card's slot without a key, as a new card ships
        let mut command = Command::new(&paths.python);
        command.arg(&paths.script).args([
            "--rng-seed",
            EMULATOR_RNG_SEED,
            "emulate",
            "--tapsigner",
        ]);

        if needs_setup {
            command.arg("--no-init");
        }

        let child = command
            .arg("--pipe")
            .arg(&socket)
            .env("PYTHONUNBUFFERED", "1")
            // a SIGABRT then writes every thread's Python stack to the log
            .env("PYTHONFAULTHANDLER", "1")
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .wrap_err_with(|| format!("Failed to start {}", paths.script.display()))?;

        let mut emulator = Self { child, socket };
        let started = Instant::now();
        let deadline = started + EMULATOR_START_TIMEOUT;
        while !emulator.socket.exists() {
            if let Some(status) = emulator.child.try_wait()? {
                bail!("ecard.py exited with {status}; {}", log_tail(log_path));
            }

            if Instant::now() > deadline {
                emulator.abort_with_stacks();
                bail!(
                    "ecard.py did not listen within {EMULATOR_START_TIMEOUT:?}; {}",
                    log_tail(log_path)
                );
            }

            thread::sleep(Duration::from_millis(100));
        }

        print_info(&format!(
            "TAPSIGNER emulator listening on {} after {:.1?}",
            emulator.socket.display(),
            started.elapsed()
        ));

        Ok(emulator)
    }

    /// Stops an emulator that hung, after asking Python to log where each thread was
    fn abort_with_stacks(&mut self) {
        let pid = self.child.id().to_string();
        if Command::new("kill").args(["-ABRT", &pid]).status().is_ok_and(|status| status.success())
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
                thread::sleep(Duration::from_millis(50));
            }
        }

        self.stop();
    }

    fn stop(&mut self) {
        if let Err(error) = self.child.kill().and_then(|()| self.child.wait().map(drop)) {
            print_warning(&format!("Failed to stop ecard.py: {error}"));
        }

        let _ = fs::remove_file(&self.socket);
    }
}

/// Prepares the card emulator's Python and script before any test asks for a card
///
/// Building the venv takes most of a minute on a fresh CI runner, longer than the UI tests wait
/// for `/tapsigner/start`. A failure only fails the TAPSIGNER tests, so the other UI tests still
/// run on a host without uv
fn prepare_emulator() -> Result<EmulatorPaths, String> {
    print_info("Preparing the TAPSIGNER emulator");

    emulator_paths().map_err(|error| {
        let error = format!("{error:#}");
        print_warning(&format!("TAPSIGNER emulator unavailable, its tests will fail: {error}"));
        error
    })
}

/// `COVE_ECARD_PYTHON` with the pinned `ecard.py`, or the shared uv venv
fn emulator_paths() -> Result<EmulatorPaths> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    let sh = Shell::new()?;
    sh.change_dir(workspace);

    match env::var_os("COVE_ECARD_PYTHON") {
        Some(python) => Ok(EmulatorPaths { python: python.into(), script: emulator_script(&sh)? }),
        None => EmulatorPaths::prepare(&sh),
    }
}

/// A directory under the cargo target dir for the emulator's log
fn host_dir() -> Result<PathBuf> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    let dir = workspace.join("target").join("ui-test-host");
    fs::create_dir_all(&dir)?;

    Ok(dir)
}

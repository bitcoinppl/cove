//! Coinkite's TAPSIGNER card emulator behind Cove's NFC transport callback
//!
//! `just test-tapsigner-emulator` installs the emulator's Python dependencies and points
//! [`PYTHON_ENV`] and [`SCRIPT_ENV`] at the interpreter and at the `ecard.py` that ships in
//! the pinned rust-cktap checkout. Each [`EmulatedCard`] is its own emulator process with
//! fresh card state, so tests never see each other's setup or PIN changes

use std::fmt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_cktap::{CkTapCard, CkTransport, emulator::find_emulator};

use crate::tap_card::{TapcardTransportProtocol, TransportError};

/// Interpreter with the emulator's Python dependencies installed
pub(crate) const PYTHON_ENV: &str = "COVE_TAPSIGNER_EMULATOR_PYTHON";

/// Path to `coinkite-tap-proto/emulator/ecard.py`
pub(crate) const SCRIPT_ENV: &str = "COVE_TAPSIGNER_EMULATOR_SCRIPT";

/// The CVC printed on every emulated card
pub(crate) const FACTORY_CVC: &str = "123456";

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// A running emulator process for one factory-fresh TAPSIGNER
pub(crate) struct EmulatedCard {
    child: Child,
    pipe: PathBuf,
    log: PathBuf,
    // removed after the child is gone, so the socket and log outlive the process
    _dir: tempfile::TempDir,
}

impl EmulatedCard {
    /// Starts a TAPSIGNER with the factory CVC whose key slot is not set up yet, as cards ship
    ///
    /// The seed fixes the card's keys and its unlucky-number errors, so a test that imports two
    /// cards must give them different seeds or the second import is a duplicate wallet
    pub(crate) fn start_unset(rng_seed: u32) -> Self {
        let python = required_env(PYTHON_ENV);
        let script = PathBuf::from(required_env(SCRIPT_ENV));
        assert!(
            script.is_file(),
            "{SCRIPT_ENV} points at {}, which does not exist; run `just test-tapsigner-emulator`",
            script.display()
        );

        // macOS limits unix socket paths to 104 bytes, which the per-user TMPDIR can exceed
        let dir = tempfile::Builder::new()
            .prefix("cove-ecard")
            .tempdir_in("/tmp")
            .expect("emulator temp dir creates");

        let pipe = dir.path().join("card.sock");
        let log = dir.path().join("ecard.log");
        let log_file = std::fs::File::create(&log).expect("emulator log creates");

        let child = Command::new(&python)
            .arg("-I")
            .arg(&script)
            .args(["--quiet", "--rng-seed", &rng_seed.to_string()])
            .args(["emulate", "--tapsigner", "--no-init"])
            .arg("--pipe")
            .arg(&pipe)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone().expect("emulator log clones"))
            .stderr(log_file)
            .spawn()
            .unwrap_or_else(|error| {
                panic!("failed to start the TAPSIGNER emulator with {python}: {error}")
            });

        let mut card = Self { child, pipe, log, _dir: dir };
        card.wait_for_socket();
        card
    }

    fn wait_for_socket(&mut self) {
        let started = Instant::now();
        let timeout = crate::test_support::loaded_host_wait(STARTUP_TIMEOUT);

        while !self.pipe.exists() {
            if let Some(status) = self.child.try_wait().expect("emulator status reads") {
                panic!("TAPSIGNER emulator exited with {status}:\n{}", self.log_contents());
            }

            assert!(
                started.elapsed() < timeout,
                "TAPSIGNER emulator did not open {} within {timeout:?}:\n{}",
                self.pipe.display(),
                self.log_contents()
            );

            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn log_contents(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Opens a new card session, as one NFC tap does
    ///
    /// The emulator serves one connection at a time, so every handle from an earlier tap
    /// must be dropped first or this session's commands wait forever
    pub(crate) async fn tap(&self) -> rust_cktap::TapSigner {
        match find_emulator(&self.pipe).await {
            Ok(CkTapCard::TapSigner(card)) => card,
            Ok(_) => panic!("the emulator was started as a TAPSIGNER"),
            Err(error) => panic!("emulator status failed: {error}\n{}", self.log_contents()),
        }
    }

    /// A new session wrapped as the callback the iOS and Android NFC layers implement
    pub(crate) async fn nfc_transport(&self) -> Box<dyn TapcardTransportProtocol> {
        Box::new(EmulatorTransport(self.tap().await.transport))
    }
}

impl Drop for EmulatedCard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!(
            "{name} is not set. The TAPSIGNER emulator tests need Coinkite's card emulator; \
             run them with `just test-tapsigner-emulator`, which installs it and sets \
             {PYTHON_ENV} and {SCRIPT_ENV}"
        )
    })
}

/// Forwards APDUs to the emulator's socket the way the platform NFC layers forward them
/// to the card
pub(crate) struct EmulatorTransport(Arc<dyn CkTransport>);

impl fmt::Debug for EmulatorTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EmulatorTransport")
    }
}

#[async_trait::async_trait]
impl TapcardTransportProtocol for EmulatorTransport {
    fn set_message(&self, _message: String) {}

    fn append_message(&self, _message: String) {}

    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TransportError> {
        self.0.transmit_apdu(command_apdu).await.map_err(TransportError::from)
    }
}

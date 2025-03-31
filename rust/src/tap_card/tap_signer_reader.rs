use std::hash::Hasher;
use std::sync::Arc;

use bitcoin::bip32::Fingerprint;
use bitcoin::hashes::HashEngine as _;
use bitcoin::secp256k1;
use nid::Nanoid;
use parking_lot::Mutex as SyncMutex;
use parking_lot::RwLock;
use rust_cktap::apdu::DeriveResponse;
use rust_cktap::commands::Authentication;
use rust_cktap::commands::Wait as _;
use rust_cktap::tap_signer::TapSignerError;
use rust_cktap::{CkTapCard, commands::CkTransport as _};
use tokio::sync::Mutex;
use tracing::debug;

use crate::database::Database;
use crate::network::Network;

use super::CkTapError;
use super::{TapcardTransport, TapcardTransportProtocol, TransportError};

#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TapSignerReaderError {
    #[error(transparent)]
    TapSignerError(#[from] TransportError),

    #[error("UnknownCardType: {0}, expected TapSigner")]
    UnknownCardType(String),

    #[error("No command")]
    NoCommand,

    #[error("Invalid pin length, must be betweeen 6 and 32, found {0}")]
    InvalidPinLength(u8),

    #[error("PIN must be numeric only, found {0}")]
    NonNumericPin(String),

    #[error("Setup is already complete")]
    SetupAlreadyComplete,

    #[error("Invalid chain code length, must be 32, found {0}")]
    InvalidChainCodeLength(u32),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = TapSignerReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

// Main interface exposed to Swift
#[derive(Debug, uniffi::Object)]
pub struct TapSignerReader {
    id: String,
    reader: Mutex<rust_cktap::TapSigner<TapcardTransport>>,
    cmd: RwLock<Option<TapSignerCmd>>,
    transport: TapcardTransport,

    /// Last response from the setup process, has started, if the last response is `Complete` then the setup process is complete
    last_response: SyncMutex<Option<Arc<SetupCmdResponse>>>,

    network: Network,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    Setup(Arc<SetupCmd>),
    Derive { pin: String },
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Object)]
pub struct SetupCmd {
    pub factory_pin: String,
    pub new_pin: String,
    pub chain_code: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum, derive_more::From)]
pub enum TapSignerResponse {
    Setup(SetupCmdResponse),
    Import(DeriveInfo),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum SetupCmdResponse {
    ContinueFromInit(ContinueFromInit),
    ContinueFromBackup(ContinueFromBackup),
    ContinueFromDerive(ContinueFromDerive),
    Complete(TapSignerSetupComplete),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromInit {
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromBackup {
    pub backup: Vec<u8>,
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromDerive {
    pub backup: Vec<u8>,
    pub derive_info: DeriveInfo,
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct TapSignerSetupComplete {
    pub backup: Vec<u8>,
    pub derive_info: DeriveInfo,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct DeriveInfo {
    pub master_pubkey: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub chain_code: Vec<u8>,
    pub path: Vec<u32>,
    pub network: Network,
}

#[uniffi::export]
impl TapSignerReader {
    #[uniffi::constructor(name = "new", default(cmd = None))]
    pub async fn new(
        transport: Box<dyn TapcardTransportProtocol>,
        cmd: Option<TapSignerCmd>,
    ) -> Result<Self> {
        let transport = TapcardTransport(Arc::new(transport));
        let card = transport
            .clone()
            .to_cktap()
            .await
            .map_err(TransportError::from)?;

        debug!("tap_card_from_status: {:?}", card);

        let card = match card {
            CkTapCard::TapSigner(card) => Ok(card),
            CkTapCard::SatsCard(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsCard".to_string(),
            )),
            CkTapCard::SatsChip(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsChip".to_string(),
            )),
        }?;

        let id: Nanoid = Nanoid::new();
        let network = Database::global().global_config.selected_network();

        let me = Self {
            id: id.to_string(),
            reader: Mutex::new(card),
            transport,
            cmd: RwLock::new(cmd),
            last_response: SyncMutex::new(None),
            network,
        };

        // if the card has a required auth delay, wait for it
        me.wait_if_needed().await?;

        Ok(me)
    }

    #[uniffi::method]
    pub async fn run(&self) -> Result<TapSignerResponse> {
        let cmd = self
            .cmd
            .write()
            .take()
            .ok_or(TapSignerReaderError::NoCommand)?;

        match cmd {
            TapSignerCmd::Setup(cmd) => {
                let response = self.setup(cmd).await?;
                Ok(TapSignerResponse::Setup(response))
            }

            TapSignerCmd::Derive { pin } => {
                let response = self.derive(&pin).await?;
                Ok(TapSignerResponse::Import(response))
            }
        }
    }

    /// Start the setup process
    pub async fn setup(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        let new_pin = cmd.new_pin.as_bytes();
        if new_pin.len() < 6 || new_pin.len() > 32 {
            return Err(TapSignerReaderError::InvalidPinLength(new_pin.len() as u8));
        }

        if !cmd.new_pin.trim().chars().all(char::is_numeric) {
            return Err(TapSignerReaderError::NonNumericPin(cmd.new_pin.to_string()));
        }

        self.init_backup_change(cmd).await
    }

    /// User started the setup process, but errored out before completing the setup, we can continue from the last step
    pub async fn continue_setup(
        &self,
        response: SetupCmdResponse,
    ) -> Result<SetupCmdResponse, Error> {
        match response {
            SetupCmdResponse::ContinueFromInit(c) => self.init_backup_change(c.continue_cmd).await,

            SetupCmdResponse::ContinueFromBackup(c) => {
                let response = self.derive_and_change(c.continue_cmd, c.backup).await;
                Ok(response)
            }

            SetupCmdResponse::ContinueFromDerive(c) => {
                let response = self.change(c.continue_cmd, c.backup, c.derive_info).await;
                Ok(response)
            }

            // already complete, just return the backup
            SetupCmdResponse::Complete(c) => Ok(SetupCmdResponse::Complete(c)),
        }
    }

    /// Get the last response from the reader
    pub fn last_response(&self) -> Option<TapSignerResponse> {
        let response = self.last_response.lock().clone()?;
        let response = Arc::unwrap_or_clone(response);
        let tap_signer_response = TapSignerResponse::Setup(response);
        Some(tap_signer_response)
    }
}

impl TapSignerReader {
    async fn wait_if_needed(&self) -> Result<(), Error> {
        let mut auth_delay = self.reader.lock().await.auth_delay;

        while let Some(delay) = auth_delay {
            let message = format!("Too many PIN attempts, waiting for {} seconds...", delay);

            self.reader
                .lock()
                .await
                .wait(None)
                .await
                .map_err(TransportError::from)?;

            self.transport.set_message(message);
            auth_delay = self.reader.lock().await.auth_delay;
        }

        self.reader.lock().await.set_auth_delay(None);
        Ok(())
    }

    async fn init_backup_change(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        let _init_response = self
            .reader
            .lock()
            .await
            .init(cmd.chain_code, &cmd.factory_pin)
            .await
            .map_err(TransportError::from)?;

        Ok(self.backup_change_xpub(cmd).await)
    }

    async fn backup_change_xpub(&self, cmd: Arc<SetupCmd>) -> SetupCmdResponse {
        let backup_response = self.reader.lock().await.backup(&cmd.factory_pin).await;

        let backup = match backup_response {
            Ok(backup) => backup.data,
            Err(e) => {
                let error = TapSignerReaderError::TapSignerError(e.into());
                let response = SetupCmdResponse::ContinueFromInit(ContinueFromInit {
                    continue_cmd: cmd,
                    error,
                });

                *self.last_response.lock() = Some(response.clone().into());

                return response;
            }
        };

        self.derive_and_change(cmd.clone(), backup).await
    }

    async fn derive_and_change(&self, cmd: Arc<SetupCmd>, backup: Vec<u8>) -> SetupCmdResponse {
        let derive_info = match self.derive(&cmd.factory_pin).await {
            Ok(derive) => derive,
            Err(error) => {
                let response = SetupCmdResponse::ContinueFromBackup(ContinueFromBackup {
                    backup,
                    continue_cmd: cmd,
                    error,
                });

                *self.last_response.lock() = Some(response.clone().into());
                return response;
            }
        };

        self.change(cmd, backup, derive_info).await
    }

    async fn change(
        &self,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
    ) -> SetupCmdResponse {
        let change_response = self
            .reader
            .lock()
            .await
            .change(&cmd.new_pin, &cmd.factory_pin)
            .await;

        if let Err(e) = change_response {
            let error = TapSignerReaderError::TapSignerError(e.into());
            let response = SetupCmdResponse::ContinueFromDerive(ContinueFromDerive {
                backup,
                derive_info,
                continue_cmd: cmd,
                error,
            });

            *self.last_response.lock() = Some(response.clone().into());
            return response;
        }

        let complete = TapSignerSetupComplete {
            backup,
            derive_info,
        };

        *self.last_response.lock() = Some(SetupCmdResponse::Complete(complete.clone()).into());
        SetupCmdResponse::Complete(complete)
    }

    async fn derive(&self, pin: &str) -> Result<DeriveInfo, Error> {
        debug!("starting derive");

        let path: [u32; 3] = match self.network {
            Network::Bitcoin => [84, 0, 0],
            _ => [84, 1, 0],
        };

        let derive_response = self.reader.lock().await.derive(&path, pin).await?;
        let derive_info = DeriveInfo::from_response(derive_response, path.to_vec(), self.network);

        Ok(derive_info)
    }
}

#[uniffi::export]
impl SetupCmd {
    #[uniffi::constructor(default(chain_code = None))]
    pub fn try_new(
        factory_pin: String,
        new_pin: String,
        chain_code: Option<Vec<u8>>,
    ) -> Result<Self, Error> {
        let chain_code = match chain_code {
            Some(chain_code) => {
                let chain_code_len = chain_code.len() as u32;
                chain_code
                    .try_into()
                    .map_err(|_| Error::InvalidChainCodeLength(chain_code_len))?
            }
            None => rust_cktap::rand_chaincode(&mut secp256k1::rand::thread_rng()),
        };

        Ok(Self {
            factory_pin,
            new_pin,
            chain_code,
        })
    }
}

impl std::hash::Hash for TapSignerReader {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.cmd.read().as_ref().hash(state);
        self.last_response.lock().as_ref().hash(state);
    }
}

impl Eq for TapSignerReader {}
impl PartialEq for TapSignerReader {
    fn eq(&self, other: &Self) -> bool {
        let response_lock = self.last_response.lock();
        let other_response_lock = other.last_response.lock();

        self.id == other.id
            && self.cmd.read().as_ref() == other.cmd.read().as_ref()
            && response_lock.as_ref() == other_response_lock.as_ref()
    }
}

impl DeriveInfo {
    pub fn from_response(
        derive_response: DeriveResponse,
        path: Vec<u32>,
        network: Network,
    ) -> Self {
        let master_xpub = derive_response.master_pubkey;
        let chain_code = derive_response.chain_code;
        let xpub = derive_response
            .pubkey
            .expect("has pubkey because path was given");

        Self {
            master_pubkey: master_xpub,
            pubkey: xpub,
            chain_code,
            path,
            network,
        }
    }

    pub fn master_fingerprint(&self) -> Fingerprint {
        use bitcoin::hashes::{Hash as _, ripemd160, sha256};

        let mut sha_engine = sha256::Hash::engine();
        sha_engine.input(self.master_pubkey.as_ref());
        let sha_result = sha256::Hash::from_engine(sha_engine);

        let mut ripemd_engine = ripemd160::Hash::engine();
        ripemd_engine.input(sha_result.as_ref());
        let hash160_result = ripemd160::Hash::from_engine(ripemd_engine);

        let mut fingerprint = [0u8; 4];
        fingerprint.copy_from_slice(&hash160_result[0..4]);

        Fingerprint::from(fingerprint)
    }
}

impl TapSignerResponse {
    pub fn setup_response(&self) -> Option<&SetupCmdResponse> {
        match self {
            TapSignerResponse::Setup(response) => Some(response),
            _ => None,
        }
    }

    pub fn derive_response(&self) -> Option<&DeriveInfo> {
        match self {
            TapSignerResponse::Import(response) => Some(response),
            _ => None,
        }
    }
}

impl From<TapSignerError> for TapSignerReaderError {
    fn from(error: TapSignerError) -> Self {
        TapSignerReaderError::TapSignerError(error.into())
    }
}

impl TapSignerReaderError {
    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            TapSignerReaderError::TapSignerError(TransportError::CkTap(CkTapError::BadAuth))
        )
    }
}

mod ffi {
    use crate::util::generate_random_chain_code;

    use super::*;

    fn derive_info() -> DeriveInfo {
        use std::str::FromStr as _;
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let original_xpub = bitcoin::bip32::Xpub::from_str(xpub).unwrap();

        let master_xpub = "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb";
        let master_xpub = bitcoin::bip32::Xpub::from_str(master_xpub).unwrap();

        let master_xpub_bytes = master_xpub.public_key.serialize();
        let xpub_bytes = original_xpub.public_key.serialize();

        DeriveInfo {
            network: Network::Bitcoin,
            master_pubkey: master_xpub_bytes.to_vec(),
            pubkey: xpub_bytes.to_vec(),
            chain_code: original_xpub.chain_code.to_bytes().to_vec(),
            path: vec![84, 1, 0],
        }
    }

    #[uniffi::export]
    fn tap_signer_response_setup_response(response: TapSignerResponse) -> Option<SetupCmdResponse> {
        response.setup_response().cloned()
    }

    #[uniffi::export]
    fn tap_signer_response_derive_response(response: TapSignerResponse) -> Option<DeriveInfo> {
        response.derive_response().cloned()
    }

    #[uniffi::export]
    fn display_tap_signer_reader_error(error: TapSignerReaderError) -> String {
        error.to_string()
    }

    #[uniffi::export]
    fn tap_signer_setup_retry_continue_cmd(preview: bool) -> SetupCmdResponse {
        assert!(preview);

        let backup = vec![0u8; 32];
        let setup_cmd = SetupCmd {
            factory_pin: "123456".to_string(),
            new_pin: "000000".to_string(),
            chain_code: generate_random_chain_code(),
        };

        SetupCmdResponse::ContinueFromDerive(ContinueFromDerive {
            backup,
            derive_info: derive_info(),
            continue_cmd: Arc::new(setup_cmd),
            error: TapSignerReaderError::NoCommand,
        })
    }

    #[uniffi::export]
    fn tap_signer_error_is_auth_error(error: TapSignerReaderError) -> bool {
        error.is_auth_error()
    }

    // MARK: - FFI PREVIEW
    #[uniffi::export]
    fn tap_signer_setup_complete_new(preview: bool) -> TapSignerSetupComplete {
        assert!(preview);

        let backup = vec![0u8; 32];
        TapSignerSetupComplete {
            backup,
            derive_info: derive_info(),
        }
    }
}

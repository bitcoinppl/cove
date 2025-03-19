use std::sync::Arc;

use bitcoin::secp256k1;
use parking_lot::RwLock;
use rust_cktap::{CkTapCard, commands::CkTransport as _};
use tokio::sync::Mutex;

use super::{TapcardTransport, TapcardTransportProtocol, TransportError};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
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

    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = TapSignerReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

// Main interface exposed to Swift
#[derive(Debug, uniffi::Object)]
pub struct TapSignerReader {
    reader: Mutex<rust_cktap::TapSigner<TapcardTransport>>,
    cmd: RwLock<Option<TapSignerCmd>>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    Setup(Arc<SetupCmd>),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct SetupCmd {
    pub factory_pin: String,
    pub new_pin: String,
    pub chain_code: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum, derive_more::From)]
pub enum TapSignerResponse {
    Setup(SetupCmdResponse),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SetupCmdResponse {
    Complete { backup: Vec<u8> },
    ContinueFromInit(ContinueFromInit),
    ContinueFromBackup(ContinueFromBackup),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromInit {
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromBackup {
    pub backup: Vec<u8>,
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[uniffi::export]
impl TapSignerReader {
    #[uniffi::constructor(name = "new", default(cmd = None))]
    pub async fn new(
        transport: Box<dyn TapcardTransportProtocol>,
        cmd: Option<TapSignerCmd>,
    ) -> Result<Self> {
        let transport = TapcardTransport(transport);
        let card = transport.to_cktap().await.map_err(TransportError::from)?;

        let card = match card {
            CkTapCard::TapSigner(card) => Ok(card),
            CkTapCard::SatsCard(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsCard".to_string(),
            )),
            CkTapCard::SatsChip(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsChip".to_string(),
            )),
        }?;

        Ok(Self {
            reader: Mutex::new(card),
            cmd: RwLock::new(cmd),
        })
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
        }
    }

    /// Start the setup process
    pub async fn setup(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        let new_pin = cmd.new_pin.as_bytes();
        if new_pin.len() < 6 || new_pin.len() > 32 {
            return Err(TapSignerReaderError::InvalidPinLength(new_pin.len() as u8));
        }

        if cmd.new_pin.chars().all(char::is_numeric) {
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
                let response = self.change(c.continue_cmd, c.backup).await;
                Ok(response)
            }

            // already complete, just return the backup
            SetupCmdResponse::Complete { backup } => {
                return Ok(SetupCmdResponse::Complete { backup });
            }
        }
    }
}

impl TapSignerReader {
    async fn init_backup_change(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        let _init_response = self
            .reader
            .lock()
            .await
            .init(cmd.chain_code, &cmd.factory_pin)
            .await
            .map_err(TransportError::from)?;

        Ok(self.backup_and_change(cmd).await)
    }

    async fn backup_and_change(&self, cmd: Arc<SetupCmd>) -> SetupCmdResponse {
        let backup_response = self.reader.lock().await.backup(&cmd.factory_pin).await;

        let backup = match backup_response {
            Ok(backup) => backup.data,
            Err(e) => {
                let error = TapSignerReaderError::TapSignerError(e.into());
                let response = SetupCmdResponse::ContinueFromInit(ContinueFromInit {
                    continue_cmd: cmd,
                    error,
                });

                return response;
            }
        };

        self.change(cmd.clone(), backup).await
    }

    async fn change(&self, cmd: Arc<SetupCmd>, backup: Vec<u8>) -> SetupCmdResponse {
        let change_response = self
            .reader
            .lock()
            .await
            .change(&cmd.new_pin, &cmd.factory_pin)
            .await;

        if let Err(e) = change_response {
            let error = TapSignerReaderError::TapSignerError(e.into());
            let response = SetupCmdResponse::ContinueFromBackup(ContinueFromBackup {
                backup,
                continue_cmd: cmd,
                error,
            });

            return response;
        }

        SetupCmdResponse::Complete { backup }
    }
}

#[uniffi::export]
impl SetupCmd {
    #[uniffi::constructor]
    pub fn new(factory_pin: String, new_pin: String) -> Self {
        let chain_code = rust_cktap::rand_chaincode(&mut secp256k1::rand::thread_rng());

        Self {
            factory_pin,
            new_pin,
            chain_code,
        }
    }
}

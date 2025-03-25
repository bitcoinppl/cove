use std::hash::Hasher;
use std::sync::Arc;

use bitcoin::bip32::Fingerprint;
use bitcoin::hashes::HashEngine as _;
use bitcoin::secp256k1;
use nid::Nanoid;
use parking_lot::Mutex as SyncMutex;
use parking_lot::RwLock;
use rust_cktap::apdu::DeriveResponse;
use rust_cktap::{CkTapCard, commands::CkTransport as _};
use tokio::sync::Mutex;

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

    /// Last response from the setup process, has started, if the last response is `Complete` then the setup process is complete
    last_response: SyncMutex<Option<Arc<SetupCmdResponse>>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    Setup(Arc<SetupCmd>),
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
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum SetupCmdResponse {
    ContinueFromInit(ContinueFromInit),
    ContinueFromBackup(ContinueFromBackup),
    ContinueFromDerive(ContinueFromDerive),
    Complete(TapSignerImportComplete),
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
pub struct TapSignerImportComplete {
    pub backup: Vec<u8>,
    pub derive_info: DeriveInfo,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct DeriveInfo {
    pub master_xpub: Vec<u8>,
    pub xpub: Vec<u8>,
    pub chain_code: Vec<u8>,
    pub path: Vec<u32>,
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

        let id: Nanoid = Nanoid::new();

        Ok(Self {
            id: id.to_string(),
            reader: Mutex::new(card),
            cmd: RwLock::new(cmd),
            last_response: SyncMutex::new(None),
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
        let path: [u32; 3] = [84, 0, 0];
        let derive_response = self
            .reader
            .lock()
            .await
            .derive(&path, &cmd.factory_pin)
            .await;

        let derive = match derive_response {
            Ok(derive) => derive,
            Err(e) => {
                let error = TapSignerReaderError::TapSignerError(e.into());
                let response = SetupCmdResponse::ContinueFromBackup(ContinueFromBackup {
                    backup,
                    continue_cmd: cmd,
                    error,
                });

                *self.last_response.lock() = Some(response.clone().into());
                return response;
            }
        };

        let derive_info = DeriveInfo::from_response(derive, path.to_vec());
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

        let complete = TapSignerImportComplete {
            backup,
            derive_info,
        };

        *self.last_response.lock() = Some(SetupCmdResponse::Complete(complete.clone()).into());
        SetupCmdResponse::Complete(complete)
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
    pub fn from_response(derive_response: DeriveResponse, path: Vec<u32>) -> Self {
        let master_xpub = derive_response.master_pubkey;
        let chain_code = derive_response.chain_code;
        let xpub = derive_response
            .pubkey
            .expect("has pubkey because path was given");

        Self {
            master_xpub,
            xpub,
            chain_code,
            path,
        }
    }
}

impl TryInto<bitcoin::bip32::Xpub> for DeriveInfo {
    type Error = Box<dyn std::error::Error>;

    fn try_into(self) -> Result<bitcoin::bip32::Xpub, Self::Error> {
        use bitcoin::{
            NetworkKind,
            bip32::{ChainCode, ChildNumber, Xpub},
            hashes::{Hash as _, ripemd160, sha256},
            secp256k1::PublicKey,
        };

        // TODO: get from derive info
        let network = NetworkKind::Main;

        // dept is always 3 and always the first (0) child, derives the standard derivation path
        let depth = 3;
        let child_number = ChildNumber::Hardened { index: 0 };

        let parent_fingerprint = {
            let parent_key = bitcoin::PublicKey::from_slice(&self.master_xpub)?;
            let mut engine = sha256::Hash::engine();
            engine.input(&parent_key.to_bytes());

            let sha = sha256::Hash::from_engine(engine);
            let mut ripemd_engine = ripemd160::Hash::engine();

            ripemd_engine.input(&sha[..]);
            let hash160 = ripemd160::Hash::from_engine(ripemd_engine);

            let hash_bytes: [u8; 4] = hash160[..4].try_into()?;
            Fingerprint::from(hash_bytes)
        };

        let public_key = PublicKey::from_slice(&self.xpub)?;

        let chain_code_bytes: [u8; 32] = self.chain_code.try_into().unwrap();
        let chain_code = ChainCode::from(chain_code_bytes);

        Ok(Xpub {
            network,
            depth,
            parent_fingerprint,
            child_number,
            public_key,
            chain_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    #[test]
    fn test_derive_info_try_into_xpub() {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let xpub = bitcoin::bip32::Xpub::from_str(xpub).unwrap();

        println!("{:?}", xpub);
    }
}

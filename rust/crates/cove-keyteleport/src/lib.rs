pub mod crypto;
pub mod numeric_code;
pub mod packet;
pub mod password;
pub mod payload;
pub mod receiver;
pub mod sender;

pub use numeric_code::NumericCode;
pub use packet::{Packet, PsbtPacket, ReceiverPacket, SenderPacket};
pub use password::TeleportPassword;
pub use payload::{
    DecodedPayload, NoteRecord, NotesPayload, NotesRecord, PasswordRecord, Payload,
    UnsupportedPayloadKind, XprvPayload,
};
pub use receiver::{PendingPayload, ReceiveRequest, ReceiverSession};
pub use sender::{SendRequest, SendResponse, SenderSession};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid numeric receiver code")]
    InvalidNumericCode,

    #[error("invalid teleport password")]
    InvalidTeleportPassword,

    #[error("invalid receiver packet")]
    InvalidReceiverPacket,

    #[error("invalid sender packet")]
    InvalidSenderPacket,

    #[error("invalid Key Teleport packet")]
    InvalidPacket,

    #[error("invalid Key Teleport URL")]
    InvalidUrl,

    #[error("unsupported Key Teleport payload type {0}")]
    UnsupportedPayload(UnsupportedPayloadKind),

    #[error("invalid mnemonic payload")]
    InvalidMnemonicPayload,

    #[error("unsupported Key Teleport mnemonic word count {0}; expected 12, 18, or 24 words")]
    UnsupportedMnemonicWordCount(usize),

    #[error("invalid xprv payload")]
    InvalidXprvPayload,

    #[error("xprv payload is not a master key")]
    NonMasterXprvPayload,

    #[error("invalid secure notes payload")]
    InvalidNotesPayload,

    #[error("checksum verification failed")]
    Checksum,

    #[error("invalid secp256k1 key")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),

    #[error("invalid BIP39 mnemonic")]
    Bip39(#[from] bip39::Error),

    #[error("invalid BBQr data")]
    BbqrJoin(#[from] bbqr::join::JoinError),

    #[error("failed to build BBQr data")]
    BbqrSplit(#[from] bbqr::split::SplitError),

    #[error("invalid base32 data")]
    Base32(#[from] data_encoding::DecodeError),

    #[error("invalid base58 data")]
    Base58(#[from] bitcoin::base58::Error),

    #[error("invalid BIP32 key")]
    Bip32(#[from] bitcoin::bip32::Error),

    #[error("invalid URL")]
    Url(#[from] url::ParseError),
}

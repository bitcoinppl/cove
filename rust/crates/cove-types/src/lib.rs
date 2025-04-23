uniffi::setup_scaffolding!();

mod address_index;
mod block_size;
mod chain_position;
mod outpoint;
mod wallet_id;

pub mod address;
pub mod amount;
pub mod color;
pub mod color_scheme;
pub mod confirm;
pub mod fees;
pub mod network;
pub mod psbt;
pub mod redb;
pub mod transaction;
pub mod unit;

// export the types
pub use address_index::AddressIndex;
pub use block_size::BlockSizeLast;
pub use confirm::{
    AddressAndAmount, ConfirmDetails, ConfirmDetailsError, InputOutputDetails, SplitOutput,
};

pub use chain_position::ChainPosition;
pub use network::Network;
pub use outpoint::OutPoint;
pub use transaction::txid::TxId;
pub use wallet_id::WalletId;

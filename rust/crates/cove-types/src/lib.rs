uniffi::setup_scaffolding!();

mod address_index;
mod block_size;
mod chain_position;
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
pub mod utxo;

// export the types
pub use address_index::AddressIndex;
pub use block_size::BlockSizeLast;
pub use confirm::{
    AddressAndAmount, ConfirmDetails, ConfirmDetailsError, InputOutputDetails, SplitOutput,
};

pub use chain_position::ChainPosition;
pub use network::Network;
pub use wallet_id::WalletId;

pub use transaction::out_point::OutPoint;
pub use transaction::tx_id::TxId;
pub use transaction::tx_in::TxIn;
pub use transaction::tx_out::TxOut;

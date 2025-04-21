uniffi::setup_scaffolding!();

mod address_index;
mod block_size;
mod confirm_details;
mod outpoint;
mod txid;
mod wallet_id;

pub use address_index::AddressIndex;
pub use block_size::BlockSizeLast;
pub use confirm_details::{
    AddressAndAmount, ConfirmDetails, ConfirmDetailsError, InputOutputDetails, SplitOutput,
};
pub use outpoint::OutPoint;
pub use txid::TxId;
pub use wallet_id::WalletId;

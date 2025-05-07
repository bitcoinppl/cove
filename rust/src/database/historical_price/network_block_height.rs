use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use cove_types::Network;

// Define a custom type that implements redb::TypeName for BlockNumber
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NetworkBlockHeight {
    network: Network,
    block_height: u32,
}

impl NetworkBlockHeight {
    pub fn new(network: impl Into<Network>, block_number: u32) -> Self {
        Self { network: network.into(), block_height: block_number }
    }
}

impl redb::Key for NetworkBlockHeight {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for NetworkBlockHeight {
    type SelfType<'a> = NetworkBlockHeight;
    type AsBytes<'a> = [u8; 5];

    fn fixed_width() -> Option<usize> {
        Some(5)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let network = Network::try_from(data[0]).expect("invalid network");
        let block_number = u32::from_le_bytes(data[1..].try_into().expect("invalid block number"));
        Self::new(network, block_number)
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        let mut bytes = [0; 5];
        bytes[0] = value.network.into();
        bytes[1..].copy_from_slice(&value.block_height.to_le_bytes());

        bytes
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new(std::any::type_name::<NetworkBlockHeight>())
    }
}

use bitcoin::hashes::serde_macros::serde_details::SerdeHash;
use std::cmp::Ordering;
use zerocopy::{FromBytes, Immutable, IntoBytes};

#[derive(FromBytes, IntoBytes, Immutable, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct OutpointKey {
    pub id: [u8; 32],
    pub index: u32,
}

impl From<&bitcoin::OutPoint> for OutpointKey {
    fn from(id: &bitcoin::OutPoint) -> Self {
        Self::new(id.txid, id.vout)
    }
}

impl From<bitcoin::OutPoint> for OutpointKey {
    fn from(id: bitcoin::OutPoint) -> Self {
        Self::from(&id)
    }
}

impl OutpointKey {
    pub fn new(id: impl AsRef<[u8; 32]>, index: u32) -> Self {
        Self {
            id: *id.as_ref(),
            index,
        }
    }

    #[allow(dead_code)]
    pub fn id(&self) -> bitcoin::Txid {
        bitcoin::Txid::from_slice_delegated(&self.id).expect("id is 32 bytes")
    }
}

impl redb::Key for OutpointKey {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for OutpointKey {
    type SelfType<'a>
        = OutpointKey
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let id = data[..32].try_into().expect("id is 32 bytes");
        let index = u32::from_be_bytes(data[32..36].try_into().expect("index is 4 bytes"));

        Self { id, index }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.as_bytes()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new("OutPointKey::bitcoin::OutPoint")
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    #[test]
    fn test_in_out_id() {
        let id = OutpointKey::new(
            bitcoin::Txid::from_str(
                "d9f76c1c2338eb2010255c16e7cbdf72c1263e81c08a465b5d1d76a36d9980dc",
            )
            .unwrap(),
            0,
        );

        assert_eq!(
            id.id(),
            bitcoin::Txid::from_str(
                "d9f76c1c2338eb2010255c16e7cbdf72c1263e81c08a465b5d1d76a36d9980dc"
            )
            .unwrap()
        );
    }
}

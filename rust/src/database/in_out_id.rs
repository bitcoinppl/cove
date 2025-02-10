use std::cmp::Ordering;
use zerocopy::{FromBytes, Immutable, IntoBytes};

#[derive(FromBytes, IntoBytes, Immutable, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct InOutId {
    pub id: [u8; 32],
    pub index: u32,
}

impl redb::Key for InOutId {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for InOutId {
    type SelfType<'a>
        = InOutId
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
        redb::TypeName::new("InOutId::bip329::InOutId")
    }
}

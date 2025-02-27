use std::{cmp::Ordering, fmt::Debug};

use derive_more::Display;
use redb::{Key, TypeName, Value};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Wrapper type to handle keys and values using cbor serialization
#[derive(Debug, Display, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Cbor<T>(pub T);

impl<T> Value for Cbor<T>
where
    T: Debug + Serialize + for<'a> Deserialize<'a>,
{
    type SelfType<'a>
        = T
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        cbor4ii::serde::from_slice(data).unwrap()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        let buf = Vec::new();
        cbor4ii::serde::to_vec(buf, value).unwrap()
    }

    fn type_name() -> TypeName {
        TypeName::new(&format!("Cbor<{}>", std::any::type_name::<T>()))
    }
}

impl<T> Key for Cbor<T>
where
    T: Debug + Serialize + DeserializeOwned + Ord,
{
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        Self::from_bytes(data1).cmp(&Self::from_bytes(data2))
    }
}

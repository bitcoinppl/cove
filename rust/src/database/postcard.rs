use std::{cmp::Ordering, fmt::Debug};

use derive_more::Display;
use redb::{Key, TypeName, Value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Wrapper type to handle keys and values using bincode serialization
#[derive(Debug, Display, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Postcard<T>(pub T);

impl<T> Value for Postcard<T>
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
        postcard::from_bytes(data).unwrap()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        postcard::to_allocvec(value).unwrap()
    }

    fn type_name() -> TypeName {
        TypeName::new(&format!("Postcard<{}>", std::any::type_name::<T>()))
    }
}

impl<T> Key for Postcard<T>
where
    T: Debug + Serialize + DeserializeOwned + Ord,
{
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        Self::from_bytes(data1).cmp(&Self::from_bytes(data2))
    }
}

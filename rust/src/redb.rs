use std::any::type_name;

use redb::TypeName;
use serde::{Deserialize, Serialize};

/// Wrapper type to handle keys and values using serde serialization
#[derive(Debug)]
pub struct Json<T>(pub T);

impl<T> redb::Value for Json<T>
where
    T: core::fmt::Debug + Serialize + for<'a> Deserialize<'a>,
{
    type SelfType<'a> = T
    where
        Self: 'a;

    type AsBytes<'a> = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        serde_json::from_slice(data).expect("failed to deserialize")
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        serde_json::to_vec(value).expect("failed to serialize")
    }

    fn type_name() -> TypeName {
        TypeName::new(&format!("SerdeJson<{}>", type_name::<T>()))
    }
}

use std::cmp::Ordering;

const KEY_LEN: usize = 36;

/// Key for per-outpoint records, encoded as the 32 txid bytes followed by the little-endian vout
///
/// The layout matches what earlier builds wrote through a native-endian `#[repr(C)]` struct on
/// every shipped target, so existing tables stay readable without a migration. Keys group by txid
/// but do not sort by vout within a txid
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutPointKey {
    pub id: [u8; 32],
    pub index: u32,
}

impl From<&bitcoin::OutPoint> for OutPointKey {
    fn from(id: &bitcoin::OutPoint) -> Self {
        Self::new(id.txid, id.vout)
    }
}

impl From<bitcoin::OutPoint> for OutPointKey {
    fn from(id: bitcoin::OutPoint) -> Self {
        Self::from(&id)
    }
}

impl OutPointKey {
    pub fn new(id: impl AsRef<[u8; 32]>, index: u32) -> Self {
        Self { id: *id.as_ref(), index }
    }
}

impl redb::Key for OutPointKey {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for OutPointKey {
    type SelfType<'a>
        = Self
    where
        Self: 'a;

    type AsBytes<'a>
        = [u8; KEY_LEN]
    where
        Self: 'a;

    // redb stores the fixed width in table metadata, so this must stay variable-width
    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let id = data[..32].try_into().expect("id is 32 bytes");
        let index = u32::from_le_bytes(data[32..KEY_LEN].try_into().expect("index is 4 bytes"));

        Self { id, index }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a + 'b,
    {
        let mut bytes = [0; KEY_LEN];
        bytes[..32].copy_from_slice(&value.id);
        bytes[32..].copy_from_slice(&value.index.to_le_bytes());

        bytes
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new("OutPointKey::bitcoin::OutPoint")
    }
}

#[cfg(test)]
mod tests {
    use redb::Value as _;

    use super::OutPointKey;

    #[test]
    fn key_keeps_stored_layout_and_decodes_non_zero_index() {
        let key = OutPointKey { id: [7; 32], index: 256 };

        let bytes = OutPointKey::as_bytes(&key);

        // existing rows were written with this exact layout, so it must not change
        assert_eq!(&bytes[32..], &[0, 1, 0, 0]);
        assert_eq!(OutPointKey::from_bytes(&bytes), key);
    }
}

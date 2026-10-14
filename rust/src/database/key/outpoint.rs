use std::cmp::Ordering;

const KEY_LEN: usize = 36;

/// Key for per-outpoint records, encoded as the 32 txid bytes followed by the big-endian vout
///
/// Big-endian makes the byte order match the numeric order, so keys sort by (txid, vout)
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

    fn fixed_width() -> Option<usize> {
        Some(KEY_LEN)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let id = data[..32].try_into().expect("id is 32 bytes");
        let index = u32::from_be_bytes(data[32..KEY_LEN].try_into().expect("index is 4 bytes"));

        Self { id, index }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a + 'b,
    {
        let mut bytes = [0; KEY_LEN];
        bytes[..32].copy_from_slice(&value.id);
        bytes[32..].copy_from_slice(&value.index.to_be_bytes());

        bytes
    }

    // distinct from the legacy little-endian name so a v2 table can never be opened with this decoder
    fn type_name() -> redb::TypeName {
        redb::TypeName::new("OutPointKey::bitcoin::OutPoint::BigEndian")
    }
}

#[cfg(test)]
mod tests {
    use redb::Value as _;

    use super::OutPointKey;

    #[test]
    fn key_round_trips_non_zero_index() {
        let key = OutPointKey { id: [7; 32], index: 256 };

        let bytes = OutPointKey::as_bytes(&key);
        let decoded = OutPointKey::from_bytes(&bytes);

        assert_eq!(decoded, key);
        assert_eq!(&bytes[32..], &[0, 0, 1, 0]);
    }
}

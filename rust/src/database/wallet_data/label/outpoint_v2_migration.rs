//! Moves input/output records from the v2 tables, whose keys encoded the vout little-endian,
//! into the v3 tables keyed by the big-endian [`OutPointKey`]

use std::cmp::Ordering;

use bip329::{InputRecord, OutputRecord};
use redb::{ReadableTable as _, TableDefinition, TableHandle as _, WriteTransaction};

use super::{INPUT_TABLE, OUTPUT_TABLE, SerdeRecord};
use crate::database::{error::DatabaseError, key::OutPointKey};

pub(crate) const LEGACY_INPUT_TABLE: TableDefinition<LegacyOutPointKey, SerdeRecord<InputRecord>> =
    TableDefinition::new("input_records_v2.cbor");

pub(crate) const LEGACY_OUTPUT_TABLE: TableDefinition<
    LegacyOutPointKey,
    SerdeRecord<OutputRecord>,
> = TableDefinition::new("output_records_v2.cbor");

/// Key as written by earlier builds: txid bytes followed by the vout in native (little-endian) order
#[derive(Debug)]
pub(crate) struct LegacyOutPointKey {
    pub(crate) id: [u8; 32],
    pub(crate) index: u32,
}

impl From<LegacyOutPointKey> for OutPointKey {
    fn from(key: LegacyOutPointKey) -> Self {
        Self { id: key.id, index: key.index }
    }
}

impl redb::Key for LegacyOutPointKey {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for LegacyOutPointKey {
    type SelfType<'a>
        = Self
    where
        Self: 'a;

    type AsBytes<'a>
        = [u8; 36]
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
        let index = u32::from_le_bytes(data[32..36].try_into().expect("index is 4 bytes"));

        Self { id, index }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a + 'b,
    {
        let mut bytes = [0; 36];
        bytes[..32].copy_from_slice(&value.id);
        bytes[32..].copy_from_slice(&value.index.to_le_bytes());

        bytes
    }

    // must stay byte-for-byte what earlier builds recorded in the table metadata
    fn type_name() -> redb::TypeName {
        redb::TypeName::new("OutPointKey::bitcoin::OutPoint")
    }
}

/// Copies every v2 row into the v3 tables and deletes the v2 tables, within the caller's transaction
///
/// Rows already present in v3 are kept. Does nothing when the v2 tables do not exist
pub(crate) fn migrate_v2_outpoint_tables(
    write_txn: &WriteTransaction,
) -> Result<(), DatabaseError> {
    migrate_table(write_txn, LEGACY_INPUT_TABLE, INPUT_TABLE)?;
    migrate_table(write_txn, LEGACY_OUTPUT_TABLE, OUTPUT_TABLE)?;

    Ok(())
}

fn migrate_table<V>(
    write_txn: &WriteTransaction,
    legacy: TableDefinition<LegacyOutPointKey, V>,
    current: TableDefinition<OutPointKey, V>,
) -> Result<(), DatabaseError>
where
    V: redb::Value + 'static,
    for<'a> V::SelfType<'a>: std::borrow::Borrow<V::SelfType<'a>>,
{
    let exists = write_txn.list_tables()?.any(|handle| handle.name() == legacy.name());
    if !exists {
        return Ok(());
    }

    {
        let legacy_table = write_txn.open_table(legacy)?;
        let mut current_table = write_txn.open_table(current)?;

        for row in legacy_table.iter()? {
            let (key, value) = row?;
            let key = OutPointKey::from(key.value());

            if current_table.get(key.clone())?.is_some() {
                continue;
            }

            current_table.insert(key, value.value())?;
        }
    }

    write_txn.delete_table(legacy)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bip329::{InputRecord, OutputRecord};
    use bitcoin::{OutPoint, Txid, hashes::Hash as _};
    use redb::TableHandle as _;

    use super::{LEGACY_INPUT_TABLE, LEGACY_OUTPUT_TABLE, LegacyOutPointKey};
    use crate::database::{
        Record,
        record::Timestamps,
        wallet_data::{WalletDataDb, WalletDataStorage},
    };
    use crate::wallet::metadata::WalletId;

    fn txid() -> Txid {
        Txid::from_byte_array([9; 32])
    }

    fn legacy_key(vout: u32) -> LegacyOutPointKey {
        LegacyOutPointKey { id: *txid().as_ref(), index: vout }
    }

    #[test]
    fn opening_wallet_data_migrates_little_endian_v2_records() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let db = redb::Database::create(tmp.path().join("wallet_data.redb"))
            .expect("failed to create db");

        // seed the tables exactly as an older build wrote them, out of vout order on purpose
        let write_txn = db.begin_write().expect("begin write");
        {
            let mut inputs = write_txn.open_table(LEGACY_INPUT_TABLE).expect("open v2 inputs");
            let mut outputs = write_txn.open_table(LEGACY_OUTPUT_TABLE).expect("open v2 outputs");

            for vout in [256, 1, 0] {
                let ref_ = OutPoint { txid: txid(), vout };
                let timestamps = Timestamps::new(1, 2);

                let input = InputRecord { ref_, label: Some(format!("input {vout}")) };
                inputs
                    .insert(legacy_key(vout), Record::with_timestamps(input, timestamps))
                    .expect("insert v2 input");

                let output =
                    OutputRecord { ref_, label: Some(format!("output {vout}")), spendable: None };
                outputs
                    .insert(legacy_key(vout), Record::with_timestamps(output, timestamps))
                    .expect("insert v2 output");
            }
        }
        write_txn.commit().expect("commit v2 rows");

        let db = Arc::new(db);
        let wallet_data = WalletDataDb::new_with_db(
            WalletId::preview_new_random(),
            db.clone(),
            WalletDataStorage::Persistent,
        )
        .expect("open wallet data");

        let input_vouts = wallet_data
            .labels
            .txn_input_records_iter(txid())
            .expect("input iter")
            .map(|record| (record.item.ref_.vout, record.item.label))
            .collect::<Vec<_>>();

        let output_vouts = wallet_data
            .labels
            .txn_output_records_iter(txid())
            .expect("output iter")
            .map(|record| (record.item.ref_.vout, record.item.label))
            .collect::<Vec<_>>();

        let expected =
            |kind: &str| [0, 1, 256].map(|vout| (vout, Some(format!("{kind} {vout}")))).to_vec();
        assert_eq!(input_vouts, expected("input"));
        assert_eq!(output_vouts, expected("output"));

        let table_names = db
            .begin_read()
            .expect("begin read")
            .list_tables()
            .expect("list tables")
            .map(|handle| handle.name().to_string())
            .collect::<Vec<_>>();

        assert!(!table_names.contains(&"input_records_v2.cbor".to_string()));
        assert!(!table_names.contains(&"output_records_v2.cbor".to_string()));
        assert!(table_names.contains(&"input_records_v3.cbor".to_string()));
        assert!(table_names.contains(&"output_records_v3.cbor".to_string()));
    }

    #[test]
    fn fresh_wallet_data_creates_only_v3_tables() {
        let wallet_data =
            WalletDataDb::new_in_memory(WalletId::preview_new_random()).expect("open wallet data");

        let table_names = wallet_data
            .db
            .begin_read()
            .expect("begin read")
            .list_tables()
            .expect("list tables")
            .map(|handle| handle.name().to_string())
            .collect::<Vec<_>>();

        assert!(table_names.iter().all(|name| !name.ends_with("_v2.cbor")));
        assert!(table_names.contains(&"input_records_v3.cbor".to_string()));
    }
}

use std::collections::BTreeSet;

use super::{MASTER_KEY_RECORD_ID, master_key_filename};
use super::{wallet_filename_from_record_id, wallet_record_id_from_filename};

pub const MASTER_KEY_DIRECTORY: &str = "master-key";
pub const WALLETS_DIRECTORY: &str = "wallets";

pub fn master_key_location() -> String {
    format!("{MASTER_KEY_DIRECTORY}/{}", master_key_filename())
}

pub fn legacy_master_key_location() -> String {
    master_key_filename()
}

pub fn master_key_read_locations() -> Vec<String> {
    vec![master_key_location(), legacy_master_key_location()]
}

pub fn master_key_upload_location() -> String {
    master_key_location()
}

pub fn wallet_location_from_record_id(record_id: &str) -> String {
    format!("{WALLETS_DIRECTORY}/{}", wallet_filename_from_record_id(record_id))
}

pub fn legacy_wallet_location_from_record_id(record_id: &str) -> String {
    wallet_filename_from_record_id(record_id)
}

pub fn wallet_read_locations(record_id: &str) -> Vec<String> {
    vec![
        wallet_location_from_record_id(record_id),
        legacy_wallet_location_from_record_id(record_id),
    ]
}

pub fn wallet_upload_location(record_id: &str) -> String {
    wallet_location_from_record_id(record_id)
}

pub fn locations_for_record_id(record_id: &str) -> Vec<String> {
    if record_id == MASTER_KEY_RECORD_ID {
        return master_key_read_locations();
    }

    wallet_read_locations(record_id)
}

pub fn wallet_record_id_from_location(location: &str) -> Option<&str> {
    let filename = location
        .strip_prefix(WALLETS_DIRECTORY)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(location);

    wallet_record_id_from_filename(filename)
}

pub fn is_master_key_location(location: &str) -> bool {
    location == master_key_location() || location == legacy_master_key_location()
}

pub fn is_wallet_location(location: &str) -> bool {
    wallet_record_id_from_location(location).is_some()
}

pub fn is_backup_location(location: &str) -> bool {
    is_master_key_location(location) || is_wallet_location(location)
}

pub fn has_backup_location<'a>(locations: impl IntoIterator<Item = &'a str>) -> bool {
    locations.into_iter().any(is_backup_location)
}

pub fn has_master_key_location<'a>(locations: impl IntoIterator<Item = &'a str>) -> bool {
    locations.into_iter().any(is_master_key_location)
}

pub fn dedupe_wallet_record_ids<'a>(locations: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    locations
        .into_iter()
        .filter_map(wallet_record_id_from_location)
        .map(String::from)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_data::wallet_filename_from_record_id;

    #[test]
    fn parses_legacy_flat_wallet_location() {
        assert_eq!(wallet_record_id_from_location("wallet-record-a.json"), Some("record-a"),);
    }

    #[test]
    fn parses_kind_prefixed_wallet_location() {
        assert_eq!(
            wallet_record_id_from_location("wallets/wallet-record-a.json"),
            Some("record-a"),
        );
    }

    #[test]
    fn rejects_non_wallet_location() {
        assert_eq!(wallet_record_id_from_location("master-key/masterkey-a.json"), None);
    }

    #[test]
    fn master_key_read_locations_prefer_current_layout() {
        let locations = master_key_read_locations();

        assert_eq!(locations[0], master_key_location());
        assert_eq!(locations[1], legacy_master_key_location());
    }

    #[test]
    fn wallet_read_locations_prefer_current_layout() {
        let locations = wallet_read_locations("record-a");

        assert_eq!(locations[0], "wallets/wallet-record-a.json");
        assert_eq!(locations[1], "wallet-record-a.json");
    }

    #[test]
    fn recognizes_legacy_and_current_master_key_locations() {
        assert!(is_master_key_location(&master_key_location()));
        assert!(is_master_key_location(&legacy_master_key_location()));
        assert!(!is_master_key_location("wallets/wallet-record-a.json"));
    }

    #[test]
    fn dedupes_legacy_and_kind_prefixed_wallet_locations() {
        let legacy = wallet_filename_from_record_id("record-a");
        let current = wallet_location_from_record_id("record-a");

        assert_eq!(
            dedupe_wallet_record_ids([legacy.as_str(), current.as_str(), "wallet-record-b.json"]),
            vec!["record-a".to_string(), "record-b".to_string()],
        );
    }
}

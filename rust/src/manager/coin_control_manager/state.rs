use std::sync::Arc;

use ahash::HashSet;
use bdk_wallet::LocalOutput;
use bitcoin::Amount;
use cove_types::{
    Network, OutPoint, WalletId,
    unit::BitcoinUnit,
    utxo::{Utxo, UtxoType},
};

use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletMetadata};

use super::{CoinControlListSort, ListSortDirection};

type State = CoinControlManagerState;
type SortState = super::CoinControlListSortState;
type ListSort = super::CoinControlListSort;

#[derive(Clone, Debug, Hash, Eq, PartialEq, uniffi::Object)]
pub struct CoinControlManagerState {
    pub wallet_id: WalletId,
    pub unit: BitcoinUnit,
    pub network: Network,

    pub utxos: Vec<Utxo>,
    pub filtered_utxos: FilteredUtxos,
    pub sort: SortState,
    pub selected_utxos: Vec<Arc<OutPoint>>,
    pub search: String,

    /// Set of currently locked outpoints, populated from the
    /// `locked_outpoints` table on construction and after lock/unlock actions.
    /// Used to populate `Utxo::is_locked` for the UI.
    pub locked_outpoints: Vec<bitcoin::OutPoint>,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, uniffi::Object)]
pub enum FilteredUtxos {
    All,
    Search(Vec<Utxo>),
}

// MARK: STATE
impl State {
    pub fn new(metadata: WalletMetadata, unspent: Vec<LocalOutput>) -> Self {
        let wallet_id = metadata.id.clone();
        let unit = metadata.selected_unit;
        let network = metadata.network;
        let utxos: Vec<Utxo> =
            unspent.into_iter().filter_map(|o| Utxo::try_from_local(o, network).ok()).collect();

        let sort = SortState::default();
        let selected_utxos = vec![];
        let search = String::new();

        // Best-effort: load locked outpoints from the wallet data db.
        // Failures are non-fatal — the manager will simply show no locks.
        let locked_outpoints = load_locked_outpoints(&wallet_id);

        let mut state = Self {
            wallet_id,
            unit,
            network,
            utxos,
            sort,
            selected_utxos,
            search,
            filtered_utxos: FilteredUtxos::All,
            locked_outpoints,
        };

        state.apply_lock_state();
        state
    }

    pub fn utxos(&self) -> Vec<Utxo> {
        match &self.filtered_utxos {
            FilteredUtxos::All => self.utxos.clone(),
            FilteredUtxos::Search(utxos) => utxos.clone(),
        }
    }

    /// Refresh `is_locked` on every utxo in `utxos` and `filtered_utxos`
    /// from the current `locked_outpoints` set.
    pub fn apply_lock_state(&mut self) {
        let locked: HashSet<bitcoin::OutPoint> = self.locked_outpoints.iter().copied().collect();

        for utxo in &mut self.utxos {
            utxo.is_locked = locked.contains(&bitcoin::OutPoint::from(utxo.outpoint.as_ref()));
        }

        if let FilteredUtxos::Search(filtered) = &mut self.filtered_utxos {
            for utxo in filtered {
                utxo.is_locked = locked.contains(&bitcoin::OutPoint::from(utxo.outpoint.as_ref()));
            }
        }
    }

    /// Refresh `locked_outpoints` from the wallet data db and re-apply lock state.
    pub fn reload_locked_outpoints(&mut self) {
        self.locked_outpoints = load_locked_outpoints(&self.wallet_id);
        self.apply_lock_state();
    }

    pub fn load_utxo_labels(&mut self) {
        let utxos = &mut self.utxos;

        let Some(wallet_db) = WalletDataDb::new_or_existing(self.wallet_id.clone()).ok() else {
            return;
        };
        let labels_db = wallet_db.labels;

        for utxo in utxos.iter_mut() {
            let label = labels_db
                .get_txn_label_record(utxo.outpoint.txid)
                .ok()
                .flatten()
                .map(|record| record.item.label)
                .unwrap_or_else(|| {
                    labels_db
                        .get_address_record(utxo.address.as_unchecked())
                        .ok()
                        .flatten()
                        .and_then(|record| record.item.label)
                });

            utxo.label = label;
        }
    }

    pub fn sort_utxos(&mut self, sort: ListSort) {
        let utxos = match &mut self.filtered_utxos {
            FilteredUtxos::All => &mut self.utxos,
            FilteredUtxos::Search(utxos) => utxos,
        };

        match sort {
            CoinControlListSort::Date(ListSortDirection::Ascending) => {
                utxos.sort_by_key(|a| a.datetime);
            }
            CoinControlListSort::Date(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.datetime.cmp(&b.datetime).reverse());
            }

            CoinControlListSort::Name(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.label.cmp(&b.label).reverse());
            }
            CoinControlListSort::Name(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.label.cmp(&b.label));
            }

            CoinControlListSort::Amount(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.amount.cmp(&b.amount));
            }

            CoinControlListSort::Amount(ListSortDirection::Descending) => {
                utxos.sort_by(|a, b| a.amount.cmp(&b.amount).reverse());
            }

            CoinControlListSort::Change(UtxoType::Output) => {
                utxos.sort_by(|a, b| a.type_.cmp(&b.type_).reverse());
            }

            CoinControlListSort::Change(UtxoType::Change) => {
                utxos.sort_by_key(|a| a.type_);
            }
        }
    }

    pub fn reset_search(&mut self) {
        let sort = self.sort.sorter();
        self.search = String::new();
        self.filtered_utxos = FilteredUtxos::All;
        self.sort_utxos(sort);
    }

    pub fn filter_utxos(&mut self, search: &str) {
        let search = &search.to_ascii_lowercase();

        // first fuzzy match on utxo label name
        let mut filtered_utxos = self
            .utxos
            .iter()
            .filter_map(|utxo| {
                let utxo_name = utxo.name().to_ascii_lowercase();
                let distance = strsim::normalized_damerau_levenshtein(&utxo_name, search);

                if distance >= 0.20 || utxo_name.contains(search) || utxo_name.starts_with(search) {
                    Some((utxo, distance))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        filtered_utxos.sort_unstable_by(|a, b| {
            a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).reverse()
        });

        let mut filtered_utxos =
            filtered_utxos.into_iter().map(|(utxo, _)| utxo.clone()).collect::<Vec<_>>();

        // next check for an exact amount match if search is digits only
        if let Ok(numeric) = search.parse::<f64>() {
            let amount_sats = numeric.trunc() as u64;
            let amount_btc_in_sats = Amount::from_btc(numeric).unwrap_or(Amount::ZERO).to_sat();

            let mut filtered_on_amount: Vec<_> = self
                .utxos
                .iter()
                .filter(|utxo| {
                    let amount = utxo.amount.as_sats();
                    amount == amount_sats || amount == amount_btc_in_sats
                })
                .cloned()
                .collect();

            // add the exact amount matches to the front of the list
            filtered_on_amount.extend(filtered_utxos);
            filtered_utxos = filtered_on_amount;
        }

        // if we have any results, update the state and return
        if !filtered_utxos.is_empty() {
            return self.filtered_utxos = FilteredUtxos::Search(filtered_utxos);
        }

        // FALLBACK SEARCH
        // 1. if no utxos found, and search looks like an address, search by address
        if search.starts_with("bc1") || search.starts_with("tb1") {
            let filtered = self
                .utxos
                .iter()
                .filter(|utxo| {
                    let address = &utxo.address.to_string();
                    address == search
                        || address.starts_with(search)
                        || address.ends_with(search)
                        || address.contains(search)
                })
                .cloned()
                .collect::<Vec<_>>();

            return self.filtered_utxos = FilteredUtxos::Search(filtered);
        }

        // 2.fallback search by txid
        let filtered = self
            .utxos
            .iter()
            .filter(|utxo| {
                let tx_id_str = &utxo.outpoint.txid.to_string();
                tx_id_str == search || tx_id_str.starts_with(search)
            })
            .cloned()
            .collect::<Vec<_>>();

        self.filtered_utxos = FilteredUtxos::Search(filtered);
    }
}

use super::*;
use cove_types::utxo::ffi_preview::preview_new_utxo_list;

/// Best-effort load of all locked outpoints for `wallet_id` from the wallet
/// data db. Returns an empty vec on any error so the manager can still load.
fn load_locked_outpoints(wallet_id: &WalletId) -> Vec<bitcoin::OutPoint> {
    let Ok(db) = WalletDataDb::new_or_existing(wallet_id.clone()) else {
        return vec![];
    };

    db.locked_outpoints
        .all_locked()
        .unwrap_or_default()
        .into_iter()
        .map(|key| bitcoin::OutPoint::new(key.id(), key.index))
        .collect()
}

#[uniffi::export]
impl CoinControlManagerState {
    #[uniffi::constructor(default(output_count = 20, change_count = 4))]
    pub fn preview_new(output_count: u8, change_count: u8) -> Self {
        let metadata = WalletMetadata::preview_new();

        let wallet_id = metadata.id.clone();
        let unit = metadata.selected_unit;
        let network = metadata.network;
        let utxos = preview_new_utxo_list(output_count, change_count);
        let sort = Default::default();
        let selected_utxos = vec![];
        let search = String::new();
        let filtered_utxos = FilteredUtxos::All;
        let locked_outpoints = vec![];

        Self {
            wallet_id,
            unit,
            network,
            utxos,
            filtered_utxos,
            sort,
            selected_utxos,
            search,
            locked_outpoints,
        }
    }
}

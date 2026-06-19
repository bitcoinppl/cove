use std::sync::Arc;

use bdk_wallet::LocalOutput;
use bitcoin::Amount;
use cove_types::{
    Network, OutPoint, WalletId,
    unit::BitcoinUnit,
    utxo::{Utxo, UtxoType},
};
use tracing::error;

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
    pub lock_state_load_failed: bool,
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
        let utxos =
            unspent.into_iter().filter_map(|o| Utxo::try_from_local(o, network).ok()).collect();

        let sort = SortState::default();
        let selected_utxos = vec![];
        let search = String::new();
        let lock_state_load_failed = false;

        Self {
            wallet_id,
            unit,
            network,
            utxos,
            sort,
            selected_utxos,
            search,
            filtered_utxos: FilteredUtxos::All,
            lock_state_load_failed,
        }
    }

    pub fn utxos(&self) -> Vec<Utxo> {
        match &self.filtered_utxos {
            FilteredUtxos::All => self.utxos.clone(),
            FilteredUtxos::Search(utxos) => utxos.clone(),
        }
    }

    pub fn load_utxo_labels(&mut self) -> bool {
        let utxos = &mut self.utxos;

        let wallet_db = match WalletDataDb::new_or_existing(self.wallet_id.clone()) {
            Ok(wallet_db) => wallet_db,
            Err(error) => {
                let wallet_id = &self.wallet_id;
                error!("failed to open wallet label database wallet_id={wallet_id}: {error}");

                self.lock_state_load_failed = true;

                // fail closed so a lock-state read failure cannot make locked UTXOs selectable
                for utxo in utxos {
                    utxo.spendable = false;
                }

                return self.prune_locked_selected_utxos();
            }
        };
        let labels_db = wallet_db.labels;
        let mut lock_state_load_failed = false;

        for utxo in utxos.iter_mut() {
            let outpoint = bitcoin::OutPoint::from(utxo.outpoint.as_ref());
            let output_record = match labels_db.get_output_record(outpoint) {
                Ok(record) => record,
                Err(error) => {
                    error!("failed to load output lock state outpoint={outpoint}: {error}");
                    lock_state_load_failed = true;

                    // fail closed so a single read failure cannot make this output selectable
                    utxo.spendable = false;
                    utxo.label = None;
                    continue;
                }
            };

            let txn_label = match labels_db.get_txn_label_record(utxo.outpoint.txid) {
                Ok(record) => record.and_then(|record| record.item.label),
                Err(error) => {
                    let txid = utxo.outpoint.txid;
                    error!("failed to load transaction label txid={txid}: {error}");
                    None
                }
            };

            let label = txn_label.or_else(|| {
                match labels_db.get_address_record(utxo.address.as_unchecked()) {
                    Ok(record) => record.and_then(|record| record.item.label),
                    Err(error) => {
                        let address = utxo.address.as_unchecked();
                        error!("failed to load address label address={address:?}: {error}");
                        None
                    }
                }
            });

            utxo.label = label;
            utxo.spendable = output_record.map(|record| record.item.spendable).unwrap_or(true);
        }

        self.lock_state_load_failed = lock_state_load_failed;

        self.prune_locked_selected_utxos()
    }

    pub fn prune_locked_selected_utxos(&mut self) -> bool {
        let old_selected_utxos = self.selected_utxos.clone();
        let spendable_outpoints = self
            .utxos
            .iter()
            .filter(|utxo| utxo.spendable)
            .map(|utxo| utxo.outpoint.clone())
            .collect::<std::collections::HashSet<_>>();

        self.selected_utxos.retain(|outpoint| spendable_outpoints.contains(outpoint));

        self.selected_utxos != old_selected_utxos
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
        let lock_state_load_failed = false;

        Self {
            wallet_id,
            unit,
            network,
            utxos,
            filtered_utxos,
            sort,
            selected_utxos,
            search,
            lock_state_load_failed,
        }
    }
}

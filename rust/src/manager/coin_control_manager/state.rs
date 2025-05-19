use bdk_wallet::LocalOutput;
use cove_types::{
    Network, WalletId,
    utxo::{Utxo, UtxoType},
};

use super::{CoinControlListSort, CoinControlManagerState, ListSortDirection};

type State = CoinControlManagerState;

// MARK: STATE
impl State {
    pub fn new(wallet_id: WalletId, unspent: Vec<LocalOutput>, network: Network) -> Self {
        let utxos =
            unspent.into_iter().filter_map(|o| Utxo::try_from_local(o, network).ok()).collect();

        let sort = CoinControlListSort::Date(ListSortDirection::Descending);
        let selected_utxos = vec![];
        let search = String::new();

        Self { wallet_id, utxos, sort, selected_utxos, search }
    }

    pub fn sort_utxos(&mut self, sort: CoinControlListSort) {
        let utxos = &mut self.utxos;

        match sort {
            CoinControlListSort::Date(ListSortDirection::Ascending) => {
                utxos.sort_by(|a, b| a.datetime.cmp(&b.datetime));
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
                utxos.sort_by(|a, b| a.type_.cmp(&b.type_));
            }
        }
    }

    pub fn filter_utxos(&mut self, search: &str) {
        let utxos = &mut self.utxos;
        utxos.sort_unstable_by(|a, b| {
            let a = strsim::normalized_damerau_levenshtein(a.name(), search);
            let b = strsim::normalized_damerau_levenshtein(b.name(), search);
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

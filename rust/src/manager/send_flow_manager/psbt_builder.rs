use act_zero::call;
use cove_types::{amount::Amount, psbt::Psbt};
use tracing::debug;

use crate::{transaction::FeeRate, wallet::Address};

use super::{AmountOrMax, CoinControlMode, EnterMode, Error, Result, RustSendFlowManager};

impl RustSendFlowManager {
    pub(crate) async fn build_psbt(
        &self,
        address: Option<Address>,
        amount: Option<Amount>,
        fee_rate: FeeRate,
    ) -> Result<Psbt> {
        debug!("build_psbt");
        let mode = self.state.lock().mode.clone();

        match mode {
            EnterMode::SetAmount => self.build_psbt_for_amount(address, amount, fee_rate).await,
            EnterMode::CoinControl(coin_control) => {
                self.build_psbt_for_coin_control(coin_control, address, fee_rate).await
            }
        }
    }

    pub(crate) async fn build_psbt_for_amount(
        &self,
        address: Option<Address>,
        amount: Option<Amount>,
        fee_rate: FeeRate,
    ) -> Result<Psbt> {
        debug!("build_psbt_for_amount");

        let (amount, address) = {
            let state = self.state.lock();

            let amount_sats = amount
                .map(|amount| amount.to_sat())
                .or_else(|| state.amount_sats)
                .ok_or_else(|| Error::unable_to_build_txn("no amount"))?;

            let amount = if state.max_selected.is_some() {
                AmountOrMax::Max
            } else {
                AmountOrMax::Amount(Amount::from_sat(amount_sats).into())
            };

            let address = address
                .or_else(|| state.address.clone().map(|address| address.as_ref().clone()))
                .ok_or_else(|| Error::unable_to_build_txn("no address"))?;

            (amount, address)
        };

        let actor = self.wallet_actor();
        let psbt = match amount {
            AmountOrMax::Amount(amount) => {
                let amount = amount.as_ref().0;
                call!(actor.build_ephemeral_tx(amount, address, fee_rate)).await.unwrap()
            }

            AmountOrMax::Max => {
                call!(actor.build_ephemeral_drain_tx(address, fee_rate)).await.unwrap()
            }
        }?;

        Ok(psbt.into())
    }

    pub(crate) async fn build_psbt_for_coin_control(
        &self,
        coin_control: CoinControlMode,
        address: Option<Address>,
        fee_rate: FeeRate,
    ) -> Result<Psbt> {
        debug!("build_psbt_for_utxo_list");

        let (address, amount) = {
            let state = self.state.lock();

            let amount = if coin_control.is_max_selected {
                coin_control.max_send()
            } else {
                state.amount_sats.map_or_else(|| coin_control.max_send(), Amount::from_sat)
            };

            let address = address
                .or_else(|| state.address.clone().map(|address| address.as_ref().clone()))
                .ok_or_else(|| Error::unable_to_build_txn("no address"))?;

            (address, bitcoin::Amount::from(amount))
        };

        let outpoints = coin_control.outpoints();
        let actor = self.wallet_actor();
        let psbt =
            call!(actor.build_manual_ephemeral_tx(outpoints, amount, address, fee_rate)).await;

        let psbt = psbt.map_err(Error::unable_to_build_txn)??;
        Ok(psbt.into())
    }
}

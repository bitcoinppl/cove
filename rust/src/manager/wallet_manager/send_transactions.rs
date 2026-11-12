use std::sync::Arc;

use act_zero::call;
use tracing::debug;

use crate::{
    manager::wallet_manager::actor::fee_bump::FeeBumpTarget,
    transaction::{Amount, FeeRate, fee_bump::FeeBumpError},
    wallet::Address,
};
use cove_types::confirm::ConfirmDetails;
use cove_util::result_ext::ResultExt as _;

use super::{Error, RustWalletManager};

impl RustWalletManager {
    pub async fn confirm_txn(
        &self,
        amount: Amount,
        address: Arc<Address>,
        fee_rate: FeeRate,
    ) -> Result<ConfirmDetails, Error> {
        let actor = self.actor.clone();

        let amount = amount.into();
        let address = Arc::unwrap_or_clone(address);
        let fee_rate = fee_rate.into();

        let psbt = call!(actor.build_tx(amount, address, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;
        let details = call!(self.actor.get_confirm_details(psbt, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(details)
    }

    pub async fn confirm_manual_txn(
        &self,
        outpoints: Vec<bitcoin::OutPoint>,
        amount: Amount,
        address: Arc<Address>,
        fee_rate: FeeRate,
    ) -> Result<ConfirmDetails, Error> {
        debug!("confirm_manual_txn amount: {amount:?}  fee_rate: {:?}", fee_rate.sat_per_vb());
        let actor = self.actor.clone();

        let amount = amount.into();
        let fee_rate = fee_rate.into();
        let address = Arc::unwrap_or_clone(address);

        let psbt = call!(actor.build_manual_tx(outpoints, amount, address, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        let details = call!(self.actor.get_confirm_details(psbt, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;
        Ok(details)
    }

    /// Whether the pending transaction `txid` can be fee bumped, and its lowest replacement rate
    pub async fn fee_bump_target(
        &self,
        txid: bitcoin::Txid,
    ) -> Result<FeeBumpTarget, FeeBumpError> {
        self.ensure_ledger_ready_for_spend()?;
        call!(self.actor.fee_bump_target(txid)).await.map_err(|_| Error::ActorNotFound)?
    }

    /// What the replacement for `txid` would pay in total at `fee_rate`, without reserving
    /// its change address
    pub async fn fee_bump_total_fee(
        &self,
        txid: bitcoin::Txid,
        fee_rate: FeeRate,
    ) -> Result<Amount, FeeBumpError> {
        self.ensure_ledger_ready_for_spend()?;
        let psbt = call!(self.actor.build_ephemeral_fee_bump(txid, fee_rate.into()))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        let fee = psbt.fee().map_err_str(FeeBumpError::Build)?;
        Ok(fee.into())
    }

    /// Build the replacement for `txid` at `fee_rate`, ready to sign like any other send
    pub async fn confirm_fee_bump_txn(
        &self,
        txid: bitcoin::Txid,
        fee_rate: FeeRate,
    ) -> Result<ConfirmDetails, FeeBumpError> {
        debug!("confirm_fee_bump_txn {txid} fee_rate: {:?}", fee_rate.sat_per_vb());
        self.ensure_ledger_ready_for_spend()?;
        let fee_rate = fee_rate.into();

        let psbt = call!(self.actor.build_fee_bump(txid, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        let details = call!(self.actor.get_confirm_details(psbt, fee_rate))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(details)
    }
}

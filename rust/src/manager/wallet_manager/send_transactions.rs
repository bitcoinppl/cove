use std::sync::Arc;

use act_zero::call;
use tracing::debug;

use crate::{
    transaction::{Amount, FeeRate},
    wallet::Address,
};
use cove_types::confirm::ConfirmDetails;

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

        let psbt = call!(actor.build_tx(amount, address, fee_rate)).await.unwrap()?;
        let details = call!(self.actor.get_confirm_details(psbt, fee_rate)).await.unwrap()?;

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

        let psbt =
            call!(actor.build_manual_tx(outpoints, amount, address, fee_rate)).await.unwrap()?;

        let details = call!(self.actor.get_confirm_details(psbt, fee_rate)).await.unwrap()?;
        Ok(details)
    }
}

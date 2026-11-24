use std::sync::Arc;

use act_zero::call;
use tracing::debug;

use crate::{
    database::Database,
    manager::wallet_manager::payjoin::PayjoinFeeContribution,
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

    /// What a Payjoin payment of `details` offers the receiver toward its coin's fee, under the
    /// fee help setting, `None` when it offers nothing
    pub async fn payjoin_fee_contribution(
        &self,
        details: Arc<ConfirmDetails>,
    ) -> Result<Option<PayjoinFeeContribution>, Error> {
        let help = Database::global().global_config.selected_payjoin_fee_help();

        call!(self.actor.payjoin_fee_contribution(details, help))
            .await
            .map_err(|_| Error::ActorNotFound)
    }
}

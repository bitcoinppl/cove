use act_zero::{ActorResult, Produces, WeakAddr, call};
use bitcoin::{Transaction, Txid};

use crate::{manager::wallet_manager::Error, node::client::NodeClient};

use super::WalletActor;

#[derive(Debug)]
pub(crate) enum BroadcastTransactionError {
    BroadcastFailed(Error),
    PostBroadcastFailed(Error),
}

impl BroadcastTransactionError {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::BroadcastFailed(error) | Self::PostBroadcastFailed(error) => error,
        }
    }
}

impl WalletActor {
    async fn node_client_for_broadcast(&mut self) -> ActorResult<Result<NodeClient, Error>> {
        Produces::ok(self.node_client().cloned().map_err(|_| {
            Error::BroadcastError(
                "failed to broadcast transaction, could not get node client, try again".to_string(),
            )
        }))
    }
}

pub(crate) async fn broadcast_to_node_with_connection(
    addr: WeakAddr<WalletActor>,
    connection: Produces<Result<(), Error>>,
    transaction: &Transaction,
) -> Result<(), BroadcastTransactionError> {
    connection
        .await
        .map_err(|_| BroadcastTransactionError::BroadcastFailed(Error::ActorNotFound))?
        .map_err(|error| {
            BroadcastTransactionError::BroadcastFailed(Error::BroadcastError(format!(
                "failed to broadcast transaction, unable to connect to node: {error:?}"
            )))
        })?;

    let node_client = call!(addr.node_client_for_broadcast())
        .await
        .map_err(|_| BroadcastTransactionError::BroadcastFailed(Error::ActorNotFound))?
        .map_err(BroadcastTransactionError::BroadcastFailed)?;

    node_client.broadcast_transaction(transaction.clone()).await.map_err(|error| {
        BroadcastTransactionError::BroadcastFailed(Error::BroadcastError(format!(
            "failed to broadcast transaction, try again: {error:?}"
        )))
    })?;

    Ok(())
}

pub(crate) async fn transaction_known_to_node(addr: WeakAddr<WalletActor>, txid: Txid) -> bool {
    let node_client = match call!(addr.node_client_for_broadcast()).await {
        Ok(Ok(node_client)) => node_client,
        _ => return false,
    };

    let response = node_client.get_transaction(txid).await;
    matches!(response, Ok(Some(ref found)) if found.compute_txid() == txid)
}

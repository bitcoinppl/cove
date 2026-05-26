//! Watches the cached receive address so payment activity can rotate future receive requests to a
//! fresh address before the next regular wallet sync

use std::{sync::Arc, time::Duration};

use act_zero::{Actor, ActorError, ActorResult, Addr, AddrLike, Produces, WeakAddr, call, send};
use bdk_wallet::chain::bitcoin::Address;
use tracing::{debug, error, warn};

use crate::{manager::wallet_manager::actor::WalletActor, node::client_builder::NodeClientBuilder};

pub const RECEIVE_ADDRESS_WATCH_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct ReceiveAddressWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    request_id: u64,
    derivation_index: u32,
    address: Arc<Address>,
    client_builder: NodeClientBuilder,
    poll_interval: Duration,
    running: bool,
}

#[async_trait::async_trait]
impl Actor for ReceiveAddressWatcher {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        send!(self.addr.start_watching());

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("ReceiveAddressWatcher Error: {error:?}");
        false
    }
}

impl ReceiveAddressWatcher {
    pub fn new(
        wallet_actor: WeakAddr<WalletActor>,
        request_id: u64,
        derivation_index: u32,
        address: Address,
        client_builder: NodeClientBuilder,
    ) -> Self {
        debug!("creating receive address watcher for index={derivation_index}");

        Self {
            wallet_actor,
            addr: Default::default(),
            request_id,
            derivation_index,
            address: Arc::new(address),
            client_builder,
            poll_interval: RECEIVE_ADDRESS_WATCH_INTERVAL,
            running: false,
        }
    }

    pub async fn start_watching(&mut self) -> ActorResult<()> {
        self.running = true;

        let address = self.address.clone();
        let client_builder = self.client_builder.clone();
        let derivation_index = self.derivation_index;
        let manager = self.wallet_actor.clone();
        let poll_interval = self.poll_interval;
        let request_id = self.request_id;

        self.addr.send_fut_with(|addr| async move {
            loop {
                tokio::time::sleep(poll_interval).await;

                match call!(addr.is_running()).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }

                let result = match client_builder.build().await {
                    Ok(client) => client.check_address_for_txn((*address).clone()).await,
                    Err(error) => Err(error),
                };

                match result {
                    Ok(true) => {
                        send!(addr.stop_watching());
                        send!(
                            manager.handle_receive_address_watch_activity(
                                request_id,
                                derivation_index
                            )
                        );
                        break;
                    }

                    Ok(false) => {
                        debug!("receive address index={derivation_index} has no activity yet");
                    }

                    Err(error) => {
                        warn!("Failed to watch receive address index={derivation_index}: {error}");
                    }
                }
            }
        });

        Produces::ok(())
    }

    pub async fn stop_watching(&mut self) -> ActorResult<()> {
        self.running = false;
        Produces::ok(())
    }

    async fn is_running(&self) -> ActorResult<bool> {
        Produces::ok(self.running)
    }
}

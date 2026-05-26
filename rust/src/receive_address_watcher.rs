//! Watches the cached receive address so payment activity can rotate future receive requests to a
//! fresh address before the next regular wallet sync

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use act_zero::{
    Actor, ActorError, ActorResult, Addr, Produces, WeakAddr, runtimes::tokio::Timer, send,
    timer::Tick,
};
use bdk_wallet::chain::bitcoin::Address;
use tracing::{debug, error, trace, warn};

use crate::{manager::wallet_manager::actor::WalletActor, node::client_builder::NodeClientBuilder};

pub const RECEIVE_ADDRESS_WATCH_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub struct ReceiveAddressWatcher {
    wallet_actor: WeakAddr<WalletActor>,
    addr: WeakAddr<Self>,
    request_id: u64,
    derivation_index: u32,
    address: Arc<Address>,
    client_builder: NodeClientBuilder,
    poll_interval: Duration,
    watch_duration: Duration,
    poll_timer: Timer,
    expiry_timer: Timer,
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
        watch_duration: Duration,
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
            watch_duration,
            poll_timer: Timer::default(),
            expiry_timer: Timer::default(),
        }
    }

    pub async fn start_watching(&mut self) -> ActorResult<()> {
        self.poll_timer.set_interval_at_weak(
            self.addr.clone(),
            Instant::now() + self.poll_interval,
            self.poll_interval,
        );

        self.expiry_timer.set_timeout_for_weak(self.addr.clone(), self.watch_duration);

        Produces::ok(())
    }

    pub async fn stop_watching(&mut self) -> ActorResult<()> {
        self.poll_timer.clear();
        self.expiry_timer.clear();
        send!(self.wallet_actor.handle_receive_address_watcher_stopped(self.request_id));

        Produces::ok(())
    }
}

#[async_trait::async_trait]
impl Tick for ReceiveAddressWatcher {
    async fn tick(&mut self) -> ActorResult<()> {
        if self.expiry_timer.tick() {
            return self.stop_watching().await;
        }

        if !self.poll_timer.tick() {
            return Produces::ok(());
        }

        let result = match self.client_builder.build().await {
            Ok(client) => client.check_address_for_txn((*self.address).clone()).await,
            Err(error) => Err(error),
        };

        match result {
            // found activity on that address
            Ok(true) => {
                self.stop_watching().await?;
                send!(
                    self.wallet_actor.handle_receive_address_watch_activity(
                        self.request_id,
                        self.derivation_index
                    )
                );
            }

            // no activity yet
            Ok(false) => {
                trace!("receive address index={} has no activity yet", self.derivation_index);
            }

            Err(error) => {
                warn!("Failed to watch receive address index={}: {error}", self.derivation_index);
            }
        }

        Produces::ok(())
    }
}

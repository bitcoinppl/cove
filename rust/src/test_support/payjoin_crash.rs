//! Stops a simulated app process right after any durable step of a Payjoin send, so a test can
//! rebuild the wallet from what that step left behind, as a crash and relaunch would (#1004)
//!
//! A run's durable steps are the writes to its Payjoin session store and the broadcasts the fake
//! node accepts. A [`CrashSwitch`] counts them across both, and once the chosen step is done the
//! process is treated as dead: later store writes fail without touching the database, and every
//! later request to the node fails, so nothing the stopped process still runs can outlive it

use std::sync::Arc;

use cove_fake_node::{LoggedRequest, Protocol, RuleHandle, Scripted};
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::{
    database::wallet_data::{PayjoinSenderSession, WalletDataDb, WalletDataError},
    manager::wallet_manager::payjoin::PayjoinSessionStore,
    test_support::fake_node::FakeNode,
    wallet_lifecycle::TerminalPayjoinPersistenceAuthority,
};

/// One step of a run that outlives its process
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DurableStep {
    /// The Payjoin session store committed a write
    StoreWrite,
    /// The node accepted a broadcast, and its answer has not reached the wallet yet
    BroadcastAccepted,
    /// The wallet got the node's answer to a broadcast, and saves the transaction next
    ///
    /// A run stopped here is not over until the wallet has saved the transaction, which shows
    /// as its next write to the session record being refused
    BroadcastAnswered,
}

/// Stops a run right after its `stop_after`th durable step, counting from one
#[derive(Clone)]
pub(crate) struct CrashSwitch(Arc<SwitchState>);

struct SwitchState {
    stop_after: usize,
    steps: Mutex<Vec<DurableStep>>,
    stopped: watch::Sender<bool>,
    refused_writes: watch::Sender<usize>,
}

impl CrashSwitch {
    pub(crate) fn after(stop_after: usize) -> Self {
        Self(Arc::new(SwitchState {
            stop_after,
            steps: Mutex::new(Vec::new()),
            stopped: watch::channel(false).0,
            refused_writes: watch::channel(0).0,
        }))
    }

    /// `db`'s Payjoin session record, whose writes count as steps and fail once stopped
    pub(crate) fn store(&self, db: WalletDataDb) -> Arc<dyn PayjoinSessionStore> {
        Arc::new(CrashingStore { db, switch: self.clone() })
    }

    /// Counts the node's broadcasts as steps, and fails every request once stopped
    ///
    /// The rule matches every request, so drop it with `clear_rules` before the relaunched
    /// wallet uses the node
    pub(crate) fn install_on(&self, node: &FakeNode) -> RuleHandle {
        let switch = self.clone();
        node.on("").script(move |request| switch.node_request(request))
    }

    /// The steps the run took, oldest first
    pub(crate) fn steps(&self) -> Vec<DurableStep> {
        self.0.steps.lock().clone()
    }

    pub(crate) fn has_stopped(&self) -> bool {
        *self.0.stopped.borrow()
    }

    /// Waits until the run has taken its last step before the crash
    pub(crate) async fn until_stopped(&self) {
        let mut stopped = self.0.stopped.subscribe();
        let _ = stopped.wait_for(|stopped| *stopped).await;

        // the wallet saves an answered broadcast after the node's answer arrives, so the
        // process lives on until the write that follows that save
        if self.steps().last() == Some(&DurableStep::BroadcastAnswered) {
            let mut refused_writes = self.0.refused_writes.subscribe();
            let _ = refused_writes.wait_for(|refused| *refused > 0).await;
        }
    }

    /// Records `step`, stopping the process when it is the last one
    fn take_step(&self, steps: &mut Vec<DurableStep>, step: DurableStep) {
        steps.push(step);
        if steps.len() == self.0.stop_after {
            self.0.stopped.send_replace(true);
        }
    }

    fn node_request(&self, request: &LoggedRequest) -> Scripted {
        let mut steps = self.0.steps.lock();
        if self.has_stopped() {
            return Scripted::Fail { status: 503, message: "the app process stopped".into() };
        }

        // only a broadcast POST has the bare `/tx` path, a lookup adds the txid
        if request.protocol != Protocol::Esplora || request.name != "/tx" {
            return Scripted::Serve;
        }

        self.take_step(&mut steps, DurableStep::BroadcastAccepted);
        if self.has_stopped() {
            return Scripted::ServeThenStall;
        }

        self.take_step(&mut steps, DurableStep::BroadcastAnswered);
        Scripted::Serve
    }

    fn write(
        &self,
        write: impl FnOnce() -> Result<(), WalletDataError>,
    ) -> Result<(), WalletDataError> {
        let mut steps = self.0.steps.lock();
        if self.has_stopped() {
            self.0.refused_writes.send_modify(|refused| *refused += 1);
            return Err(WalletDataError::Save("the app process stopped".into()));
        }

        write()?;
        self.take_step(&mut steps, DurableStep::StoreWrite);
        Ok(())
    }
}

/// A wallet's Payjoin session record whose writes are steps of a [`CrashSwitch`]
#[derive(Debug)]
struct CrashingStore {
    db: WalletDataDb,
    switch: CrashSwitch,
}

impl std::fmt::Debug for CrashSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrashSwitch").field("stop_after", &self.0.stop_after).finish()
    }
}

impl PayjoinSessionStore for CrashingStore {
    fn session(&self) -> Result<Option<PayjoinSenderSession>, WalletDataError> {
        PayjoinSessionStore::session(&self.db)
    }

    fn save(&self, session: PayjoinSenderSession) -> Result<(), WalletDataError> {
        self.switch.write(|| PayjoinSessionStore::save(&self.db, session))
    }

    fn save_terminal_fallback(
        &self,
        session: PayjoinSenderSession,
        authority: &TerminalPayjoinPersistenceAuthority,
    ) -> Result<(), WalletDataError> {
        self.switch
            .write(|| PayjoinSessionStore::save_terminal_fallback(&self.db, session, authority))
    }

    fn delete(&self) -> Result<(), WalletDataError> {
        self.switch.write(|| PayjoinSessionStore::delete(&self.db))
    }
}

/// A wallet's Payjoin session record whose next write fails once without touching the
/// database, as a transient storage error would
#[derive(Debug)]
pub(crate) struct FlakyStore {
    db: WalletDataDb,
    fail_next_write: Mutex<bool>,
}

impl FlakyStore {
    pub(crate) fn failing_once(db: WalletDataDb) -> Arc<Self> {
        Arc::new(Self { db, fail_next_write: Mutex::new(true) })
    }

    fn write(
        &self,
        write: impl FnOnce() -> Result<(), WalletDataError>,
    ) -> Result<(), WalletDataError> {
        if std::mem::take(&mut *self.fail_next_write.lock()) {
            return Err(WalletDataError::Save("the store failed this write once".into()));
        }

        write()
    }
}

impl PayjoinSessionStore for FlakyStore {
    fn session(&self) -> Result<Option<PayjoinSenderSession>, WalletDataError> {
        PayjoinSessionStore::session(&self.db)
    }

    fn save(&self, session: PayjoinSenderSession) -> Result<(), WalletDataError> {
        self.write(|| PayjoinSessionStore::save(&self.db, session))
    }

    fn save_terminal_fallback(
        &self,
        session: PayjoinSenderSession,
        authority: &TerminalPayjoinPersistenceAuthority,
    ) -> Result<(), WalletDataError> {
        self.write(|| PayjoinSessionStore::save_terminal_fallback(&self.db, session, authority))
    }

    fn delete(&self) -> Result<(), WalletDataError> {
        self.write(|| PayjoinSessionStore::delete(&self.db))
    }
}

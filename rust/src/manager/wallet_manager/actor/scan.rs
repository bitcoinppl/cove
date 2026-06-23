use std::{collections::BTreeMap, time::Duration};

use act_zero::*;
use bdk_wallet::{
    KeychainKind,
    chain::{
        TxGraph,
        spk_client::{FullScanRequest, FullScanResponse},
    },
};
use cove_bdk_progressive_scan::{KeychainProgress, ScanEvent, ScanProgress, ScanUpdate};
use cove_common::consts::GAP_LIMIT;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{
    manager::wallet_manager::{WalletScanPhase, WalletScanProgress, WalletScanStatus},
    node::client::{Error as NodeClientError, NodeClient},
};

use super::{WalletActor, WalletScanGeneration};

const SCAN_PROGRESS_INTERVAL: Duration = Duration::from_millis(75);
const SCAN_PARTIAL_FLUSH_INTERVAL: Duration = Duration::from_millis(300);
const SCAN_PROGRESS_BASIS_POINTS: u32 = 10_000;
const FULL_SCAN_STOP_GAP: usize = 150;

pub(crate) const RETURNING_WALLET_SCAN_PROGRESS_DELAY: Duration = Duration::from_secs(5);
pub(crate) const EMPTY_WALLET_SCAN_PROGRESS_DELAY: Duration = RETURNING_WALLET_SCAN_PROGRESS_DELAY;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum FullScanType {
    /// Full scan uses a 150-address stop gap
    Full,
    Rescan(u32),
}

impl FullScanType {
    /// Returns the address gap that stops this scan type
    pub(crate) const fn stop_gap(&self) -> usize {
        match self {
            Self::Full => FULL_SCAN_STOP_GAP,
            Self::Rescan(gap) => *gap as usize,
        }
    }

    /// Returns the UI phase represented by this full scan type
    pub(crate) const fn phase(&self) -> WalletScanPhase {
        match self {
            Self::Full => WalletScanPhase::Full,
            Self::Rescan(_) => WalletScanPhase::Rescan,
        }
    }
}

pub(crate) struct PreparedProgressiveScan {
    pub(crate) node_client: NodeClient,
    pub(crate) graph: TxGraph,
    pub(crate) full_scan_request: FullScanRequest<KeychainKind>,
    pub(crate) last_revealed_indices: BTreeMap<KeychainKind, u32>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ScanRequestOrder {
    Standard,
    ReceivePriority,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ScanProgressStart {
    Immediate,
    Delayed(Duration),
}

pub(crate) struct WalletScanEvent {
    generation: WalletScanGeneration,
    kind: WalletScanEventKind,
}

impl WalletScanEvent {
    pub(crate) const fn new(generation: WalletScanGeneration, kind: WalletScanEventKind) -> Self {
        Self { generation, kind }
    }

    pub(crate) const fn generation(&self) -> WalletScanGeneration {
        self.generation
    }

    pub(crate) fn into_kind(self) -> WalletScanEventKind {
        self.kind
    }
}

pub(crate) enum WalletScanEventKind {
    FullScanStarted(FullScanType),
    IncrementalScanStarted,
    FullScanPrepareFailed(FullScanType),
    IncrementalScanPrepareFailed,
    StatusChanged(WalletScanStatus),
    PartialUpdate(ScanUpdate<KeychainKind>),
    FlushUi,
    FullScanFinished {
        scan_type: FullScanType,
        result: Result<FullScanResponse<KeychainKind>, NodeClientError>,
    },
    IncrementalScanFinished {
        result: Result<FullScanResponse<KeychainKind>, NodeClientError>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RunningScan {
    Full(FullScanType),
    Incremental,
}

impl RunningScan {
    const fn phase(self) -> WalletScanPhase {
        match self {
            Self::Full(scan_type) => scan_type.phase(),
            Self::Incremental => WalletScanPhase::Incremental,
        }
    }

    const fn stop_gap(self) -> usize {
        match self {
            Self::Full(scan_type) => scan_type.stop_gap(),
            Self::Incremental => GAP_LIMIT as usize,
        }
    }

    const fn request_order(self) -> ScanRequestOrder {
        match self {
            Self::Full(_) => ScanRequestOrder::Standard,
            Self::Incremental => ScanRequestOrder::ReceivePriority,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ScanActorGeneration(u64);

impl ScanActorGeneration {
    const INITIAL: Self = Self(0);

    const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

struct ProgressiveFullScanJob {
    scan: RunningScan,
    scan_generation: ScanActorGeneration,
    wallet_generation: WalletScanGeneration,
    cancel_token: CancellationToken,
    prepared: PreparedProgressiveScan,
    progress_reveal_at: tokio::time::Instant,
}

struct ProgressiveFullScanResult {
    scan: RunningScan,
    scan_generation: ScanActorGeneration,
    wallet_generation: WalletScanGeneration,
    result: Result<FullScanResponse<KeychainKind>, NodeClientError>,
}

impl ProgressiveFullScanResult {
    /// Returns whether the scan completed with a usable node response
    fn scan_succeeded(&self) -> bool {
        self.result.is_ok()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ScanFlushDecision {
    Immediate,
    Debounce(tokio::time::Instant),
}

#[derive(Debug, Default)]
struct ScanFlushCadence {
    sent_first_flush: bool,
}

impl ScanFlushCadence {
    /// Chooses whether a partial wallet update should flush immediately or be debounced
    fn record_update(&mut self, now: tokio::time::Instant) -> ScanFlushDecision {
        if self.sent_first_flush {
            ScanFlushDecision::Debounce(now + SCAN_PARTIAL_FLUSH_INTERVAL)
        } else {
            self.sent_first_flush = true;
            ScanFlushDecision::Immediate
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct QueuedRescan {
    gap_limit: u32,
    wallet_generation: WalletScanGeneration,
}

pub(crate) struct WalletScanActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    progress: BTreeMap<KeychainKind, KeychainProgress>,
    active_cancel_token: Option<CancellationToken>,
    active_generation: Option<ScanActorGeneration>,
    active_wallet_generation: Option<WalletScanGeneration>,
    next_generation: ScanActorGeneration,
    queued_rescan: Option<QueuedRescan>,
    max_progress_basis_points: u32,
    progress_is_visible: bool,
}

impl WalletScanActor {
    /// Creates a scan actor that reports wallet scan events back to the wallet actor
    pub(crate) fn new(wallet_addr: WeakAddr<WalletActor>) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            progress: BTreeMap::default(),
            active_cancel_token: None,
            active_generation: None,
            active_wallet_generation: None,
            next_generation: ScanActorGeneration::INITIAL,
            queued_rescan: None,
            max_progress_basis_points: 0,
            progress_is_visible: false,
        }
    }

    /// Starts a full wallet scan unless another scan is already active
    pub(crate) async fn start_full_scan(
        &mut self,
        wallet_generation: WalletScanGeneration,
        progress_start: ScanProgressStart,
    ) -> ActorResult<()> {
        if self.scan_in_progress() {
            debug!("scan already in progress, skipping full scan");
            return Produces::ok(());
        }

        self.begin_scan(RunningScan::Full(FullScanType::Full), progress_start, wallet_generation)
    }

    /// Starts an incremental scan unless another scan is already active
    pub(crate) async fn start_incremental_scan(
        &mut self,
        wallet_generation: WalletScanGeneration,
        progress_start: ScanProgressStart,
    ) -> ActorResult<()> {
        if self.scan_in_progress() {
            debug!("scan already in progress, skipping incremental scan");
            return Produces::ok(());
        }

        self.begin_scan(RunningScan::Incremental, progress_start, wallet_generation)
    }

    /// Starts or queues a rescan with the requested gap limit
    pub(crate) async fn start_rescan(
        &mut self,
        gap_limit: u32,
        wallet_generation: WalletScanGeneration,
    ) -> ActorResult<()> {
        if self.scan_in_progress() {
            debug!("scan already in progress, queueing rescan gap_limit={gap_limit}");
            self.queued_rescan = Some(QueuedRescan { gap_limit, wallet_generation });
            return Produces::ok(());
        }

        self.begin_scan(
            RunningScan::Full(FullScanType::Rescan(gap_limit)),
            ScanProgressStart::Immediate,
            wallet_generation,
        )
    }

    /// Cancels active scan work and reports an idle scan state
    pub(crate) async fn shutdown(
        &mut self,
        wallet_generation: WalletScanGeneration,
    ) -> ActorResult<()> {
        self.clear_scan_lifecycle();
        self.send_event(
            wallet_generation,
            WalletScanEventKind::StatusChanged(WalletScanStatus::Idle),
        );
        Produces::ok(())
    }

    fn scan_in_progress(&self) -> bool {
        self.active_generation.is_some()
    }

    /// Initializes scan state and asks the wallet actor to prepare the scan request
    fn begin_scan(
        &mut self,
        scan: RunningScan,
        progress_start: ScanProgressStart,
        wallet_generation: WalletScanGeneration,
    ) -> ActorResult<()> {
        self.progress.clear();
        self.max_progress_basis_points = 0;
        self.progress_is_visible = matches!(progress_start, ScanProgressStart::Immediate);
        let now = tokio::time::Instant::now();
        let progress_reveal_at = match progress_start {
            ScanProgressStart::Immediate => now,
            ScanProgressStart::Delayed(delay) => now + delay,
        };

        let (scan_generation, cancel_token) = self.start_scan_generation(wallet_generation);
        match scan {
            RunningScan::Full(scan_type) => {
                self.send_event(wallet_generation, WalletScanEventKind::FullScanStarted(scan_type));
            }
            RunningScan::Incremental => {
                self.send_event(wallet_generation, WalletScanEventKind::IncrementalScanStarted);
            }
        }

        self.send_event(
            wallet_generation,
            WalletScanEventKind::StatusChanged(initial_scan_status(scan, progress_start)),
        );

        let request_order = scan.request_order();
        let addr = self.addr.clone();
        let wallet_addr = self.wallet_addr.clone();
        self.addr.send_fut(async move {
            match call!(wallet_addr.prepare_progressive_scan(request_order, wallet_generation))
                .await
            {
                Ok(Some(prepared)) => {
                    let job = ProgressiveFullScanJob {
                        scan,
                        scan_generation,
                        wallet_generation,
                        cancel_token,
                        prepared,
                        progress_reveal_at,
                    };
                    send!(addr.run_progressive_scan_job(job));
                }
                Ok(None) => {
                    send!(addr.handle_stale_scan_prepare(scan_generation));
                }
                Err(error) => {
                    debug!("failed to prepare progressive scan: {error:?}");
                    send!(addr.handle_scan_prepare_failed(
                        scan_generation,
                        wallet_generation,
                        scan
                    ));
                }
            }
        });

        Produces::ok(())
    }

    /// Runs a prepared scan job when its generation is still current
    async fn run_progressive_scan_job(&mut self, job: ProgressiveFullScanJob) -> ActorResult<()> {
        if !should_accept_scan_generation(self.active_generation, job.scan_generation) {
            debug!("ignoring stale progressive scan job");
            return Produces::ok(());
        }

        let start = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        let scan_result = self
            .run_progressive_full_scan(job)
            .await?
            .await
            .map_err(|error| Box::new(error) as ActorError)?;

        let now = std::time::UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done progressive scan in {}s", now - start);

        self.handle_scan_result(scan_result).await
    }

    /// Drives the node scan future while forwarding progress and debounced updates
    async fn run_progressive_full_scan(
        &mut self,
        job: ProgressiveFullScanJob,
    ) -> ActorResult<ProgressiveFullScanResult> {
        let (events_tx, events_rx) = flume::bounded(32);
        let scan = job.scan;
        let scan_generation = job.scan_generation;
        let wallet_generation = job.wallet_generation;
        let cancel_token = job.cancel_token.clone();

        self.seed_scan_progress(job.prepared.full_scan_request.keychains(), scan.stop_gap() as u32);

        let scan_future = job.prepared.node_client.start_progressive_wallet_scan(
            &job.prepared.graph,
            job.prepared.full_scan_request,
            job.prepared.last_revealed_indices,
            scan.stop_gap(),
            events_tx,
            cancel_token.clone(),
        );

        let mut flush_cadence = ScanFlushCadence::default();
        let mut last_progress_sent = Option::<tokio::time::Instant>::None;
        let mut flush_timer_armed = false;

        let flush_timer = tokio::time::sleep(SCAN_PARTIAL_FLUSH_INTERVAL);
        let progress_reveal_timer = tokio::time::sleep_until(job.progress_reveal_at);

        tokio::pin!(scan_future);
        tokio::pin!(flush_timer);
        tokio::pin!(progress_reveal_timer);

        let scan_result = loop {
            tokio::select! {
                // scan finished before another progress, flush, or reveal event
                result = &mut scan_future => break result,

                // scan emitted progress or a partial wallet update
                event = events_rx.recv_async() => {
                    let should_flush = match event {
                        Ok(event) => self.forward_scan_event(
                            event,
                            scan_generation,
                            wallet_generation,
                            scan.phase(),
                            &mut last_progress_sent,
                        ),
                        Err(_) => false,
                    };

                    if !should_flush {
                        continue;
                    }

                    match flush_cadence.record_update(tokio::time::Instant::now()) {
                        ScanFlushDecision::Immediate => {
                            self.send_event(wallet_generation, WalletScanEventKind::FlushUi);
                        }

                        ScanFlushDecision::Debounce(deadline) => {
                            flush_timer.as_mut().reset(deadline);
                            flush_timer_armed = true;
                        }
                    }
                }

                // debounced partial updates are ready to be sent to the UI
                _ = &mut flush_timer, if flush_timer_armed => {
                    flush_timer_armed = false;
                    self.send_event(wallet_generation, WalletScanEventKind::FlushUi);
                }

                // delayed scans switch from the lightweight spinner to detailed progress
                _ = &mut progress_reveal_timer, if !self.progress_is_visible => {
                    self.reveal_delayed_progress(scan_generation, wallet_generation, scan);
                }
            }
        };

        while let Ok(event) = events_rx.try_recv() {
            let scan_phase = scan.phase();
            let should_flush = self.forward_scan_event(
                event,
                scan_generation,
                wallet_generation,
                scan_phase,
                &mut last_progress_sent,
            );

            if should_flush {
                let now = tokio::time::Instant::now();
                let flush_decision = flush_cadence.record_update(now);

                match flush_decision {
                    ScanFlushDecision::Immediate => {
                        self.send_event(wallet_generation, WalletScanEventKind::FlushUi);
                    }
                    ScanFlushDecision::Debounce(deadline) => {
                        flush_timer.as_mut().reset(deadline);
                        flush_timer_armed = true;
                    }
                }
            }
        }

        if flush_timer_armed && should_flush_pending_after_scan_result(&scan_result, &cancel_token)
        {
            self.send_event(wallet_generation, WalletScanEventKind::FlushUi);
        }

        Produces::ok(ProgressiveFullScanResult {
            scan,
            scan_generation,
            wallet_generation,
            result: scan_result,
        })
    }

    /// Applies a finished scan result if it still belongs to the active generation
    async fn handle_scan_result(
        &mut self,
        scan_result: ProgressiveFullScanResult,
    ) -> ActorResult<()> {
        if !should_accept_scan_generation(self.active_generation, scan_result.scan_generation) {
            debug!("ignoring stale progressive scan result");
            return Produces::ok(());
        }

        let cancel_token = self.active_cancel_token.as_ref();
        if let Err(error) = &scan_result.result
            && is_cancelled_progressive_scan(error, cancel_token)
        {
            self.clear_scan_lifecycle();
            self.send_event(
                scan_result.wallet_generation,
                WalletScanEventKind::StatusChanged(WalletScanStatus::Idle),
            );
            return Produces::ok(());
        }

        self.clear_active_scan_work();

        match scan_result.scan {
            RunningScan::Full(scan_type) => {
                let scan_succeeded = scan_result.scan_succeeded();
                let apply_result =
                    call!(self.wallet_addr.handle_wallet_scan_event(WalletScanEvent::new(
                        scan_result.wallet_generation,
                        WalletScanEventKind::FullScanFinished {
                            scan_type,
                            result: scan_result.result,
                        },
                    )))
                    .await;

                if scan_succeeded && let Err(error) = apply_result {
                    return Err(Box::new(error));
                }

                self.active_wallet_generation = None;
                self.start_queued_rescan().await?;
            }
            RunningScan::Incremental => {
                let scan_succeeded = scan_result.scan_succeeded();
                let apply_result =
                    call!(self.wallet_addr.handle_wallet_scan_event(WalletScanEvent::new(
                        scan_result.wallet_generation,
                        WalletScanEventKind::IncrementalScanFinished { result: scan_result.result },
                    )))
                    .await;

                if !scan_succeeded {
                    self.active_wallet_generation = None;
                    self.start_queued_rescan().await?;

                    return Produces::ok(());
                }

                if let Err(error) = apply_result {
                    return Err(Box::new(error));
                }

                self.active_wallet_generation = None;
                self.start_queued_rescan().await?;
            }
        }

        Produces::ok(())
    }

    /// Starts a queued rescan after the active scan has finished applying
    async fn start_queued_rescan(&mut self) -> ActorResult<()> {
        let Some(queued_rescan) = self.queued_rescan.take() else {
            return Produces::ok(());
        };

        self.begin_scan(
            RunningScan::Full(FullScanType::Rescan(queued_rescan.gap_limit)),
            ScanProgressStart::Immediate,
            queued_rescan.wallet_generation,
        )
    }

    /// Reveals detailed progress for delayed scans that are still active
    fn reveal_delayed_progress(
        &mut self,
        scan_generation: ScanActorGeneration,
        wallet_generation: WalletScanGeneration,
        scan: RunningScan,
    ) {
        if !should_reveal_delayed_progress(
            self.active_generation,
            scan_generation,
            self.progress_is_visible,
        ) {
            return;
        }

        self.progress_is_visible = true;
        let status = self
            .scan_status_from_current_progress(scan.phase())
            .unwrap_or_else(|| initial_progress_status(scan));
        self.send_event(wallet_generation, WalletScanEventKind::StatusChanged(status));
    }

    /// Clears a scan whose wallet generation became stale before preparation finished
    async fn handle_stale_scan_prepare(
        &mut self,
        scan_generation: ScanActorGeneration,
    ) -> ActorResult<()> {
        if should_accept_scan_generation(self.active_generation, scan_generation) {
            // address type switches already emitted idle when resetting the scan lifecycle
            self.clear_active_scan();
        }

        Produces::ok(())
    }

    /// Reports scan preparation failure when the failed generation is still active
    async fn handle_scan_prepare_failed(
        &mut self,
        scan_generation: ScanActorGeneration,
        wallet_generation: WalletScanGeneration,
        scan: RunningScan,
    ) -> ActorResult<()> {
        if !should_accept_scan_generation(self.active_generation, scan_generation) {
            return Produces::ok(());
        }

        self.clear_scan_lifecycle();

        match scan {
            RunningScan::Full(scan_type) => {
                self.send_event(
                    wallet_generation,
                    WalletScanEventKind::FullScanPrepareFailed(scan_type),
                );
            }
            RunningScan::Incremental => {
                self.send_event(
                    wallet_generation,
                    WalletScanEventKind::IncrementalScanPrepareFailed,
                );
            }
        }

        self.send_event(
            wallet_generation,
            WalletScanEventKind::StatusChanged(WalletScanStatus::Idle),
        );
        Produces::ok(())
    }

    /// Forwards node scan events and returns whether the UI needs a partial flush
    fn forward_scan_event(
        &mut self,
        event: ScanEvent<KeychainKind>,
        scan_generation: ScanActorGeneration,
        wallet_generation: WalletScanGeneration,
        phase: WalletScanPhase,
        last_progress_sent: &mut Option<tokio::time::Instant>,
    ) -> bool {
        if !should_accept_scan_generation(self.active_generation, scan_generation) {
            return false;
        }

        match event {
            ScanEvent::Progress(progress) => {
                if should_forward_scan_progress(last_progress_sent, tokio::time::Instant::now()) {
                    let status = self.scan_status_from_progress(progress, phase);
                    if self.progress_is_visible {
                        self.send_event(
                            wallet_generation,
                            WalletScanEventKind::StatusChanged(status),
                        );
                    }
                }
                false
            }
            ScanEvent::Update(update) => {
                self.send_event(wallet_generation, WalletScanEventKind::PartialUpdate(update));
                true
            }
            ScanEvent::Complete(_) => false,
        }
    }

    /// Merges one keychain progress event into the aggregate UI scan status
    fn scan_status_from_progress(
        &mut self,
        progress: ScanProgress<KeychainKind>,
        phase: WalletScanPhase,
    ) -> WalletScanStatus {
        self.progress.insert(
            progress.keychain,
            KeychainProgress {
                checked: progress.checked,
                gap: progress.gap,
                stop_gap: progress.stop_gap,
            },
        );

        let total = total_keychain_progress(self.progress.values().copied()).unwrap_or_default();
        let progress_basis_points = scan_progress_basis_points(total);
        let progress_basis_points = self.record_visible_progress(progress_basis_points);

        WalletScanStatus::Scanning(WalletScanProgress {
            phase,
            checked: total.checked,
            gap: total.gap,
            stop_gap: total.stop_gap,
            progress_basis_points,
        })
    }

    /// Records the highest visible progress value for the current scan
    fn record_visible_progress(&mut self, progress_basis_points: u32) -> u32 {
        self.max_progress_basis_points = self.max_progress_basis_points.max(progress_basis_points);
        self.max_progress_basis_points
    }

    /// Builds the current visible scan status from accumulated keychain progress
    fn scan_status_from_current_progress(
        &self,
        phase: WalletScanPhase,
    ) -> Option<WalletScanStatus> {
        let total = total_keychain_progress(self.progress.values().copied())?;

        Some(WalletScanStatus::Scanning(WalletScanProgress {
            phase,
            checked: total.checked,
            gap: total.gap,
            stop_gap: total.stop_gap,
            progress_basis_points: self.max_progress_basis_points,
        }))
    }

    /// Starts a new scan generation and cancels any previous active generation
    fn start_scan_generation(
        &mut self,
        wallet_generation: WalletScanGeneration,
    ) -> (ScanActorGeneration, CancellationToken) {
        if let Some(cancel_token) = self.active_cancel_token.take() {
            cancel_token.cancel();
        }

        let cancel_token = CancellationToken::new();
        let scan_generation = self.next_generation;
        self.next_generation = self.next_generation.next();
        self.active_cancel_token = Some(cancel_token.clone());
        self.active_generation = Some(scan_generation);
        self.active_wallet_generation = Some(wallet_generation);
        (scan_generation, cancel_token)
    }

    /// Clears the active scan generation after a scan finishes normally
    fn clear_active_scan(&mut self) {
        self.clear_active_scan_work();
        self.active_wallet_generation = None;
    }

    /// Clears completed scan work while keeping the wallet generation for error cleanup
    fn clear_active_scan_work(&mut self) {
        self.active_cancel_token = None;
        self.active_generation = None;
        self.progress_is_visible = false;
    }

    /// Cancels active scan work and resets queued scan lifecycle state
    fn clear_scan_lifecycle(&mut self) {
        if let Some(cancel_token) = self.active_cancel_token.take() {
            cancel_token.cancel();
        }
        self.active_generation = None;
        self.active_wallet_generation = None;
        self.queued_rescan = None;
        self.progress.clear();
        self.max_progress_basis_points = 0;
        self.progress_is_visible = false;
    }

    /// Seeds per-keychain progress so the UI can show aggregate scan work immediately
    fn seed_scan_progress(
        &mut self,
        keychains: impl IntoIterator<Item = KeychainKind>,
        stop_gap: u32,
    ) {
        self.progress = keychains
            .into_iter()
            .map(|keychain| (keychain, KeychainProgress { checked: 0, gap: 0, stop_gap }))
            .collect();
    }

    /// Sends a scan event to the wallet actor without waiting for it to apply
    fn send_event(&self, generation: WalletScanGeneration, kind: WalletScanEventKind) {
        send!(self.wallet_addr.handle_wallet_scan_event(WalletScanEvent::new(generation, kind)));
    }
}

#[async_trait::async_trait]
impl Actor for WalletScanActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletScanActor error: {error:?}");
        let wallet_generation = self.active_wallet_generation;
        self.clear_scan_lifecycle();
        if let Some(wallet_generation) = wallet_generation {
            self.send_event(
                wallet_generation,
                WalletScanEventKind::StatusChanged(WalletScanStatus::Idle),
            );
        }
        false
    }
}

impl Drop for WalletScanActor {
    fn drop(&mut self) {
        self.clear_scan_lifecycle();
    }
}

fn should_forward_scan_progress(
    last_progress_sent: &mut Option<tokio::time::Instant>,
    now: tokio::time::Instant,
) -> bool {
    let should_forward = last_progress_sent
        .map(|last_sent| now.duration_since(last_sent) >= SCAN_PROGRESS_INTERVAL)
        .unwrap_or(true);

    if should_forward {
        *last_progress_sent = Some(now);
    }

    should_forward
}

/// Builds the initial scan status shown before node progress arrives
fn initial_scan_status(scan: RunningScan, progress_start: ScanProgressStart) -> WalletScanStatus {
    match progress_start {
        ScanProgressStart::Immediate => initial_progress_status(scan),
        ScanProgressStart::Delayed(_) => WalletScanStatus::ScanningPendingProgress(scan.phase()),
    }
}

/// Builds an empty progress status for the requested scan
fn initial_progress_status(scan: RunningScan) -> WalletScanStatus {
    WalletScanStatus::Scanning(WalletScanProgress {
        phase: scan.phase(),
        checked: 0,
        gap: 0,
        stop_gap: scan.stop_gap() as u32,
        progress_basis_points: 0,
    })
}

/// Aggregates progress across all scanned keychains
fn total_keychain_progress(
    progress: impl IntoIterator<Item = KeychainProgress>,
) -> Option<KeychainProgress> {
    progress.into_iter().fold(None, |total, progress| {
        let mut total = total.unwrap_or_default();
        total.checked = total.checked.saturating_add(progress.checked);
        total.gap = total.gap.saturating_add(progress.gap);
        total.stop_gap = total.stop_gap.saturating_add(progress.stop_gap);
        Some(total)
    })
}

/// Returns whether a delayed scan should switch from pending to visible progress
fn should_reveal_delayed_progress(
    active_generation: Option<ScanActorGeneration>,
    reveal_generation: ScanActorGeneration,
    progress_is_visible: bool,
) -> bool {
    !progress_is_visible && should_accept_scan_generation(active_generation, reveal_generation)
}

/// Returns whether a node scan error represents expected cancellation
fn is_cancelled_progressive_scan(
    error: &NodeClientError,
    cancel_token: Option<&CancellationToken>,
) -> bool {
    match error {
        NodeClientError::ProgressiveScan(cove_bdk_progressive_scan::Error::Cancelled) => true,
        NodeClientError::ProgressiveScan(cove_bdk_progressive_scan::Error::ChannelClosed) => {
            cancel_token.is_some_and(CancellationToken::is_cancelled)
        }
        _ => false,
    }
}

/// Returns whether pending partial updates should flush after scan completion
fn should_flush_pending_after_scan_result(
    scan_result: &std::result::Result<FullScanResponse<KeychainKind>, NodeClientError>,
    cancel_token: &CancellationToken,
) -> bool {
    match scan_result {
        Ok(_) => false,
        Err(error) if is_cancelled_progressive_scan(error, Some(cancel_token)) => false,
        Err(_) => true,
    }
}

/// Estimates scan progress in basis points from checked addresses and remaining gap
fn scan_progress_basis_points(progress: KeychainProgress) -> u32 {
    let checked = u64::from(progress.checked);
    let remaining_gap = u64::from(progress.stop_gap.saturating_sub(progress.gap));
    let denominator = checked.saturating_add(remaining_gap);
    if denominator == 0 {
        return 0;
    }

    let basis_points = checked.saturating_mul(u64::from(SCAN_PROGRESS_BASIS_POINTS)) / denominator;
    basis_points.min(u64::from(SCAN_PROGRESS_BASIS_POINTS)) as u32
}

/// Returns whether a successful full scan should refresh persisted full-scan metadata
pub(crate) const fn should_update_full_scan_metadata(scan_type: FullScanType) -> bool {
    match scan_type {
        FullScanType::Full => true,
        FullScanType::Rescan(gap_limit) => gap_limit as usize >= FullScanType::Full.stop_gap(),
    }
}

/// Returns whether an event belongs to the active scan generation
fn should_accept_scan_generation(
    active_generation: Option<ScanActorGeneration>,
    event_generation: ScanActorGeneration,
) -> bool {
    active_generation == Some(event_generation)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use act_zero::{Actor, WeakAddr};
    use bdk_wallet::{KeychainKind, chain::spk_client::FullScanResponse};
    use cove_bdk_progressive_scan::{KeychainProgress, ScanProgress};
    use cove_common::consts::GAP_LIMIT;
    use tokio_util::sync::CancellationToken;

    use super::{
        FullScanType, ProgressiveFullScanResult, RunningScan, SCAN_PARTIAL_FLUSH_INTERVAL,
        SCAN_PROGRESS_BASIS_POINTS, SCAN_PROGRESS_INTERVAL, ScanActorGeneration, ScanFlushCadence,
        ScanFlushDecision, ScanProgressStart, ScanRequestOrder, WalletScanActor, WalletScanEvent,
        WalletScanEventKind, WalletScanGeneration, WalletScanPhase, WalletScanProgress,
        WalletScanStatus, initial_scan_status, is_cancelled_progressive_scan,
        scan_progress_basis_points, should_accept_scan_generation,
        should_flush_pending_after_scan_result, should_forward_scan_progress,
        should_reveal_delayed_progress, should_update_full_scan_metadata,
    };

    #[test]
    fn first_scan_progress_is_forwarded() {
        let mut last_progress_sent = None;
        let now = tokio::time::Instant::now();

        assert!(should_forward_scan_progress(&mut last_progress_sent, now));
        assert_eq!(last_progress_sent, Some(now));
    }

    #[test]
    fn scan_progress_is_throttled_until_interval_passes() {
        let now = tokio::time::Instant::now();
        let mut last_progress_sent = Some(now);

        assert!(!should_forward_scan_progress(
            &mut last_progress_sent,
            now + SCAN_PROGRESS_INTERVAL - Duration::from_millis(1),
        ));
        assert_eq!(last_progress_sent, Some(now));

        let next = now + SCAN_PROGRESS_INTERVAL;

        assert!(should_forward_scan_progress(&mut last_progress_sent, next));
        assert_eq!(last_progress_sent, Some(next));
    }

    #[test]
    fn scan_actor_aggregates_progress_across_keychains() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.seed_scan_progress([KeychainKind::External, KeychainKind::Internal], 20);

        let external_status = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::External, checked: 2, gap: 2, stop_gap: 20 },
            WalletScanPhase::Full,
        );
        let internal_status = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::Internal, checked: 1, gap: 1, stop_gap: 20 },
            WalletScanPhase::Full,
        );

        assert_eq!(
            external_status,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Full,
                checked: 2,
                gap: 2,
                stop_gap: 40,
                progress_basis_points: 500,
            })
        );
        assert_eq!(
            internal_status,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Full,
                checked: 3,
                gap: 3,
                stop_gap: 40,
                progress_basis_points: 750,
            })
        );
    }

    #[test]
    fn scan_actor_progress_basis_points_never_decrease_within_scan() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.seed_scan_progress([KeychainKind::External, KeychainKind::Internal], 20);

        let before_used_address = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::External, checked: 10, gap: 10, stop_gap: 20 },
            WalletScanPhase::Full,
        );
        let after_used_address = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::External, checked: 11, gap: 0, stop_gap: 20 },
            WalletScanPhase::Full,
        );

        assert_eq!(
            before_used_address,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Full,
                checked: 10,
                gap: 10,
                stop_gap: 40,
                progress_basis_points: 2_500,
            })
        );
        assert_eq!(
            after_used_address,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Full,
                checked: 11,
                gap: 0,
                stop_gap: 40,
                progress_basis_points: 2_500,
            })
        );
    }

    #[test]
    fn full_scan_progress_reports_raw_checked_count() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.seed_scan_progress([KeychainKind::External, KeychainKind::Internal], 150);
        let status = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::External, checked: 151, gap: 0, stop_gap: 150 },
            WalletScanPhase::Full,
        );

        assert_eq!(
            status,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Full,
                checked: 151,
                gap: 0,
                stop_gap: 300,
                progress_basis_points: 3_348,
            })
        );
    }

    #[test]
    fn scan_progress_basis_points_estimates_checked_against_remaining_gap() {
        assert_eq!(
            scan_progress_basis_points(KeychainProgress { checked: 10, gap: 2, stop_gap: 5 }),
            7_692
        );
        assert_eq!(
            scan_progress_basis_points(KeychainProgress { checked: 10, gap: 5, stop_gap: 5 }),
            SCAN_PROGRESS_BASIS_POINTS
        );
    }

    #[test]
    fn scan_flush_cadence_flushes_first_update_immediately() {
        let mut cadence = ScanFlushCadence::default();
        let now = tokio::time::Instant::now();

        assert_eq!(cadence.record_update(now), ScanFlushDecision::Immediate);
    }

    #[test]
    fn scan_flush_cadence_debounces_later_updates() {
        let mut cadence = ScanFlushCadence::default();
        let now = tokio::time::Instant::now();
        let later = now + Duration::from_millis(10);

        assert_eq!(cadence.record_update(now), ScanFlushDecision::Immediate);
        assert_eq!(
            cadence.record_update(later),
            ScanFlushDecision::Debounce(later + SCAN_PARTIAL_FLUSH_INTERVAL)
        );
    }

    #[test]
    fn failed_scan_flushes_pending_debounced_updates() {
        let result = Err(crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::ChannelClosed,
        ));
        let cancel_token = CancellationToken::new();

        assert!(should_flush_pending_after_scan_result(&result, &cancel_token));
    }

    #[test]
    fn drained_update_after_first_flush_is_pending_on_scan_error() {
        let mut cadence = ScanFlushCadence { sent_first_flush: true };
        let now = tokio::time::Instant::now();

        assert_eq!(
            cadence.record_update(now),
            ScanFlushDecision::Debounce(now + SCAN_PARTIAL_FLUSH_INTERVAL)
        );

        let result = Err(crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::ChannelClosed,
        ));
        let cancel_token = CancellationToken::new();

        assert!(should_flush_pending_after_scan_result(&result, &cancel_token));
    }

    #[test]
    fn successful_or_cancelled_scan_does_not_flush_pending_debounce() {
        let success = Ok(FullScanResponse::default());
        let cancelled = Err(crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::Cancelled,
        ));
        let cancel_token = CancellationToken::new();

        assert!(!should_flush_pending_after_scan_result(&success, &cancel_token));
        assert!(!should_flush_pending_after_scan_result(&cancelled, &cancel_token));
    }

    #[test]
    fn channel_closed_is_cancellation_only_when_token_is_cancelled() {
        let cancelled = crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::Cancelled,
        );
        let channel_closed = crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::ChannelClosed,
        );
        let cancel_token = CancellationToken::new();

        assert!(is_cancelled_progressive_scan(&cancelled, None));
        assert!(!is_cancelled_progressive_scan(&channel_closed, Some(&cancel_token)));

        cancel_token.cancel();

        assert!(is_cancelled_progressive_scan(&channel_closed, Some(&cancel_token)));
        assert!(!should_flush_pending_after_scan_result(&Err(channel_closed), &cancel_token));
    }

    #[test]
    fn full_scan_updates_full_scan_metadata() {
        assert!(should_update_full_scan_metadata(FullScanType::Full));
    }

    #[test]
    fn small_rescan_completes_without_updating_full_scan_metadata() {
        assert!(!should_update_full_scan_metadata(FullScanType::Rescan(20)));
    }

    #[test]
    fn full_range_rescan_updates_full_scan_metadata() {
        assert!(should_update_full_scan_metadata(FullScanType::Rescan(150)));
    }

    #[test]
    fn scan_request_order_is_receive_prioritized_only_for_incremental_scans() {
        assert_eq!(
            RunningScan::Full(FullScanType::Full).request_order(),
            ScanRequestOrder::Standard
        );
        assert_eq!(
            RunningScan::Full(FullScanType::Rescan(20)).request_order(),
            ScanRequestOrder::Standard
        );
        assert_eq!(RunningScan::Incremental.request_order(), ScanRequestOrder::ReceivePriority);
    }

    #[test]
    fn immediate_scan_start_shows_progress_status() {
        assert_eq!(
            initial_scan_status(RunningScan::Incremental, ScanProgressStart::Immediate),
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Incremental,
                checked: 0,
                gap: 0,
                stop_gap: GAP_LIMIT as u32,
                progress_basis_points: 0,
            })
        );
    }

    #[test]
    fn delayed_scan_start_hides_progress_status() {
        assert_eq!(
            initial_scan_status(
                RunningScan::Incremental,
                ScanProgressStart::Delayed(Duration::from_secs(5)),
            ),
            WalletScanStatus::ScanningPendingProgress(WalletScanPhase::Incremental)
        );
    }

    #[test]
    fn delayed_progress_reveals_only_for_active_hidden_generation() {
        let active_generation = ScanActorGeneration(7);
        let stale_generation = ScanActorGeneration(6);

        assert!(should_reveal_delayed_progress(Some(active_generation), active_generation, false));
        assert!(!should_reveal_delayed_progress(Some(active_generation), stale_generation, false));
        assert!(!should_reveal_delayed_progress(None, active_generation, false));
        assert!(!should_reveal_delayed_progress(Some(active_generation), active_generation, true));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn incremental_scan_request_is_ignored_while_scan_is_active() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        let active_generation = ScanActorGeneration(7);
        let next_generation = ScanActorGeneration(11);
        scan_actor.active_generation = Some(active_generation);
        scan_actor.next_generation = next_generation;

        assert!(
            scan_actor
                .start_incremental_scan(WalletScanGeneration::INITIAL, ScanProgressStart::Immediate)
                .await
                .is_ok()
        );

        assert_eq!(scan_actor.active_generation, Some(active_generation));
        assert_eq!(scan_actor.next_generation, next_generation);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_scan_request_is_ignored_while_scan_is_active() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        let active_generation = ScanActorGeneration(7);
        let next_generation = ScanActorGeneration(11);
        let cancel_token = CancellationToken::new();
        let token_observer = cancel_token.clone();
        scan_actor.active_cancel_token = Some(cancel_token);
        scan_actor.active_generation = Some(active_generation);
        scan_actor.next_generation = next_generation;

        assert!(
            scan_actor
                .start_full_scan(WalletScanGeneration::INITIAL, ScanProgressStart::Immediate)
                .await
                .is_ok()
        );

        assert!(!token_observer.is_cancelled());
        assert_eq!(scan_actor.active_generation, Some(active_generation));
        assert_eq!(scan_actor.next_generation, next_generation);
    }

    #[test]
    fn scan_events_are_accepted_only_for_active_generation() {
        let active_generation = ScanActorGeneration(7);
        let stale_generation = ScanActorGeneration(6);

        assert!(should_accept_scan_generation(Some(active_generation), active_generation));
        assert!(!should_accept_scan_generation(Some(active_generation), stale_generation));
        assert!(!should_accept_scan_generation(None, active_generation));
    }

    #[test]
    fn starting_new_scan_generation_cancels_previous_token() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        let wallet_generation = WalletScanGeneration::INITIAL;
        let (first_generation, first_cancel_token) =
            scan_actor.start_scan_generation(wallet_generation);
        let first_token_observer = first_cancel_token.clone();

        let (second_generation, second_cancel_token) =
            scan_actor.start_scan_generation(wallet_generation.next());

        assert_eq!(first_generation, ScanActorGeneration::INITIAL);
        assert_eq!(second_generation, ScanActorGeneration::INITIAL.next());
        assert!(first_token_observer.is_cancelled());
        assert!(!second_cancel_token.is_cancelled());
        assert_eq!(scan_actor.active_generation, Some(ScanActorGeneration::INITIAL.next()));
        assert_eq!(scan_actor.active_wallet_generation, Some(wallet_generation.next()));
    }

    #[test]
    fn clear_scan_lifecycle_cancels_active_scan_and_clears_queued_state() {
        let cancel_token = CancellationToken::new();
        let token_observer = cancel_token.clone();
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.active_cancel_token = Some(cancel_token);
        scan_actor.active_generation = Some(ScanActorGeneration(7));
        scan_actor.active_wallet_generation = Some(WalletScanGeneration::INITIAL);
        scan_actor.queued_rescan = Some(super::QueuedRescan {
            gap_limit: 42,
            wallet_generation: WalletScanGeneration::INITIAL,
        });

        scan_actor.clear_scan_lifecycle();

        assert!(token_observer.is_cancelled());
        assert!(scan_actor.active_cancel_token.is_none());
        assert!(scan_actor.active_generation.is_none());
        assert!(scan_actor.active_wallet_generation.is_none());
        assert!(scan_actor.queued_rescan.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scan_apply_error_preserves_wallet_generation_for_error_cleanup() {
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.active_cancel_token = Some(CancellationToken::new());
        let active_generation = ScanActorGeneration(7);
        scan_actor.active_generation = Some(active_generation);
        scan_actor.active_wallet_generation = Some(WalletScanGeneration::INITIAL);
        scan_actor.progress_is_visible = true;

        let error = scan_actor
            .handle_scan_result(ProgressiveFullScanResult {
                scan: RunningScan::Full(FullScanType::Full),
                scan_generation: active_generation,
                wallet_generation: WalletScanGeneration::INITIAL,
                result: Ok(FullScanResponse::default()),
            })
            .await
            .expect_err("detached wallet actor should make scan apply fail");

        assert!(scan_actor.active_cancel_token.is_none());
        assert!(scan_actor.active_generation.is_none());
        assert_eq!(scan_actor.active_wallet_generation, Some(WalletScanGeneration::INITIAL));
        assert!(!scan_actor.progress_is_visible);

        assert!(!Actor::error(&mut scan_actor, error).await);
        assert!(scan_actor.active_wallet_generation.is_none());
    }

    #[test]
    fn wallet_scan_event_carries_wallet_generation() {
        let generation = WalletScanGeneration::INITIAL.next();
        let event = WalletScanEvent::new(
            generation,
            WalletScanEventKind::StatusChanged(WalletScanStatus::Idle),
        );

        assert_eq!(event.generation(), generation);
    }
}

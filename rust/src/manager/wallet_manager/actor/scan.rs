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
use tracing::debug;

use crate::{
    manager::wallet_manager::{WalletScanPhase, WalletScanProgress, WalletScanStatus},
    node::client::NodeClient,
};

use super::WalletActor;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum FullScanType {
    /// Initial scan scans for 20 addresses GAP_LIMIT
    Initial,
    /// Expanded scan scans for 150 addresses GAP_LIMIT
    Expanded,
    Rescan(u32),
}

impl FullScanType {
    pub(crate) const fn stop_gap(&self) -> usize {
        match self {
            Self::Initial => 20,
            Self::Expanded => 150,
            Self::Rescan(gap) => *gap as usize,
        }
    }

    pub(crate) const fn phase(&self) -> WalletScanPhase {
        match self {
            Self::Initial => WalletScanPhase::Initial,
            Self::Expanded => WalletScanPhase::Expanded,
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

pub(crate) enum WalletScanEvent {
    FullScanStarted(FullScanType),
    IncrementalScanStarted,
    StatusChanged(WalletScanStatus),
    PartialUpdate(ScanUpdate<KeychainKind>),
    FlushUi,
    FullScanFinished {
        scan_type: FullScanType,
        result: std::result::Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    },
    IncrementalScanFinished {
        result: std::result::Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FullScanCompletionEffect {
    DeferUserCompletion,
    CompleteUserScan { update_full_scan_metadata: bool },
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
}

struct ProgressiveFullScanJob {
    scan: RunningScan,
    scan_generation: u64,
    cancel_token: CancellationToken,
    prepared: PreparedProgressiveScan,
}

const SCAN_PROGRESS_INTERVAL: Duration = Duration::from_millis(75);
const SCAN_PARTIAL_FLUSH_INTERVAL: Duration = Duration::from_millis(300);
const SCAN_PROGRESS_BASIS_POINTS: u32 = 10_000;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ScanFlushDecision {
    Immediate,
    Debounce(tokio::time::Instant),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum QueuedRescanDisposition {
    Start,
    KeepQueued,
}

#[derive(Debug, Default)]
struct ScanFlushCadence {
    sent_first_flush: bool,
}

impl ScanFlushCadence {
    fn record_update(&mut self, now: tokio::time::Instant) -> ScanFlushDecision {
        if self.sent_first_flush {
            ScanFlushDecision::Debounce(now + SCAN_PARTIAL_FLUSH_INTERVAL)
        } else {
            self.sent_first_flush = true;
            ScanFlushDecision::Immediate
        }
    }
}

pub(crate) struct WalletScanActor {
    addr: WeakAddr<Self>,
    wallet_addr: WeakAddr<WalletActor>,
    progress: BTreeMap<KeychainKind, KeychainProgress>,
    active_cancel_token: Option<CancellationToken>,
    active_generation: Option<u64>,
    next_generation: u64,
    queued_rescan_gap_limit: Option<u32>,
    reserved_follow_up_scan: bool,
    max_progress_basis_points: u32,
}

impl WalletScanActor {
    pub(crate) fn new(wallet_addr: WeakAddr<WalletActor>) -> Self {
        Self {
            addr: WeakAddr::default(),
            wallet_addr,
            progress: BTreeMap::default(),
            active_cancel_token: None,
            active_generation: None,
            next_generation: 0,
            queued_rescan_gap_limit: None,
            reserved_follow_up_scan: false,
            max_progress_basis_points: 0,
        }
    }

    pub(crate) async fn start_initial_full_scan(&mut self) -> ActorResult<()> {
        self.begin_scan(RunningScan::Full(FullScanType::Initial))
    }

    pub(crate) async fn start_incremental_scan(&mut self) -> ActorResult<()> {
        self.begin_scan(RunningScan::Incremental)
    }

    async fn start_expanded_full_scan(&mut self) -> ActorResult<()> {
        self.begin_scan(RunningScan::Full(FullScanType::Expanded))
    }

    pub(crate) async fn start_rescan(&mut self, gap_limit: u32) -> ActorResult<()> {
        if self.active_generation.is_some() || self.reserved_follow_up_scan {
            debug!("scan already in progress, queueing rescan gap_limit={gap_limit}");
            self.queued_rescan_gap_limit = Some(gap_limit);
            return Produces::ok(());
        }

        self.begin_scan(RunningScan::Full(FullScanType::Rescan(gap_limit)))
    }

    pub(crate) async fn shutdown(&mut self) -> ActorResult<()> {
        self.clear_scan_lifecycle();
        self.send_event(WalletScanEvent::StatusChanged(WalletScanStatus::Idle));
        Produces::ok(())
    }

    fn begin_scan(&mut self, scan: RunningScan) -> ActorResult<()> {
        self.reserved_follow_up_scan = false;
        self.progress.clear();
        self.max_progress_basis_points = 0;

        let (scan_generation, cancel_token) = self.start_scan_generation();
        match scan {
            RunningScan::Full(scan_type) => {
                self.send_event(WalletScanEvent::FullScanStarted(scan_type));
            }
            RunningScan::Incremental => {
                self.send_event(WalletScanEvent::IncrementalScanStarted);
            }
        }
        self.send_event(WalletScanEvent::StatusChanged(WalletScanStatus::Scanning(
            WalletScanProgress {
                phase: scan.phase(),
                checked: 0,
                gap: 0,
                stop_gap: scan.stop_gap() as u32,
                progress_basis_points: 0,
            },
        )));

        let addr = self.addr.clone();
        let wallet_addr = self.wallet_addr.clone();
        self.addr.send_fut(async move {
            match call!(wallet_addr.prepare_progressive_scan()).await {
                Ok(prepared) => {
                    let job =
                        ProgressiveFullScanJob { scan, scan_generation, cancel_token, prepared };
                    send!(addr.run_progressive_scan_job(job));
                }
                Err(error) => {
                    debug!("failed to prepare progressive scan: {error:?}");
                    send!(addr.handle_scan_prepare_failed());
                }
            }
        });

        Produces::ok(())
    }

    async fn run_progressive_scan_job(&mut self, job: ProgressiveFullScanJob) -> ActorResult<()> {
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

    async fn run_progressive_full_scan(
        &mut self,
        job: ProgressiveFullScanJob,
    ) -> ActorResult<(
        RunningScan,
        std::result::Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    )> {
        let (events_tx, events_rx) = flume::bounded(32);
        self.seed_scan_progress(
            job.prepared.full_scan_request.keychains(),
            job.scan.stop_gap() as u32,
        );

        let scan_future = job.prepared.node_client.start_progressive_wallet_scan(
            &job.prepared.graph,
            job.prepared.full_scan_request,
            job.prepared.last_revealed_indices,
            job.scan.stop_gap(),
            events_tx,
            job.cancel_token.clone(),
        );
        tokio::pin!(scan_future);
        let mut flush_cadence = ScanFlushCadence::default();
        let mut flush_deadline = Option::<tokio::time::Instant>::None;
        let mut last_progress_sent = Option::<tokio::time::Instant>::None;

        let scan_result = loop {
            tokio::select! {
                result = &mut scan_future => break result,
                event = events_rx.recv_async() => {
                    if let Ok(event) = event
                        && self.forward_scan_event(
                        event,
                        job.scan_generation,
                        job.scan.phase(),
                        &mut last_progress_sent,
                    )
                        && record_scan_update_flush(
                            &mut flush_cadence,
                            &mut flush_deadline,
                            tokio::time::Instant::now(),
                        )
                    {
                        self.send_event(WalletScanEvent::FlushUi);
                    }
                }
                _ = async {
                    tokio::time::sleep_until(flush_deadline.expect("flush deadline is set")).await;
                }, if flush_deadline.is_some() => {
                    flush_deadline = None;
                    self.send_event(WalletScanEvent::FlushUi);
                }
            }
        };

        while let Ok(event) = events_rx.try_recv() {
            if self.forward_scan_event(
                event,
                job.scan_generation,
                job.scan.phase(),
                &mut last_progress_sent,
            ) && record_scan_update_flush(
                &mut flush_cadence,
                &mut flush_deadline,
                tokio::time::Instant::now(),
            ) {
                self.send_event(WalletScanEvent::FlushUi);
            }
        }

        if flush_deadline.is_some()
            && should_flush_pending_after_scan_result(&scan_result, &job.cancel_token)
        {
            self.send_event(WalletScanEvent::FlushUi);
        }

        Produces::ok((job.scan, scan_result))
    }

    async fn handle_scan_result(
        &mut self,
        (scan, result): (
            RunningScan,
            std::result::Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
        ),
    ) -> ActorResult<()> {
        let cancel_token = self.active_cancel_token.as_ref();
        if let Err(error) = &result
            && is_cancelled_progressive_scan(error, cancel_token)
        {
            self.clear_scan_lifecycle();
            self.send_event(WalletScanEvent::StatusChanged(WalletScanStatus::Idle));
            return Produces::ok(());
        }

        self.clear_active_scan();

        match scan {
            RunningScan::Full(scan_type) => {
                let result_is_ok = result.is_ok();
                let apply_result = call!(self.wallet_addr.handle_wallet_scan_event(
                    WalletScanEvent::FullScanFinished { scan_type, result }
                ))
                .await;

                if let Err(error) = apply_result {
                    return Err(Box::new(error));
                }

                if result_is_ok && scan_type == FullScanType::Initial {
                    self.reserved_follow_up_scan = true;
                    let addr = self.addr.clone();
                    send!(addr.start_expanded_full_scan());
                    return Produces::ok(());
                }

                if !result_is_ok {
                    self.handle_queued_rescan(queued_rescan_after_failed_full_scan(scan_type))
                        .await?;
                } else {
                    self.handle_queued_rescan(queued_rescan_after_successful_full_scan(scan_type))
                        .await?;
                }
            }
            RunningScan::Incremental => {
                let result_is_ok = result.is_ok();
                let apply_result =
                    call!(self.wallet_addr.handle_wallet_scan_event(
                        WalletScanEvent::IncrementalScanFinished { result }
                    ))
                    .await;

                self.start_queued_rescan().await?;

                if let Err(error) = apply_result {
                    return Err(Box::new(error));
                }

                if !result_is_ok {
                    return Produces::ok(());
                }
            }
        }

        Produces::ok(())
    }

    async fn handle_queued_rescan(
        &mut self,
        disposition: QueuedRescanDisposition,
    ) -> ActorResult<()> {
        match disposition {
            QueuedRescanDisposition::Start => self.start_queued_rescan().await,
            QueuedRescanDisposition::KeepQueued => Produces::ok(()),
        }
    }

    async fn start_queued_rescan(&mut self) -> ActorResult<()> {
        let Some(gap_limit) = self.queued_rescan_gap_limit.take() else {
            return Produces::ok(());
        };

        self.begin_scan(RunningScan::Full(FullScanType::Rescan(gap_limit)))
    }

    async fn handle_scan_prepare_failed(&mut self) -> ActorResult<()> {
        self.clear_active_scan();
        self.send_event(WalletScanEvent::StatusChanged(WalletScanStatus::Idle));
        Produces::ok(())
    }

    fn forward_scan_event(
        &mut self,
        event: ScanEvent<KeychainKind>,
        scan_generation: u64,
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
                    self.send_event(WalletScanEvent::StatusChanged(status));
                }
                false
            }
            ScanEvent::Update(update) => {
                self.send_event(WalletScanEvent::PartialUpdate(update));
                true
            }
            ScanEvent::Complete(_) => false,
        }
    }

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

        let total =
            self.progress.values().fold(KeychainProgress::default(), |mut total, progress| {
                total.checked = total.checked.saturating_add(progress.checked);
                total.gap = total.gap.saturating_add(progress.gap);
                total.stop_gap = total.stop_gap.saturating_add(progress.stop_gap);
                total
            });
        let progress_basis_points = scan_progress_basis_points(total);
        self.max_progress_basis_points = self.max_progress_basis_points.max(progress_basis_points);

        WalletScanStatus::Scanning(WalletScanProgress {
            phase,
            checked: total.checked,
            gap: total.gap,
            stop_gap: total.stop_gap,
            progress_basis_points: self.max_progress_basis_points,
        })
    }

    fn start_scan_generation(&mut self) -> (u64, CancellationToken) {
        let cancel_token = CancellationToken::new();
        let scan_generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.active_cancel_token = Some(cancel_token.clone());
        self.active_generation = Some(scan_generation);
        (scan_generation, cancel_token)
    }

    fn clear_active_scan(&mut self) {
        self.active_cancel_token = None;
        self.active_generation = None;
    }

    fn clear_scan_lifecycle(&mut self) {
        if let Some(cancel_token) = self.active_cancel_token.take() {
            cancel_token.cancel();
        }
        self.active_generation = None;
        self.queued_rescan_gap_limit = None;
        self.reserved_follow_up_scan = false;
    }

    fn seed_scan_progress(
        &mut self,
        keychains: impl IntoIterator<Item = KeychainKind>,
        stop_gap: u32,
    ) {
        self.progress = keychains
            .into_iter()
            .map(|keychain| (keychain, KeychainProgress { checked: 0, gap: 0, stop_gap }))
            .collect();
        self.max_progress_basis_points = 0;
    }

    fn send_event(&self, event: WalletScanEvent) {
        send!(self.wallet_addr.handle_wallet_scan_event(event));
    }
}

#[async_trait::async_trait]
impl Actor for WalletScanActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
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

fn is_cancelled_progressive_scan(
    error: &crate::node::client::Error,
    cancel_token: Option<&CancellationToken>,
) -> bool {
    match error {
        crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::Cancelled,
        ) => true,
        crate::node::client::Error::ProgressiveScan(
            cove_bdk_progressive_scan::Error::ChannelClosed,
        ) => cancel_token.is_some_and(CancellationToken::is_cancelled),
        _ => false,
    }
}

fn should_flush_pending_after_scan_result(
    scan_result: &std::result::Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    cancel_token: &CancellationToken,
) -> bool {
    match scan_result {
        Ok(_) => false,
        Err(error) if is_cancelled_progressive_scan(error, Some(cancel_token)) => false,
        Err(_) => true,
    }
}

fn record_scan_update_flush(
    flush_cadence: &mut ScanFlushCadence,
    flush_deadline: &mut Option<tokio::time::Instant>,
    now: tokio::time::Instant,
) -> bool {
    match flush_cadence.record_update(now) {
        ScanFlushDecision::Immediate => true,
        ScanFlushDecision::Debounce(deadline) => {
            *flush_deadline = Some(deadline);
            false
        }
    }
}

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

pub(crate) fn successful_full_scan_completion_effect(
    scan_type: FullScanType,
) -> FullScanCompletionEffect {
    match scan_type {
        FullScanType::Initial => FullScanCompletionEffect::DeferUserCompletion,
        FullScanType::Expanded => {
            FullScanCompletionEffect::CompleteUserScan { update_full_scan_metadata: true }
        }
        FullScanType::Rescan(gap_limit) => FullScanCompletionEffect::CompleteUserScan {
            update_full_scan_metadata: gap_limit as usize >= FullScanType::Expanded.stop_gap(),
        },
    }
}

fn queued_rescan_after_successful_full_scan(scan_type: FullScanType) -> QueuedRescanDisposition {
    match scan_type {
        FullScanType::Initial => QueuedRescanDisposition::KeepQueued,
        FullScanType::Expanded | FullScanType::Rescan(_) => QueuedRescanDisposition::Start,
    }
}

fn queued_rescan_after_failed_full_scan(_scan_type: FullScanType) -> QueuedRescanDisposition {
    QueuedRescanDisposition::Start
}

fn should_accept_scan_generation(active_generation: Option<u64>, event_generation: u64) -> bool {
    active_generation == Some(event_generation)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use act_zero::WeakAddr;
    use bdk_wallet::{KeychainKind, chain::spk_client::FullScanResponse};
    use cove_bdk_progressive_scan::{KeychainProgress, ScanProgress};
    use tokio_util::sync::CancellationToken;

    use super::{
        FullScanCompletionEffect, FullScanType, QueuedRescanDisposition,
        SCAN_PARTIAL_FLUSH_INTERVAL, SCAN_PROGRESS_BASIS_POINTS, SCAN_PROGRESS_INTERVAL,
        ScanFlushCadence, ScanFlushDecision, WalletScanActor, WalletScanPhase, WalletScanProgress,
        WalletScanStatus, is_cancelled_progressive_scan, queued_rescan_after_failed_full_scan,
        queued_rescan_after_successful_full_scan, record_scan_update_flush,
        scan_progress_basis_points, should_accept_scan_generation,
        should_flush_pending_after_scan_result, should_forward_scan_progress,
        successful_full_scan_completion_effect,
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
            WalletScanPhase::Expanded,
        );
        let internal_status = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::Internal, checked: 1, gap: 1, stop_gap: 20 },
            WalletScanPhase::Expanded,
        );

        assert_eq!(
            external_status,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Expanded,
                checked: 2,
                gap: 2,
                stop_gap: 40,
                progress_basis_points: 500,
            })
        );
        assert_eq!(
            internal_status,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Expanded,
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
            WalletScanPhase::Expanded,
        );
        let after_used_address = scan_actor.scan_status_from_progress(
            ScanProgress { keychain: KeychainKind::External, checked: 11, gap: 0, stop_gap: 20 },
            WalletScanPhase::Expanded,
        );

        assert_eq!(
            before_used_address,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Expanded,
                checked: 10,
                gap: 10,
                stop_gap: 40,
                progress_basis_points: 2_500,
            })
        );
        assert_eq!(
            after_used_address,
            WalletScanStatus::Scanning(WalletScanProgress {
                phase: WalletScanPhase::Expanded,
                checked: 11,
                gap: 0,
                stop_gap: 40,
                progress_basis_points: 2_500,
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
    fn record_scan_update_flush_records_debounced_deadline() {
        let mut cadence = ScanFlushCadence::default();
        let mut deadline = None;
        let now = tokio::time::Instant::now();
        let later = now + Duration::from_millis(10);

        assert!(record_scan_update_flush(&mut cadence, &mut deadline, now));
        assert_eq!(deadline, None);
        assert!(!record_scan_update_flush(&mut cadence, &mut deadline, later));
        assert_eq!(deadline, Some(later + SCAN_PARTIAL_FLUSH_INTERVAL));
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
    fn initial_full_scan_defers_user_visible_completion() {
        assert_eq!(
            successful_full_scan_completion_effect(FullScanType::Initial),
            FullScanCompletionEffect::DeferUserCompletion
        );
    }

    #[test]
    fn expanded_full_scan_completes_and_updates_full_scan_metadata() {
        assert_eq!(
            successful_full_scan_completion_effect(FullScanType::Expanded),
            FullScanCompletionEffect::CompleteUserScan { update_full_scan_metadata: true }
        );
    }

    #[test]
    fn small_rescan_completes_without_updating_full_scan_metadata() {
        assert_eq!(
            successful_full_scan_completion_effect(FullScanType::Rescan(20)),
            FullScanCompletionEffect::CompleteUserScan { update_full_scan_metadata: false }
        );
    }

    #[test]
    fn expanded_range_rescan_updates_full_scan_metadata() {
        assert_eq!(
            successful_full_scan_completion_effect(FullScanType::Rescan(150)),
            FullScanCompletionEffect::CompleteUserScan { update_full_scan_metadata: true }
        );
    }

    #[test]
    fn queued_rescan_waits_for_expanded_scan_after_successful_initial_scan() {
        assert_eq!(
            queued_rescan_after_successful_full_scan(FullScanType::Initial),
            QueuedRescanDisposition::KeepQueued
        );
    }

    #[test]
    fn queued_rescan_starts_after_successful_expanded_or_rescan_completion() {
        assert_eq!(
            queued_rescan_after_successful_full_scan(FullScanType::Expanded),
            QueuedRescanDisposition::Start
        );
        assert_eq!(
            queued_rescan_after_successful_full_scan(FullScanType::Rescan(20)),
            QueuedRescanDisposition::Start
        );
    }

    #[test]
    fn queued_rescan_starts_after_any_full_scan_failure() {
        assert_eq!(
            queued_rescan_after_failed_full_scan(FullScanType::Initial),
            QueuedRescanDisposition::Start
        );
        assert_eq!(
            queued_rescan_after_failed_full_scan(FullScanType::Expanded),
            QueuedRescanDisposition::Start
        );
        assert_eq!(
            queued_rescan_after_failed_full_scan(FullScanType::Rescan(20)),
            QueuedRescanDisposition::Start
        );
    }

    #[test]
    fn scan_events_are_accepted_only_for_active_generation() {
        assert!(should_accept_scan_generation(Some(7), 7));
        assert!(!should_accept_scan_generation(Some(7), 6));
        assert!(!should_accept_scan_generation(None, 7));
    }

    #[test]
    fn clear_scan_lifecycle_cancels_active_scan_and_clears_queued_state() {
        let cancel_token = CancellationToken::new();
        let token_observer = cancel_token.clone();
        let mut scan_actor = WalletScanActor::new(WeakAddr::default());
        scan_actor.active_cancel_token = Some(cancel_token);
        scan_actor.active_generation = Some(7);
        scan_actor.queued_rescan_gap_limit = Some(42);

        scan_actor.clear_scan_lifecycle();

        assert!(token_observer.is_cancelled());
        assert!(scan_actor.active_cancel_token.is_none());
        assert!(scan_actor.active_generation.is_none());
        assert!(scan_actor.queued_rescan_gap_limit.is_none());
    }
}

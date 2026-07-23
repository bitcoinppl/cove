use std::collections::HashSet;
use std::sync::Arc;

use act_zero::*;
use cove_types::network::Network;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::database::Database;
use crate::manager::reconcile_channel::ReconcileChannel;
use crate::node::Node;
use crate::node::tls::TlsTrust;
use crate::node_scanner::scan::{self, ScanContext};
use crate::node_scanner::subnet;
use crate::node_scanner::verify::{VerifiedMetadata, VerifyError};
use crate::node_scanner::{
    FoundNode, FoundNodeTransport, NodeScanProgress, NodeScannerError, NodeScannerReconcileMessage,
    NodeScannerResult, NodeScannerState, ScanEvent, TrustRequiredEndpoint,
};

#[derive(Debug, Clone)]
pub(crate) struct TrustPromotion {
    pub(crate) generation: u64,
    pub(crate) network: Network,
    pub(crate) candidate: TrustRequiredEndpoint,
    pub(crate) trust: TlsTrust,
    pub(crate) tcp_fallback: bool,
}

pub(crate) struct PromotionVerification {
    pub(crate) metadata: VerifiedMetadata,
    pub(crate) transport: FoundNodeTransport,
}

pub(crate) struct NodeScannerActor {
    addr: WeakAddr<Self>,
    state: Arc<RwLock<NodeScannerState>>,
    reconciler: ReconcileChannel<NodeScannerReconcileMessage>,
    next_generation: u64,
    active_generation: Option<u64>,
    active_cancel: Option<CancellationToken>,
    tcp_fallback_hosts: HashSet<String>,
}

impl std::fmt::Debug for NodeScannerActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeScannerActor")
            .field("state", &self.state)
            .field("next_generation", &self.next_generation)
            .field("active_generation", &self.active_generation)
            .finish_non_exhaustive()
    }
}

impl NodeScannerActor {
    pub(crate) fn new(
        state: Arc<RwLock<NodeScannerState>>,
        reconciler: ReconcileChannel<NodeScannerReconcileMessage>,
    ) -> Self {
        Self {
            addr: WeakAddr::default(),
            state,
            reconciler,
            next_generation: 1,
            active_generation: None,
            active_cancel: None,
            tcp_fallback_hosts: HashSet::new(),
        }
    }

    pub(crate) async fn start_scan(&mut self) -> ActorResult<()> {
        if matches!(*self.state.read(), NodeScannerState::Scanning { .. }) {
            return Produces::ok(());
        }

        let generation = self.claim_generation();
        self.active_generation = Some(generation);
        self.tcp_fallback_hosts.clear();

        let network = Database::global().global_config.selected_network();
        let hosts = match subnet::hosts_to_scan() {
            Ok(hosts) => hosts,
            Err(error) => {
                self.set_state(NodeScannerState::Failed {
                    generation,
                    network,
                    error: error.clone(),
                    results: Vec::new(),
                });
                self.reconciler.send(NodeScannerReconcileMessage::ScanFailed { generation, error });

                return Produces::ok(());
            }
        };

        let progress = NodeScanProgress { checked_hosts: 0, total_hosts: hosts.len() as u32 };
        self.set_state(NodeScannerState::Scanning {
            generation,
            network,
            progress,
            results: Vec::new(),
        });
        self.reconciler.send(NodeScannerReconcileMessage::ScanStarted {
            generation,
            network,
            progress,
        });

        let selected = Database::global().global_config.selected_node();
        let context = ScanContext {
            generation,
            network,
            selected_tls_url: selected.tls.as_ref().map(|_| selected.url),
            selected_tls_trust: selected.tls,
        };
        let cancel = CancellationToken::new();
        self.active_cancel = Some(cancel.clone());

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let (events_tx, events_rx) = flume::bounded(128);
            let scan = scan::run(hosts, context, cancel, events_tx);
            tokio::pin!(scan);

            loop {
                tokio::select! {
                    terminal = &mut scan => {
                        while let Ok(event) = events_rx.try_recv() {
                            let _ = call!(addr.handle_scan_event(generation, event)).await;
                        }

                        if let Some(event) = terminal {
                            let _ = call!(addr.handle_scan_event(generation, event)).await;
                        }
                        break;
                    }
                    event = events_rx.recv_async() => {
                        let Ok(event) = event else { continue };
                        let _ = call!(addr.handle_scan_event(generation, event)).await;
                    }
                }
            }
        });

        Produces::ok(())
    }

    pub(crate) async fn stop_scan(&mut self) -> ActorResult<()> {
        self.stop_active_scan();
        Produces::ok(())
    }

    pub(crate) async fn shutdown(&mut self) -> ActorResult<()> {
        self.cancel_active_work();
        Produces::ok(())
    }

    pub(crate) async fn handle_scan_event(
        &mut self,
        generation: u64,
        event: ScanEvent,
    ) -> ActorResult<()> {
        if self.active_generation != Some(generation)
            || !matches!(*self.state.read(), NodeScannerState::Scanning { .. })
        {
            debug!("ignoring stale local node scan event generation={generation}");
            return Produces::ok(());
        }

        match event {
            ScanEvent::Progress(progress) => self.apply_progress(generation, progress),
            ScanEvent::Result { result, tcp_fallback } => {
                if tcp_fallback {
                    self.tcp_fallback_hosts.insert(result.ip().to_string());
                }
                self.apply_result(generation, result);
            }
            ScanEvent::Finished(summary) => {
                debug!(
                    "local node scan completed generation={generation} successful_connects={}",
                    summary.successful_connects
                );
                self.finish_scan(generation);
            }
            ScanEvent::Failed(error) => self.fail_scan(generation, error),
        }

        Produces::ok(())
    }

    pub(crate) async fn create_node(
        &mut self,
        found: FoundNode,
    ) -> ActorResult<Result<Node, NodeScannerError>> {
        let result = self.validated_found(&found).and_then(|network| {
            self.ensure_network(network)?;
            self.stop_active_scan();

            Ok(node_from_found(found, network))
        });

        Produces::ok(result)
    }

    pub(crate) async fn prepare_trust_promotion(
        &mut self,
        candidate: TrustRequiredEndpoint,
    ) -> ActorResult<Result<TrustPromotion, NodeScannerError>> {
        let result = self.validated_candidate(&candidate).and_then(|network| {
            self.ensure_network(network)?;
            self.stop_active_scan();

            Ok(TrustPromotion {
                generation: candidate.scan_generation,
                network,
                trust: TlsTrust::PinnedFingerprint { sha256: candidate.certificate.sha256.clone() },
                tcp_fallback: self.tcp_fallback_hosts.contains(&candidate.ip),
                candidate,
            })
        });

        Produces::ok(result)
    }

    pub(crate) async fn complete_trust_promotion(
        &mut self,
        promotion: TrustPromotion,
        verification: Result<PromotionVerification, VerifyError>,
    ) -> ActorResult<Result<Node, NodeScannerError>> {
        let result = self.complete_promotion(promotion, verification);
        Produces::ok(result)
    }

    fn complete_promotion(
        &mut self,
        promotion: TrustPromotion,
        verification: Result<PromotionVerification, VerifyError>,
    ) -> Result<Node, NodeScannerError> {
        let network = self.validated_candidate(&promotion.candidate)?;
        if network != promotion.network || self.active_generation != Some(promotion.generation) {
            return Err(NodeScannerError::ResultUnavailable);
        }
        self.ensure_network(network)?;

        let verification = match verification {
            Ok(verification) => verification,
            Err(error) => {
                self.remove_result(promotion.generation, &promotion.candidate.ip);

                return Err(NodeScannerError::ScanFailed(error.to_string()));
            }
        };
        let (port, tls) = match verification.transport {
            FoundNodeTransport::Tcp => (50001, None),
            FoundNodeTransport::Tls => (promotion.candidate.port, Some(promotion.trust)),
        };
        let found = FoundNode {
            scan_generation: promotion.generation,
            ip: promotion.candidate.ip,
            port,
            transport: verification.transport,
            server_software: verification.metadata.server_software,
            protocol_version: verification.metadata.protocol_version,
            tls,
        };

        self.apply_result(promotion.generation, NodeScannerResult::Verified(found.clone()));

        Ok(node_from_found(found, network))
    }

    fn apply_progress(&mut self, generation: u64, progress: NodeScanProgress) {
        let NodeScannerState::Scanning {
            generation: state_generation,
            network,
            progress: current,
            results,
        } = self.state.read().clone()
        else {
            return;
        };
        if generation != state_generation || progress.checked_hosts < current.checked_hosts {
            return;
        }

        self.set_state(NodeScannerState::Scanning { generation, network, progress, results });
        self.reconciler.send(NodeScannerReconcileMessage::ScanProgress { generation, progress });
    }

    fn apply_result(&mut self, generation: u64, result: NodeScannerResult) {
        let state = self.state.read().clone();
        let Some(updated) = map_results(state, |results| upsert_result(results, result.clone()))
        else {
            return;
        };

        self.set_state(updated);
        self.reconciler.send(NodeScannerReconcileMessage::ResultUpserted { generation, result });
    }

    fn remove_result(&mut self, generation: u64, ip: &str) {
        let state = self.state.read().clone();
        let Some(updated) = map_results(state, |results| {
            results.retain(|result| result.ip() != ip);
        }) else {
            return;
        };

        self.set_state(updated);
        self.reconciler
            .send(NodeScannerReconcileMessage::ResultRemoved { generation, ip: ip.to_string() });
    }

    fn finish_scan(&mut self, generation: u64) {
        let NodeScannerState::Scanning { network, results, .. } = self.state.read().clone() else {
            return;
        };
        self.active_cancel = None;
        self.set_state(NodeScannerState::Finished { generation, network, results });
        self.reconciler.send(NodeScannerReconcileMessage::ScanFinished { generation });
    }

    fn fail_scan(&mut self, generation: u64, error: NodeScannerError) {
        let NodeScannerState::Scanning { network, results, .. } = self.state.read().clone() else {
            return;
        };
        self.active_cancel = None;
        self.set_state(NodeScannerState::Failed {
            generation,
            network,
            error: error.clone(),
            results,
        });
        self.reconciler.send(NodeScannerReconcileMessage::ScanFailed { generation, error });
    }

    fn stop_active_scan(&mut self) {
        let NodeScannerState::Scanning { generation, network, results, .. } =
            self.state.read().clone()
        else {
            return;
        };
        self.cancel_active_work();
        self.active_generation = Some(generation);
        self.set_state(NodeScannerState::Stopped { generation, network, results });
        self.reconciler.send(NodeScannerReconcileMessage::ScanStopped { generation });
    }

    fn cancel_active_work(&mut self) {
        if let Some(cancel) = self.active_cancel.take() {
            cancel.cancel();
        }
    }

    fn claim_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);

        generation
    }

    fn validated_found(&self, found: &FoundNode) -> Result<Network, NodeScannerError> {
        let state = self.state.read();
        if !selectable_state(&state) || state.generation() != Some(found.scan_generation) {
            return Err(NodeScannerError::ResultUnavailable);
        }
        let exact = state
            .results()
            .iter()
            .any(|result| result == &NodeScannerResult::Verified(found.clone()));
        if !exact {
            return Err(NodeScannerError::ResultUnavailable);
        }

        state.network().ok_or(NodeScannerError::ResultUnavailable)
    }

    fn validated_candidate(
        &self,
        candidate: &TrustRequiredEndpoint,
    ) -> Result<Network, NodeScannerError> {
        let state = self.state.read();
        if !selectable_state(&state) || state.generation() != Some(candidate.scan_generation) {
            return Err(NodeScannerError::ResultUnavailable);
        }
        let exact = state
            .results()
            .iter()
            .any(|result| result == &NodeScannerResult::TrustRequired(candidate.clone()));
        if !exact {
            return Err(NodeScannerError::ResultUnavailable);
        }

        state.network().ok_or(NodeScannerError::ResultUnavailable)
    }

    fn ensure_network(&self, expected: Network) -> Result<(), NodeScannerError> {
        if Database::global().global_config.selected_network() != expected {
            return Err(NodeScannerError::NetworkChanged);
        }

        Ok(())
    }

    fn set_state(&self, state: NodeScannerState) {
        *self.state.write() = state;
    }
}

#[async_trait::async_trait]
impl Actor for NodeScannerActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }

    async fn error(&mut self, actor_error: ActorError) -> bool {
        error!("NodeScannerActor error: {actor_error:?}");
        self.cancel_active_work();
        false
    }
}

impl Drop for NodeScannerActor {
    fn drop(&mut self) {
        self.cancel_active_work();
    }
}

fn selectable_state(state: &NodeScannerState) -> bool {
    matches!(
        state,
        NodeScannerState::Scanning { .. }
            | NodeScannerState::Stopped { .. }
            | NodeScannerState::Finished { .. }
    )
}

fn map_results(
    state: NodeScannerState,
    update: impl FnOnce(&mut Vec<NodeScannerResult>),
) -> Option<NodeScannerState> {
    let state = match state {
        NodeScannerState::Scanning { generation, network, progress, mut results } => {
            update(&mut results);
            NodeScannerState::Scanning { generation, network, progress, results }
        }
        NodeScannerState::Stopped { generation, network, mut results } => {
            update(&mut results);
            NodeScannerState::Stopped { generation, network, results }
        }
        NodeScannerState::Finished { generation, network, mut results } => {
            update(&mut results);
            NodeScannerState::Finished { generation, network, results }
        }
        NodeScannerState::Idle | NodeScannerState::Failed { .. } => return None,
    };

    Some(state)
}

fn upsert_result(results: &mut Vec<NodeScannerResult>, result: NodeScannerResult) {
    if let Some(existing) = results.iter_mut().find(|existing| existing.ip() == result.ip()) {
        *existing = result;
    } else {
        results.push(result);
    }
}

fn node_from_found(found: FoundNode, network: Network) -> Node {
    Node {
        tls: found.tls.clone(),
        ..Node::new_electrum(found.server_software.clone(), found.url(), network)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::manager::deferred_sender::SingleOrMany;

    fn found(generation: u64, ip: &str) -> FoundNode {
        FoundNode {
            scan_generation: generation,
            ip: ip.to_string(),
            port: 50001,
            transport: FoundNodeTransport::Tcp,
            server_software: "Fulcrum".to_string(),
            protocol_version: "1.4".to_string(),
            tls: None,
        }
    }

    fn candidate(generation: u64, ip: &str) -> TrustRequiredEndpoint {
        TrustRequiredEndpoint {
            scan_generation: generation,
            ip: ip.to_string(),
            port: 50002,
            certificate: crate::node_connect::NodeCertificate {
                sha256: vec![7; 32],
                display: "07".to_string(),
            },
        }
    }

    fn scanning(generation: u64, results: Vec<NodeScannerResult>) -> Arc<RwLock<NodeScannerState>> {
        Arc::new(RwLock::new(NodeScannerState::Scanning {
            generation,
            network: Network::Bitcoin,
            progress: NodeScanProgress { checked_hosts: 0, total_hosts: 10 },
            results,
        }))
    }

    fn actor_with_state(
        state: Arc<RwLock<NodeScannerState>>,
    ) -> (NodeScannerActor, ReconcileChannel<NodeScannerReconcileMessage>) {
        let channel = ReconcileChannel::new(32);
        let mut actor = NodeScannerActor::new(state, channel.clone());
        actor.active_generation = Some(7);
        actor.next_generation = 8;

        (actor, channel)
    }

    fn init_database() {
        static INIT: OnceLock<()> = OnceLock::new();

        INIT.get_or_init(|| {
            let root = std::env::temp_dir().join("cove-node-scanner-tests");
            cove_common::consts::set_root_data_dir(root).expect("set scanner test data directory");
            crate::database::test_support::init_test_database();
        });
    }

    #[tokio::test]
    async fn second_start_is_a_noop_while_scanning() {
        let state = scanning(7, Vec::new());
        let (mut actor, channel) = actor_with_state(state.clone());

        actor.start_scan().await.unwrap();

        assert_eq!(actor.next_generation, 8);
        assert_eq!(state.read().generation(), Some(7));
        assert!(channel.receiver().try_recv().is_err());
    }

    #[tokio::test]
    async fn stop_retains_results_and_rejects_post_cancel_events() {
        let result = NodeScannerResult::Verified(found(7, "192.168.1.2"));
        let state = scanning(7, vec![result.clone()]);
        let (mut actor, channel) = actor_with_state(state.clone());
        let cancel = CancellationToken::new();
        actor.active_cancel = Some(cancel.clone());

        actor.stop_scan().await.unwrap();

        assert!(cancel.is_cancelled());
        assert_eq!(
            *state.read(),
            NodeScannerState::Stopped {
                generation: 7,
                network: Network::Bitcoin,
                results: vec![result]
            }
        );
        assert!(matches!(
            channel.receiver().recv().unwrap(),
            SingleOrMany::Single(NodeScannerReconcileMessage::ScanStopped { generation: 7 })
        ));

        actor
            .handle_scan_event(
                7,
                ScanEvent::Result {
                    result: NodeScannerResult::Verified(found(7, "192.168.1.3")),
                    tcp_fallback: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(state.read().results().len(), 1);
    }

    #[tokio::test]
    async fn finish_and_failure_are_terminal_and_retain_results() {
        let result = NodeScannerResult::Verified(found(7, "192.168.1.2"));
        let state = scanning(7, vec![result.clone()]);
        let (mut actor, _) = actor_with_state(state.clone());

        actor.handle_scan_event(7, ScanEvent::Finished(Default::default())).await.unwrap();
        assert_eq!(
            *state.read(),
            NodeScannerState::Finished {
                generation: 7,
                network: Network::Bitcoin,
                results: vec![result.clone()],
            }
        );

        let state = scanning(7, vec![result.clone()]);
        let (mut actor, _) = actor_with_state(state.clone());
        let error = NodeScannerError::ScanFailed("fixture".to_string());

        actor.handle_scan_event(7, ScanEvent::Failed(error.clone())).await.unwrap();
        assert_eq!(
            *state.read(),
            NodeScannerState::Failed {
                generation: 7,
                network: Network::Bitcoin,
                error,
                results: vec![result],
            }
        );
    }

    #[tokio::test]
    async fn progress_never_regresses() {
        let state = scanning(7, Vec::new());
        let (mut actor, _) = actor_with_state(state.clone());

        actor
            .handle_scan_event(
                7,
                ScanEvent::Progress(NodeScanProgress { checked_hosts: 8, total_hosts: 10 }),
            )
            .await
            .unwrap();
        actor
            .handle_scan_event(
                7,
                ScanEvent::Progress(NodeScanProgress { checked_hosts: 4, total_hosts: 10 }),
            )
            .await
            .unwrap();

        assert!(matches!(
            *state.read(),
            NodeScannerState::Scanning {
                progress: NodeScanProgress { checked_hosts: 8, total_hosts: 10 },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn stale_generation_events_cannot_mutate_a_new_scan() {
        let state = scanning(7, Vec::new());
        let (mut actor, channel) = actor_with_state(state.clone());

        actor
            .handle_scan_event(
                6,
                ScanEvent::Result {
                    result: NodeScannerResult::Verified(found(6, "192.168.1.2")),
                    tcp_fallback: false,
                },
            )
            .await
            .unwrap();

        assert!(state.read().results().is_empty());
        assert!(channel.receiver().try_recv().is_err());
    }

    #[tokio::test]
    async fn snapshot_is_written_before_result_delta() {
        let state = scanning(7, Vec::new());
        let (mut actor, channel) = actor_with_state(state.clone());
        let result = NodeScannerResult::Verified(found(7, "192.168.1.2"));

        actor
            .handle_scan_event(7, ScanEvent::Result { result: result.clone(), tcp_fallback: false })
            .await
            .unwrap();
        let message = channel.receiver().recv().unwrap();

        assert_eq!(state.read().results(), std::slice::from_ref(&result));
        assert_eq!(
            message,
            SingleOrMany::Single(NodeScannerReconcileMessage::ResultUpserted {
                generation: 7,
                result,
            })
        );
    }

    #[tokio::test]
    async fn result_upsert_keeps_one_result_per_host() {
        let pending = NodeScannerResult::TrustRequired(candidate(7, "192.168.1.2"));
        let state = scanning(7, vec![pending]);
        let (mut actor, _) = actor_with_state(state.clone());
        let verified = NodeScannerResult::Verified(found(7, "192.168.1.2"));

        actor
            .handle_scan_event(
                7,
                ScanEvent::Result { result: verified.clone(), tcp_fallback: true },
            )
            .await
            .unwrap();

        assert_eq!(state.read().results(), std::slice::from_ref(&verified));
        assert!(actor.tcp_fallback_hosts.contains("192.168.1.2"));
    }

    #[test]
    fn generations_are_monotonic_and_rescan_state_can_start_clean() {
        let state = Arc::new(RwLock::new(NodeScannerState::Finished {
            generation: 7,
            network: Network::Bitcoin,
            results: vec![NodeScannerResult::Verified(found(7, "192.168.1.2"))],
        }));
        let (mut actor, _) = actor_with_state(state.clone());

        assert_eq!(actor.claim_generation(), 8);
        assert_eq!(actor.claim_generation(), 9);

        *state.write() = NodeScannerState::Scanning {
            generation: 9,
            network: Network::Bitcoin,
            progress: NodeScanProgress { checked_hosts: 0, total_hosts: 10 },
            results: Vec::new(),
        };
        assert!(state.read().results().is_empty());
    }

    #[test]
    fn drop_cancels_active_work() {
        let state = scanning(7, Vec::new());
        let (mut actor, _) = actor_with_state(state);
        let cancel = CancellationToken::new();
        actor.active_cancel = Some(cancel.clone());

        drop(actor);

        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn exact_verified_members_create_without_persisting_and_stop_atomically() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_database();
        let database = Database::global();
        database.global_config.set_selected_network(Network::Bitcoin).unwrap();
        let selected_before = database.global_config.selected_node();
        let verified = found(7, "192.168.1.2");

        for state in [
            scanning(7, vec![NodeScannerResult::Verified(verified.clone())]),
            Arc::new(RwLock::new(NodeScannerState::Stopped {
                generation: 7,
                network: Network::Bitcoin,
                results: vec![NodeScannerResult::Verified(verified.clone())],
            })),
            Arc::new(RwLock::new(NodeScannerState::Finished {
                generation: 7,
                network: Network::Bitcoin,
                results: vec![NodeScannerResult::Verified(verified.clone())],
            })),
        ] {
            let (mut actor, _) = actor_with_state(state.clone());
            let node = actor.create_node(verified.clone()).await.unwrap().await.unwrap().unwrap();

            assert_eq!(node.url, "tcp://192.168.1.2:50001");
            assert_eq!(node.name, "Fulcrum");
            assert_eq!(node.tls, None);
            if matches!(*state.read(), NodeScannerState::Stopped { .. }) {
                assert_eq!(state.read().results().len(), 1);
            }
        }

        assert_eq!(database.global_config.selected_node(), selected_before);
    }

    #[tokio::test]
    async fn fabricated_stale_and_changed_network_results_are_rejected() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_database();
        let database = Database::global();
        database.global_config.set_selected_network(Network::Bitcoin).unwrap();
        let verified = found(7, "192.168.1.2");
        let state = scanning(7, vec![NodeScannerResult::Verified(verified.clone())]);
        let (mut actor, _) = actor_with_state(state);

        assert_eq!(
            actor.create_node(found(7, "192.168.1.3")).await.unwrap().await.unwrap(),
            Err(NodeScannerError::ResultUnavailable)
        );
        assert_eq!(
            actor.create_node(found(6, "192.168.1.2")).await.unwrap().await.unwrap(),
            Err(NodeScannerError::ResultUnavailable)
        );

        database.global_config.set_selected_network(Network::Testnet).unwrap();
        assert_eq!(
            actor.create_node(verified).await.unwrap().await.unwrap(),
            Err(NodeScannerError::NetworkChanged)
        );
        database.global_config.set_selected_network(Network::Bitcoin).unwrap();
    }

    #[tokio::test]
    async fn confirmed_promotion_can_fall_back_to_verified_tcp_and_failure_removes_pending() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_database();
        Database::global().global_config.set_selected_network(Network::Bitcoin).unwrap();
        let pending = candidate(7, "192.168.1.2");
        let state = Arc::new(RwLock::new(NodeScannerState::Stopped {
            generation: 7,
            network: Network::Bitcoin,
            results: vec![NodeScannerResult::TrustRequired(pending.clone())],
        }));
        let (mut actor, _) = actor_with_state(state.clone());
        actor.tcp_fallback_hosts.insert(pending.ip.clone());
        let promotion =
            actor.prepare_trust_promotion(pending.clone()).await.unwrap().await.unwrap().unwrap();
        assert!(promotion.tcp_fallback);

        let node = actor
            .complete_trust_promotion(
                promotion,
                Ok(PromotionVerification {
                    metadata: VerifiedMetadata {
                        server_software: "Fulcrum".to_string(),
                        protocol_version: "1.4".to_string(),
                    },
                    transport: FoundNodeTransport::Tcp,
                }),
            )
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap();

        assert_eq!(node.url, "tcp://192.168.1.2:50001");
        assert_eq!(node.tls, None);
        assert!(matches!(state.read().results(), [NodeScannerResult::Verified(_)]));

        let pending = candidate(7, "192.168.1.3");
        let state = Arc::new(RwLock::new(NodeScannerState::Finished {
            generation: 7,
            network: Network::Bitcoin,
            results: vec![NodeScannerResult::TrustRequired(pending.clone())],
        }));
        let (mut actor, channel) = actor_with_state(state.clone());
        let promotion =
            actor.prepare_trust_promotion(pending).await.unwrap().await.unwrap().unwrap();
        let result = actor
            .complete_trust_promotion(promotion, Err(VerifyError::CertificateChanged))
            .await
            .unwrap()
            .await
            .unwrap();

        assert!(matches!(result, Err(NodeScannerError::ScanFailed(_))));
        assert!(state.read().results().is_empty());
        assert!(matches!(
            channel.receiver().recv().unwrap(),
            SingleOrMany::Single(NodeScannerReconcileMessage::ResultRemoved { .. })
        ));
    }
}

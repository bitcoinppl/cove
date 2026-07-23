mod actor;

use std::sync::Arc;

use act_zero::{Addr, call, send};
use cove_util::ResultExt as _;
use parking_lot::RwLock;

use crate::manager::deferred_sender::SingleOrMany;
use crate::manager::reconcile_channel::ReconcileChannel;
use crate::node::Node;
use crate::node_scanner::verify::{self, VerifyError};
use crate::node_scanner::{
    FoundNode, NodeScannerError, NodeScannerReconcileMessage, NodeScannerState,
    TrustRequiredEndpoint,
};

use actor::{NodeScannerActor, TrustPromotion};

type Reconciler = dyn NodeScannerReconciler;

/// Receives ordered local-node scanner deltas
#[uniffi::export(callback_interface)]
pub trait NodeScannerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Applies one scanner delta
    fn reconcile(&self, message: NodeScannerReconcileMessage);
    /// Applies an ordered batch of scanner deltas
    fn reconcile_many(&self, messages: Vec<NodeScannerReconcileMessage>);
}

/// Actor-backed owner of one local-node discovery lifecycle
#[derive(Debug, uniffi::Object)]
pub struct RustNodeScannerManager {
    actor: Addr<NodeScannerActor>,
    state: Arc<RwLock<NodeScannerState>>,
    reconciler: ReconcileChannel<NodeScannerReconcileMessage>,
}

#[uniffi::export(async_runtime = "tokio")]
impl RustNodeScannerManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        let state = Arc::new(RwLock::new(NodeScannerState::Idle));
        let reconciler = ReconcileChannel::new(1024);
        let actor =
            cove_tokio::task::spawn_actor(NodeScannerActor::new(state.clone(), reconciler.clone()));

        Self { actor, state, reconciler }
    }

    /// Starts forwarding typed scanner deltas to the frontend
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen_async(move |messages| match messages {
            SingleOrMany::Single(message) => reconciler.reconcile(message),
            SingleOrMany::Many(messages) => reconciler.reconcile_many(messages),
        });
    }

    /// Starts a new generation unless a scan is already active
    pub async fn start_scan(&self) {
        let _ = call!(self.actor.start_scan()).await;
    }

    /// Stops the active scan while retaining its current results
    pub async fn stop_scan(&self) {
        let _ = call!(self.actor.stop_scan()).await;
    }

    /// Returns the atomic bootstrap snapshot
    pub fn state(&self) -> NodeScannerState {
        self.state.read().clone()
    }

    /// Creates an unpersisted node from an exact verified result member
    pub async fn create_node(&self, found: FoundNode) -> Result<Node, NodeScannerError> {
        call!(self.actor.create_node(found)).await.map_err_str(NodeScannerError::ScanFailed)?
    }

    /// Pins, re-verifies, and creates an unpersisted node from an exact pending member
    pub async fn trust_and_create_node(
        &self,
        candidate: TrustRequiredEndpoint,
    ) -> Result<Node, NodeScannerError> {
        let promotion = call!(self.actor.prepare_trust_promotion(candidate))
            .await
            .map_err_str(NodeScannerError::ScanFailed)??;
        let verification = verify_promotion(&promotion).await;

        call!(self.actor.complete_trust_promotion(promotion, verification))
            .await
            .map_err_str(NodeScannerError::ScanFailed)?
    }
}

impl Default for RustNodeScannerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RustNodeScannerManager {
    fn drop(&mut self) {
        send!(self.actor.shutdown());
    }
}

async fn verify_promotion(
    promotion: &TrustPromotion,
) -> Result<actor::PromotionVerification, VerifyError> {
    let url = promotion.candidate.url();
    let network = promotion.network;
    let trust = promotion.trust.clone();
    let work =
        cove_tokio::unblock::run_blocking(move || verify::verify(&url, network, Some(&trust)));

    let tls_result = tokio::time::timeout(verify::VERIFICATION_OBSERVER_TIMEOUT, work)
        .await
        .map_err(|_| VerifyError::TimedOut)?;

    match tls_result {
        Ok(metadata) => Ok(actor::PromotionVerification {
            metadata,
            transport: crate::node_scanner::FoundNodeTransport::Tls,
        }),
        Err(error @ VerifyError::CertificateChanged) => Err(error),
        Err(error) if !promotion.tcp_fallback => Err(error),
        Err(_) => {
            let url = format!("tcp://{}:50001", promotion.candidate.ip);
            let network = promotion.network;
            let work =
                cove_tokio::unblock::run_blocking(move || verify::verify(&url, network, None));
            let metadata = tokio::time::timeout(verify::VERIFICATION_OBSERVER_TIMEOUT, work)
                .await
                .map_err(|_| VerifyError::TimedOut)??;

            Ok(actor::PromotionVerification {
                metadata,
                transport: crate::node_scanner::FoundNodeTransport::Tcp,
            })
        }
    }
}

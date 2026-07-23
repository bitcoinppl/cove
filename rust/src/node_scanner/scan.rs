use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt as _, stream};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::node::tls::TlsTrust;
use crate::node_scanner::verify::{self, VerifiedMetadata, VerifyError};
use crate::node_scanner::{
    FoundNode, FoundNodeTransport, NodeScanProgress, NodeScannerError, NodeScannerResult,
    ScanEvent, ScanSummary, TrustRequiredEndpoint,
};

const TCP_PORT: u16 = 50001;
const TLS_PORT: u16 = 50002;
const HOST_CONCURRENCY: usize = 64;
const VERIFY_CONCURRENCY: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const SCAN_WATCHDOG: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: u32 = 8;

#[derive(Debug, Clone)]
pub(crate) struct ScanContext {
    pub(crate) generation: u64,
    pub(crate) network: cove_types::network::Network,
    pub(crate) selected_tls_url: Option<String>,
    pub(crate) selected_tls_trust: Option<TlsTrust>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ProbeStats {
    successful_connects: u32,
    failed_probes: u32,
    host_unreachable_probes: u32,
}

impl std::ops::AddAssign for ProbeStats {
    fn add_assign(&mut self, rhs: Self) {
        self.successful_connects += rhs.successful_connects;
        self.failed_probes += rhs.failed_probes;
        self.host_unreachable_probes += rhs.host_unreachable_probes;
    }
}

#[derive(Debug)]
struct HostOutcome {
    result: Option<NodeScannerResult>,
    stats: ProbeStats,
    tcp_connected: bool,
}

#[derive(Debug)]
enum ProbeOutcome {
    Connected,
    Failed { host_unreachable: bool },
}

enum TlsResolution {
    Result(NodeScannerResult),
    NoUsableEndpoint,
    CertificateChanged,
}

enum HostDecision {
    Result(NodeScannerResult),
    VerifyTcp,
    Ignore,
}

impl ProbeOutcome {
    const fn connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    const fn stats(&self) -> ProbeStats {
        match self {
            Self::Connected => {
                ProbeStats { successful_connects: 1, failed_probes: 0, host_unreachable_probes: 0 }
            }
            Self::Failed { host_unreachable } => ProbeStats {
                successful_connects: 0,
                failed_probes: 1,
                host_unreachable_probes: *host_unreachable as u32,
            },
        }
    }
}

pub(crate) async fn run(
    hosts: Vec<Ipv4Addr>,
    context: ScanContext,
    cancel: CancellationToken,
    events: flume::Sender<ScanEvent>,
) -> Option<ScanEvent> {
    let total_hosts = hosts.len() as u32;
    let future = run_inner(hosts, context, cancel.clone(), events, total_hosts);

    tokio::select! {
        () = cancel.cancelled() => None,
        event = watchdog(SCAN_WATCHDOG, future) => Some(event),
    }
}

async fn watchdog(duration: Duration, scan: impl Future<Output = ScanSummary>) -> ScanEvent {
    match tokio::time::timeout(duration, scan).await {
        Ok(summary) => terminal_event(summary),
        Err(_) => {
            ScanEvent::Failed(NodeScannerError::ScanFailed("scan watchdog expired".to_string()))
        }
    }
}

async fn run_inner(
    hosts: Vec<Ipv4Addr>,
    context: ScanContext,
    cancel: CancellationToken,
    events: flume::Sender<ScanEvent>,
    total_hosts: u32,
) -> ScanSummary {
    let verifier = Arc::new(Semaphore::new(VERIFY_CONCURRENCY));
    let mut outcomes = stream::iter(hosts)
        .map(|ip| resolve_host(ip, context.clone(), cancel.clone(), verifier.clone()))
        .buffer_unordered(HOST_CONCURRENCY);

    let mut checked_hosts = 0;
    let mut stats = ProbeStats::default();
    while let Some(outcome) = outcomes.next().await {
        if cancel.is_cancelled() {
            break;
        }

        checked_hosts += 1;
        stats += outcome.stats;

        if let Some(result) = outcome.result {
            let _ = events
                .send_async(ScanEvent::Result { result, tcp_fallback: outcome.tcp_connected })
                .await;
        }

        if checked_hosts % PROGRESS_INTERVAL == 0 || checked_hosts == total_hosts {
            let progress = NodeScanProgress { checked_hosts, total_hosts };
            let _ = events.send_async(ScanEvent::Progress(progress)).await;
        }
    }

    ScanSummary {
        successful_connects: stats.successful_connects,
        failed_probes: stats.failed_probes,
        host_unreachable_probes: stats.host_unreachable_probes,
    }
}

fn terminal_event(summary: ScanSummary) -> ScanEvent {
    #[cfg(target_os = "ios")]
    if summary.permission_denied() {
        return ScanEvent::Failed(NodeScannerError::LocalNetworkPermissionDenied);
    }

    ScanEvent::Finished(summary)
}

async fn resolve_host(
    ip: Ipv4Addr,
    context: ScanContext,
    cancel: CancellationToken,
    verifier: Arc<Semaphore>,
) -> HostOutcome {
    let (tcp, tls) = tokio::join!(probe(ip, TCP_PORT), probe(ip, TLS_PORT));
    let mut stats = tcp.stats();
    stats += tls.stats();

    if cancel.is_cancelled() {
        return HostOutcome { result: None, stats, tcp_connected: tcp.connected() };
    }

    let permit = tokio::select! {
        () = cancel.cancelled() => return HostOutcome {
            result: None,
            stats,
            tcp_connected: tcp.connected(),
        },
        permit = verifier.acquire_owned() => permit.expect("scanner semaphore remains open"),
    };

    let tls_resolution = if tls.connected() {
        resolve_tls(ip, &context, &cancel, &permit).await
    } else {
        TlsResolution::NoUsableEndpoint
    };

    let result = match choose_host_result(tls_resolution, tcp.connected()) {
        HostDecision::Result(result) => Some(result),
        HostDecision::VerifyTcp => {
            verify_found(ip, TCP_PORT, FoundNodeTransport::Tcp, None, &context, &cancel, &permit)
                .await
                .ok()
                .map(NodeScannerResult::Verified)
        }
        HostDecision::Ignore => None,
    };

    HostOutcome { result, stats, tcp_connected: tcp.connected() }
}

fn choose_host_result(tls: TlsResolution, tcp_connected: bool) -> HostDecision {
    match tls {
        TlsResolution::Result(result) => HostDecision::Result(result),
        TlsResolution::CertificateChanged => HostDecision::Ignore,
        TlsResolution::NoUsableEndpoint if tcp_connected => HostDecision::VerifyTcp,
        TlsResolution::NoUsableEndpoint => HostDecision::Ignore,
    }
}

async fn resolve_tls(
    ip: Ipv4Addr,
    context: &ScanContext,
    cancel: &CancellationToken,
    permit: &OwnedSemaphorePermit,
) -> TlsResolution {
    let url = format!("ssl://{ip}:{TLS_PORT}");
    let selected_trust = (context.selected_tls_url.as_deref() == Some(url.as_str()))
        .then(|| context.selected_tls_trust.clone())
        .flatten();

    if let Some(trust) = selected_trust {
        return match verify_found(
            ip,
            TLS_PORT,
            FoundNodeTransport::Tls,
            Some(trust),
            context,
            cancel,
            permit,
        )
        .await
        {
            Ok(node) => TlsResolution::Result(NodeScannerResult::Verified(node)),
            Err(VerifyError::CertificateChanged) => TlsResolution::CertificateChanged,
            Err(_) => TlsResolution::NoUsableEndpoint,
        };
    }

    match verify_found(ip, TLS_PORT, FoundNodeTransport::Tls, None, context, cancel, permit).await {
        Ok(node) => TlsResolution::Result(NodeScannerResult::Verified(node)),
        Err(VerifyError::CertificateRejected) => {
            let Ok(certificate) =
                run_blocking(cancel, move || verify::capture_certificate(&url)).await
            else {
                return TlsResolution::NoUsableEndpoint;
            };

            TlsResolution::Result(NodeScannerResult::TrustRequired(TrustRequiredEndpoint {
                scan_generation: context.generation,
                ip: ip.to_string(),
                port: TLS_PORT,
                certificate,
            }))
        }
        Err(_) => TlsResolution::NoUsableEndpoint,
    }
}

async fn verify_found(
    ip: Ipv4Addr,
    port: u16,
    transport: FoundNodeTransport,
    tls: Option<TlsTrust>,
    context: &ScanContext,
    cancel: &CancellationToken,
    _permit: &OwnedSemaphorePermit,
) -> Result<FoundNode, VerifyError> {
    let scheme = match transport {
        FoundNodeTransport::Tcp => "tcp",
        FoundNodeTransport::Tls => "ssl",
    };
    let url = format!("{scheme}://{ip}:{port}");
    let network = context.network;
    let verification_tls = tls.clone();
    let metadata =
        run_blocking(cancel, move || verify::verify(&url, network, verification_tls.as_ref()))
            .await?;

    Ok(found_node(context.generation, ip, port, transport, tls, metadata))
}

async fn run_blocking<T: Send + 'static>(
    cancel: &CancellationToken,
    work: impl FnOnce() -> Result<T, VerifyError> + Send + 'static,
) -> Result<T, VerifyError> {
    let work = cove_tokio::unblock::run_blocking(work);

    tokio::select! {
        () = cancel.cancelled() => Err(VerifyError::TimedOut),
        result = tokio::time::timeout(verify::VERIFICATION_OBSERVER_TIMEOUT, work) => {
            result.map_err(|_| VerifyError::TimedOut)?
        }
    }
}

fn found_node(
    generation: u64,
    ip: Ipv4Addr,
    port: u16,
    transport: FoundNodeTransport,
    tls: Option<TlsTrust>,
    metadata: VerifiedMetadata,
) -> FoundNode {
    FoundNode {
        scan_generation: generation,
        ip: ip.to_string(),
        port,
        transport,
        server_software: metadata.server_software,
        protocol_version: metadata.protocol_version,
        tls,
    }
}

async fn probe(ip: Ipv4Addr, port: u16) -> ProbeOutcome {
    let address = SocketAddr::new(IpAddr::V4(ip), port);

    match tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(address)).await {
        Ok(Ok(_)) => ProbeOutcome::Connected,
        Ok(Err(error)) => ProbeOutcome::Failed {
            host_unreachable: error.raw_os_error() == Some(65)
                || error.kind() == std::io::ErrorKind::HostUnreachable,
        },
        Err(_) => ProbeOutcome::Failed { host_unreachable: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_denial_requires_zero_connects_and_ninety_percent_unreachable() {
        assert!(
            ScanSummary { successful_connects: 0, failed_probes: 10, host_unreachable_probes: 9 }
                .permission_denied()
        );
        assert!(
            !ScanSummary { successful_connects: 1, failed_probes: 10, host_unreachable_probes: 10 }
                .permission_denied()
        );
        assert!(
            !ScanSummary { successful_connects: 0, failed_probes: 10, host_unreachable_probes: 8 }
                .permission_denied()
        );
    }

    #[test]
    fn progress_interval_is_bounded_and_includes_completion() {
        let emitted = (1..=19)
            .filter(|checked| checked % PROGRESS_INTERVAL == 0 || *checked == 19)
            .collect::<Vec<_>>();

        assert_eq!(emitted, vec![8, 16, 19]);
    }

    #[test]
    fn tls_results_win_and_tcp_is_only_a_fallback() {
        let tls = NodeScannerResult::Verified(found_node(
            1,
            Ipv4Addr::new(192, 168, 1, 2),
            TLS_PORT,
            FoundNodeTransport::Tls,
            None,
            VerifiedMetadata {
                server_software: "Fulcrum".to_string(),
                protocol_version: "1.4".to_string(),
            },
        ));

        assert!(matches!(
            choose_host_result(TlsResolution::Result(tls.clone()), true),
            HostDecision::Result(result) if result == tls
        ));
        assert!(matches!(
            choose_host_result(TlsResolution::NoUsableEndpoint, true),
            HostDecision::VerifyTcp
        ));
        assert!(matches!(
            choose_host_result(TlsResolution::NoUsableEndpoint, false),
            HostDecision::Ignore
        ));
        assert!(matches!(
            choose_host_result(TlsResolution::CertificateChanged, true),
            HostDecision::Ignore
        ));
    }

    #[test]
    fn unknown_certificate_remains_the_only_result_for_its_host() {
        let pending = NodeScannerResult::TrustRequired(TrustRequiredEndpoint {
            scan_generation: 1,
            ip: "192.168.1.2".to_string(),
            port: TLS_PORT,
            certificate: crate::node_connect::NodeCertificate {
                sha256: vec![1; 32],
                display: "01".to_string(),
            },
        });

        assert!(matches!(
            choose_host_result(TlsResolution::Result(pending.clone()), true),
            HostDecision::Result(result) if result == pending
        ));
    }

    #[tokio::test]
    async fn watchdog_fails_a_stalled_scan() {
        let event = watchdog(Duration::from_millis(1), std::future::pending()).await;

        assert!(matches!(event, ScanEvent::Failed(NodeScannerError::ScanFailed(_))));
    }
}

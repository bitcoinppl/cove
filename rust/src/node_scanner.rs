//! Local-network Electrum discovery domain and scan pipeline

pub(crate) mod scan;
pub(crate) mod subnet;
pub(crate) mod verify;

use cove_types::network::Network;

use crate::node::tls::TlsTrust;
use crate::node_connect::NodeCertificate;

/// Transport used by a verified discovered node
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum FoundNodeTransport {
    /// Unencrypted Electrum TCP
    Tcp,
    /// Certificate-verified Electrum TLS
    Tls,
}

/// A discovered Electrum server whose network has been verified
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct FoundNode {
    /// Scan generation that proved this result
    pub scan_generation: u64,
    /// Discovered IPv4 address
    pub ip: String,
    /// Discovered Electrum port
    pub port: u16,
    /// Verified connection transport
    pub transport: FoundNodeTransport,
    /// Software label reported by the server
    pub server_software: String,
    /// Electrum protocol version reported or negotiated by the server
    pub protocol_version: String,
    /// Confirmed custom trust required to reconnect, when applicable
    pub tls: Option<TlsTrust>,
}

impl FoundNode {
    pub(crate) fn url(&self) -> String {
        let scheme = match self.transport {
            FoundNodeTransport::Tcp => "tcp",
            FoundNodeTransport::Tls => "ssl",
        };

        format!("{scheme}://{}:{}", self.ip, self.port)
    }
}

/// A TLS endpoint awaiting explicit fingerprint confirmation
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TrustRequiredEndpoint {
    /// Scan generation that captured this endpoint
    pub scan_generation: u64,
    /// Discovered IPv4 address
    pub ip: String,
    /// TLS port that presented the certificate
    pub port: u16,
    /// Untrusted leaf certificate shown for explicit confirmation
    pub certificate: NodeCertificate,
}

impl TrustRequiredEndpoint {
    pub(crate) fn url(&self) -> String {
        format!("ssl://{}:{}", self.ip, self.port)
    }
}

/// Result produced for one scanned host
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum NodeScannerResult {
    /// Network-verified selectable result
    Verified(FoundNode),
    /// Unverified TLS endpoint awaiting fingerprint confirmation
    TrustRequired(TrustRequiredEndpoint),
}

impl NodeScannerResult {
    pub(crate) fn ip(&self) -> &str {
        match self {
            Self::Verified(node) => &node.ip,
            Self::TrustRequired(endpoint) => &endpoint.ip,
        }
    }
}

/// Host-level progress for a local-node scan
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct NodeScanProgress {
    /// Hosts whose dual-port work item has completed
    pub checked_hosts: u32,
    /// Hosts selected for this bounded generation
    pub total_hosts: u32,
}

/// Atomic snapshot of the local-node scanner lifecycle
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum NodeScannerState {
    /// No generation has started
    Idle,
    /// A generation is actively scanning
    Scanning {
        /// Active scan generation
        generation: u64,
        /// Network every result must prove
        network: Network,
        /// Current bounded host progress
        progress: NodeScanProgress,
        /// Current one-per-host results
        results: Vec<NodeScannerResult>,
    },
    /// A user-stopped generation with retained results
    Stopped {
        /// Retained scan generation
        generation: u64,
        /// Network every retained result proved or awaits proving
        network: Network,
        /// Results retained when scanning stopped
        results: Vec<NodeScannerResult>,
    },
    /// A successfully completed generation
    Finished {
        /// Completed scan generation
        generation: u64,
        /// Network every result proved or awaits proving
        network: Network,
        /// Final one-per-host results
        results: Vec<NodeScannerResult>,
    },
    /// A failed generation with any results found before failure
    Failed {
        /// Failed scan generation
        generation: u64,
        /// Network the failed generation scanned
        network: Network,
        /// Terminal failure
        error: NodeScannerError,
        /// Results found before the failure
        results: Vec<NodeScannerResult>,
    },
}

impl NodeScannerState {
    pub(crate) const fn generation(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Scanning { generation, .. }
            | Self::Stopped { generation, .. }
            | Self::Finished { generation, .. }
            | Self::Failed { generation, .. } => Some(*generation),
        }
    }

    pub(crate) const fn network(&self) -> Option<Network> {
        match self {
            Self::Idle => None,
            Self::Scanning { network, .. }
            | Self::Stopped { network, .. }
            | Self::Finished { network, .. }
            | Self::Failed { network, .. } => Some(*network),
        }
    }

    pub(crate) fn results(&self) -> &[NodeScannerResult] {
        match self {
            Self::Idle => &[],
            Self::Scanning { results, .. }
            | Self::Stopped { results, .. }
            | Self::Finished { results, .. }
            | Self::Failed { results, .. } => results,
        }
    }
}

/// Failure reported by local-node discovery or result creation
#[derive(Debug, Clone, Hash, Eq, PartialEq, thiserror::Error, uniffi::Error)]
pub enum NodeScannerError {
    /// No eligible RFC1918 interface or fallback address exists
    #[error("no eligible local network was found")]
    NoLocalNetwork,
    /// iOS denied local-network access
    #[error("local network access was denied")]
    LocalNetworkPermissionDenied,
    /// An unexpected scan or verification failure
    #[error("local node scan failed: {0}")]
    ScanFailed(String),
    /// The supplied value is not an exact current result member
    #[error("the selected discovery result is no longer available")]
    ResultUnavailable,
    /// The selected Bitcoin network differs from the scan network
    #[error("the selected Bitcoin network changed during discovery")]
    NetworkChanged,
}

/// Typed delta emitted after the scanner snapshot has been updated
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum NodeScannerReconcileMessage {
    /// A new generation replaced all previous results
    ScanStarted { generation: u64, network: Network, progress: NodeScanProgress },
    /// Host progress advanced
    ScanProgress { generation: u64, progress: NodeScanProgress },
    /// One host result was inserted or replaced
    ResultUpserted { generation: u64, result: NodeScannerResult },
    /// One host result became unusable and was removed
    ResultRemoved { generation: u64, ip: String },
    /// The user stopped the generation
    ScanStopped { generation: u64 },
    /// The generation completed
    ScanFinished { generation: u64 },
    /// The generation failed
    ScanFailed { generation: u64, error: NodeScannerError },
}

#[derive(Debug)]
pub(crate) enum ScanEvent {
    Progress(NodeScanProgress),
    Result { result: NodeScannerResult, tcp_fallback: bool },
    Finished(ScanSummary),
    Failed(NodeScannerError),
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct ScanSummary {
    pub(crate) successful_connects: u32,
    pub(crate) failed_probes: u32,
    pub(crate) host_unreachable_probes: u32,
}

#[cfg(any(test, target_os = "ios"))]
impl ScanSummary {
    pub(crate) fn permission_denied(self) -> bool {
        self.successful_connects == 0
            && self.failed_probes > 0
            && u64::from(self.host_unreachable_probes) * 10 >= u64::from(self.failed_probes) * 9
    }
}

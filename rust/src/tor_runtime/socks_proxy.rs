//! Minimal SOCKS5 CONNECT proxy for the built-in Tor runtime
//!
//! Every byte of SOCKS protocol handling comes from `tor-socksproto` and every
//! byte of relaying from `futures-copy`. The accept loop, handshake driver,
//! address-family preferences, isolation key, and `ErrorKind` reply mapping are
//! adapted from arti 2.5.0 (`src/proxy.rs` and `src/proxy/socks.rs`),
//! Copyright (c) The Tor Project, Inc., licensed MIT OR Apache-2.0.
//!
//! Cove serves SOCKS5 CONNECT only: SOCKS4, RESOLVE, RESOLVE_PTR, BIND, and
//! UDP ASSOCIATE are all refused, which upstream arti supports.

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};

use arti_client::{
    ErrorKind, HasKind as _, IntoTorAddr as _, StreamPrefs, TorClient, isolation::IsolationHelper,
};
use cove_util::result_ext::ResultExt as _;
use futures::{
    AsyncReadExt as _, AsyncWriteExt as _, StreamExt as _,
    io::{AsyncRead, AsyncWrite, BufReader},
};
use tor_rtcompat::{
    NetStreamListener as _, NetStreamProvider, PreferredRuntime, SleepProvider as _,
    SleepProviderExt as _, SpawnExt as _,
};
use tor_socksproto::{
    Buffer, Handshake as _, NextStep, SocksAuth, SocksCmd, SocksProxyHandshake, SocksRequest,
    SocksStatus, SocksVersion,
};
use tracing::debug;

use super::Error;

/// Read buffer applied to both sides of a relayed connection, mirroring arti
const APP_STREAM_BUF_LEN: usize = 4096;

/// Pause between accepts while the process is out of file descriptors
///
/// Retrying immediately would spin a whole core until some other task closes a
/// descriptor, which on a phone is a battery and thermal problem, not just noise
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Bound on completing a SOCKS handshake
///
/// Any local process can open a loopback connection; one that connects and then
/// says nothing must not hold a task and a descriptor for the runtime's lifetime
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

const SOCKS4_VERSION: u8 = 4;
const SOCKS5_VERSION: u8 = 5;

/// SOCKS5 method-selection reply meaning "no acceptable authentication methods"
const SOCKS5_NO_ACCEPTABLE_METHODS: [u8; 2] = [SOCKS5_VERSION, 0xFF];

/// SOCKS5 COMMAND_NOT_SUPPORTED reply with an unspecified IPv4 bind address
const SOCKS5_COMMAND_NOT_SUPPORTED: [u8; 10] = [SOCKS5_VERSION, 0x07, 0, 1, 0, 0, 0, 0, 0, 0];

/// SOCKS4 "request rejected or failed" reply with a zeroed bind address
const SOCKS4_REJECTED: [u8; 8] = [0, 91, 0, 0, 0, 0, 0, 0];

type Listener = <PreferredRuntime as NetStreamProvider>::Listener;

/// Why one proxied connection ended
///
/// Never rendered to the user and never carries a proxied address: the target of
/// a local request is the user's browsing history, not diagnostic material
#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    /// The peer did not complete a SOCKS handshake
    #[error("SOCKS handshake failed: {0}")]
    Handshake(#[from] tor_socksproto::Error),

    /// The peer did not complete its SOCKS handshake inside the bound
    #[error("SOCKS handshake timed out")]
    HandshakeTimeout,

    /// Reading from or writing to the local peer failed
    #[error("proxy connection input/output failed: {0}")]
    Io(#[from] io::Error),

    /// A SOCKS reply could not be encoded
    #[error("failed to encode a SOCKS reply: {0}")]
    EncodeReply(String),

    /// The request was well-formed but is not one Cove serves
    #[error("refused an unsupported SOCKS request: {0}")]
    Refused(SocksStatus),

    /// The requested target is not a usable Tor address
    #[error("the requested target address is not usable")]
    TargetAddress,

    /// Tor could not open a stream to the requested target
    #[error("failed to open a Tor stream: {0}")]
    Connect(#[source] arti_client::Error),
}

/// Accepts and serves SOCKS connections until the listener stream ends
///
/// Returns `Ok(())` only when the listener itself stops yielding connections;
/// accept and spawn failures that leave the proxy unable to serve are fatal
pub(crate) async fn run(
    client: Arc<TorClient<PreferredRuntime>>,
    listener: Listener,
) -> Result<(), Error> {
    let mut incoming = listener.incoming();

    while let Some(accepted) = incoming.next().await {
        let stream = match accepted {
            Ok((stream, _peer)) => stream,
            Err(error) => match accept_failure(&error) {
                AcceptFailure::Fatal => return Err(Error::Proxy(error.to_string())),
                AcceptFailure::DescriptorsExhausted => {
                    debug!("built-in Tor proxy ran out of file descriptors: {error}");
                    client.runtime().sleep(ACCEPT_BACKOFF).await;

                    continue;
                }
            },
        };

        let connection_client = client.clone();
        let spawned = client.runtime().spawn(async move {
            if let Err(error) = handle_connection(connection_client, stream).await {
                debug!("built-in Tor proxy connection closed: {error}");
            }
        });

        if let Err(error) = spawned {
            return Err(Error::Proxy(error.to_string()));
        }
    }

    Ok(())
}

/// Negotiates one connection and relays it over Tor until either side closes
async fn handle_connection<S>(
    client: Arc<TorClient<PreferredRuntime>>,
    mut stream: S,
) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let request = client
        .runtime()
        .timeout(HANDSHAKE_TIMEOUT, negotiate(&mut stream))
        .await
        .map_err(|_| ConnectionError::HandshakeTimeout)??;

    if let Err(status) = validate_request(&request) {
        refuse(&mut stream, &request, status).await;

        return Err(ConnectionError::Refused(status));
    }

    let addr = request.addr().to_string();
    let mut prefs = stream_prefs(&addr);
    prefs.set_isolation(SocksIsolationKey(request.auth().clone()));

    let Ok(target) = (addr, request.port()).into_tor_addr() else {
        refuse(&mut stream, &request, SocksStatus::GENERAL_FAILURE).await;

        return Err(ConnectionError::TargetAddress);
    };

    let tor_stream = match client.connect_with_prefs(&target, &prefs).await {
        Ok(tor_stream) => tor_stream,
        Err(error) => {
            refuse(&mut stream, &request, status_for_error(error.kind())).await;

            return Err(ConnectionError::Connect(error));
        }
    };

    let reply =
        request.reply(SocksStatus::SUCCEEDED, None).map_err_str(ConnectionError::EncodeReply)?;
    write_all_and_flush(&mut stream, &reply).await?;

    // eof::Close in both directions propagates a half-close: a local EOF closes
    // the Tor stream, which sends an END cell, and a Tor EOF closes the local side
    futures_copy::copy_buf_bidirectional(
        BufReader::with_capacity(APP_STREAM_BUF_LEN, stream),
        BufReader::with_capacity(APP_STREAM_BUF_LEN, tor_stream),
        futures_copy::eof::Close,
        futures_copy::eof::Close,
    )
    .await?;

    Ok(())
}

/// Drives `tor-socksproto` to a completed request
///
/// Handshake reads go straight into the protocol buffer, so no bytes can be
/// stranded in an intermediate reader once the relay phase begins
async fn negotiate<S>(stream: &mut S) -> Result<SocksRequest, ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut handshake = SocksProxyHandshake::new();
    let mut buffer = Buffer::new();

    // tracked so an unsupported request can be refused in the peer's own dialect
    let mut first_byte: Option<u8> = None;
    let mut method_negotiated = false;

    loop {
        let step = match handshake.step(&mut buffer) {
            Ok(step) => step,
            Err(error) => {
                refuse_unsupported(stream, &error, first_byte, method_negotiated).await;

                return Err(error.into());
            }
        };

        match step {
            NextStep::Recv(mut recv) => {
                let received = stream.read(recv.buf()).await?;
                if first_byte.is_none() && received > 0 {
                    first_byte = Some(recv.buf()[0]);
                }

                // a zero-length read is EOF, which note_received reports as an error
                recv.note_received(received)?;
            }

            NextStep::Send(data) => {
                write_all_and_flush(stream, &data).await?;
                method_negotiated = true;
            }

            NextStep::Finished(finished) => {
                return Ok(finished.into_output_forbid_pipelining()?);
            }
        }
    }
}

/// Cove serves SOCKS5 CONNECT only; `Err` carries the status to reply with
fn validate_request(request: &SocksRequest) -> Result<(), SocksStatus> {
    // SOCKS4 has no address type for onion services and no way to signal most
    // failures, so Cove refuses it rather than serving a degraded route
    if request.version() != SocksVersion::V5 {
        return Err(SocksStatus::GENERAL_FAILURE);
    }

    // RESOLVE and RESOLVE_PTR would answer DNS questions without opening a
    // stream, which no Cove client asks for
    if request.command() != SocksCmd::CONNECT {
        return Err(SocksStatus::COMMAND_NOT_SUPPORTED);
    }

    Ok(())
}

/// Refuses, best effort, the requests tor-socksproto rejects before yielding a request
///
/// A peer that is not recognizably speaking SOCKS is dropped in silence, like
/// upstream arti: there is no reply it would understand
async fn refuse_unsupported<S>(
    stream: &mut S,
    error: &tor_socksproto::Error,
    first_byte: Option<u8>,
    method_negotiated: bool,
) where
    S: AsyncWrite + Unpin,
{
    if !matches!(error, tor_socksproto::Error::NotImplemented(_)) {
        return;
    }

    let reply: &[u8] = match (first_byte, method_negotiated) {
        // before method negotiation the client offered no method we support
        (Some(SOCKS5_VERSION), false) => &SOCKS5_NO_ACCEPTABLE_METHODS,
        // after it, the command itself is unsupported: BIND or UDP ASSOCIATE
        (Some(SOCKS5_VERSION), true) => &SOCKS5_COMMAND_NOT_SUPPORTED,
        (Some(SOCKS4_VERSION), _) => &SOCKS4_REJECTED,
        _ => return,
    };

    let _ = write_all_and_close(stream, reply).await;
}

/// Address families Tor may use for a target, adapted from arti's `stream_preference`
fn stream_prefs(addr: &str) -> StreamPrefs {
    let mut prefs = StreamPrefs::new();

    if addr.parse::<Ipv4Addr>().is_ok() {
        prefs.ipv4_only();
    } else if addr.parse::<Ipv6Addr>().is_ok() {
        prefs.ipv6_only();
    } else {
        prefs.ipv4_preferred();
    }

    prefs
}

/// Maps an Arti failure onto the SOCKS status that explains it best
fn status_for_error(kind: ErrorKind) -> SocksStatus {
    match kind {
        ErrorKind::RemoteNetworkFailed => SocksStatus::TTL_EXPIRED,
        ErrorKind::OnionServiceNotFound => SocksStatus::HS_DESC_NOT_FOUND,
        ErrorKind::OnionServiceAddressInvalid => SocksStatus::HS_BAD_ADDRESS,
        ErrorKind::OnionServiceMissingClientAuth => SocksStatus::HS_MISSING_CLIENT_AUTH,
        ErrorKind::OnionServiceWrongClientAuth => SocksStatus::HS_WRONG_CLIENT_AUTH,
        ErrorKind::OnionServiceNotRunning
        | ErrorKind::OnionServiceConnectionFailed
        | ErrorKind::OnionServiceProtocolViolation => SocksStatus::HS_INTRO_FAILED,
        _ => SocksStatus::GENERAL_FAILURE,
    }
}

/// What an accept failure means for the accept loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcceptFailure {
    /// The listener can no longer serve connections
    Fatal,
    /// The process is out of file descriptors, which passes once tasks release theirs
    DescriptorsExhausted,
}

/// Classifies an accept failure
fn accept_failure(error: &io::Error) -> AcceptFailure {
    // EMFILE and ENFILE are transient descriptor exhaustion rather than a broken
    // listener, and std does not distinguish them by ErrorKind
    #[cfg(unix)]
    let exhausted = matches!(error.raw_os_error(), Some(libc::EMFILE | libc::ENFILE));

    #[cfg(not(unix))]
    let exhausted = false;

    if exhausted { AcceptFailure::DescriptorsExhausted } else { AcceptFailure::Fatal }
}

/// Refuses a request, best effort
///
/// A peer that has already gone away cannot be told why it was refused, and the
/// caller reports the real reason regardless of whether this reply lands
async fn refuse<S>(stream: &mut S, request: &SocksRequest, status: SocksStatus)
where
    S: AsyncWrite + Unpin,
{
    let Ok(reply) = request.reply(status, None) else { return };

    let _ = write_all_and_close(stream, &reply).await;
}

async fn write_all_and_flush<S>(stream: &mut S, data: &[u8]) -> Result<(), ConnectionError>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(data).await?;
    stream.flush().await?;

    Ok(())
}

async fn write_all_and_close<S>(stream: &mut S, data: &[u8]) -> Result<(), ConnectionError>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(data).await?;
    stream.close().await?;

    Ok(())
}

/// Circuit isolation key derived from the credentials a client presented
///
/// The username is an advisory label, not a security boundary: the loopback
/// listener is unauthenticated, so any local process can present any label. The
/// worst a hostile local app achieves is sharing one bucket, which is exactly
/// the behavior Cove had before isolation existed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SocksIsolationKey(SocksAuth);

impl IsolationHelper for SocksIsolationKey {
    fn compatible_same_type(&self, other: &Self) -> bool {
        self == other
    }

    fn join_same_type(&self, other: &Self) -> Option<Self> {
        (self == other).then(|| self.clone())
    }

    fn enables_long_lived_circuits(&self) -> bool {
        match &self.0 {
            SocksAuth::Socks4(auth) => !auth.is_empty(),
            SocksAuth::Username(username, password) => {
                !(username.is_empty() && password.is_empty())
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};
    use tokio_util::compat::{Compat, TokioAsyncReadCompatExt as _};

    use super::*;

    const CONNECT_EXAMPLE_443: &[u8] = &[
        5, 1, 0, 3, 11, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b'c', b'o', b'm', 0x01,
        0xBB,
    ];

    /// Runs `negotiate` against an in-memory peer that scripts the client side
    ///
    /// The script returns the bytes it read, so a test that ends without the proxy
    /// closing the connection never waits on an end of file that will not arrive
    async fn negotiate_with<F>(peer_script: F) -> (Result<SocksRequest, ConnectionError>, Vec<u8>)
    where
        F: AsyncFnOnce(&mut DuplexStream) -> Vec<u8>,
    {
        let (proxy_side, mut peer_side) = duplex(1024);
        let mut proxy_side: Compat<DuplexStream> = proxy_side.compat();

        tokio::join!(negotiate(&mut proxy_side), peer_script(&mut peer_side))
    }

    /// Reads the whole refusal the proxy writes before closing the connection
    async fn read_refusal(peer: &mut DuplexStream) -> Vec<u8> {
        let mut refusal = Vec::new();
        peer.read_to_end(&mut refusal).await.unwrap();

        refusal
    }

    #[tokio::test]
    async fn negotiates_an_unauthenticated_connect_request() {
        let (result, replies) = negotiate_with(async |peer| {
            peer.write_all(&[5, 1, 0]).await.unwrap();

            let mut method = [0_u8; 2];
            peer.read_exact(&mut method).await.unwrap();

            peer.write_all(CONNECT_EXAMPLE_443).await.unwrap();

            method.to_vec()
        })
        .await;

        let request = result.unwrap();

        assert_eq!(replies, [5, 0]);
        assert_eq!(request.version(), SocksVersion::V5);
        assert_eq!(request.command(), SocksCmd::CONNECT);
        assert_eq!(request.addr().to_string(), "example.com");
        assert_eq!(request.port(), 443);
        assert_eq!(request.auth(), &SocksAuth::NoAuth);
    }

    #[tokio::test]
    async fn surfaces_username_and_password_credentials() {
        let (result, replies) = negotiate_with(async |peer| {
            peer.write_all(&[5, 1, 2]).await.unwrap();

            let mut method = [0_u8; 2];
            peer.read_exact(&mut method).await.unwrap();

            peer.write_all(&[1, 4, b'n', b'o', b'd', b'e', 4, b'c', b'o', b'v', b'e'])
                .await
                .unwrap();

            let mut authenticated = [0_u8; 2];
            peer.read_exact(&mut authenticated).await.unwrap();

            peer.write_all(CONNECT_EXAMPLE_443).await.unwrap();

            [method, authenticated].concat()
        })
        .await;

        let request = result.unwrap();

        assert_eq!(replies, [5, 2, 1, 0]);
        assert_eq!(request.auth(), &SocksAuth::Username(b"node".to_vec(), b"cove".to_vec()));
    }

    #[tokio::test]
    async fn refuses_udp_associate_with_command_not_supported() {
        let (result, replies) = negotiate_with(async |peer| {
            peer.write_all(&[5, 1, 0]).await.unwrap();

            let mut method = [0_u8; 2];
            peer.read_exact(&mut method).await.unwrap();

            let mut request = CONNECT_EXAMPLE_443.to_vec();
            request[1] = 3;
            peer.write_all(&request).await.unwrap();

            read_refusal(peer).await
        })
        .await;

        assert!(matches!(
            result,
            Err(ConnectionError::Handshake(tor_socksproto::Error::NotImplemented(_)))
        ));
        assert_eq!(replies, SOCKS5_COMMAND_NOT_SUPPORTED);
    }

    #[tokio::test]
    async fn refuses_clients_offering_no_supported_authentication_method() {
        let (result, replies) = negotiate_with(async |peer| {
            // GSSAPI only
            peer.write_all(&[5, 1, 1]).await.unwrap();

            read_refusal(peer).await
        })
        .await;

        assert!(matches!(
            result,
            Err(ConnectionError::Handshake(tor_socksproto::Error::NotImplemented(_)))
        ));
        assert_eq!(replies, SOCKS5_NO_ACCEPTABLE_METHODS);
    }

    #[tokio::test]
    async fn refuses_a_socks4_bind_request() {
        let (result, replies) = negotiate_with(async |peer| {
            // SOCKS4 BIND for 203.0.113.7:80 with an empty user field
            peer.write_all(&[4, 2, 0x00, 0x50, 203, 0, 113, 7, 0]).await.unwrap();

            read_refusal(peer).await
        })
        .await;

        assert!(matches!(
            result,
            Err(ConnectionError::Handshake(tor_socksproto::Error::NotImplemented(_)))
        ));
        assert_eq!(replies, SOCKS4_REJECTED);
    }

    #[tokio::test]
    async fn reports_an_end_of_file_in_the_middle_of_a_handshake() {
        let (result, replies) = negotiate_with(async |peer| {
            peer.write_all(&[5, 1, 0]).await.unwrap();

            let mut method = [0_u8; 2];
            peer.read_exact(&mut method).await.unwrap();

            peer.shutdown().await.unwrap();

            method.to_vec()
        })
        .await;

        assert!(matches!(
            result,
            Err(ConnectionError::Handshake(tor_socksproto::Error::UnexpectedEof))
        ));
        assert_eq!(replies, [5, 0]);
    }

    #[tokio::test]
    async fn rejects_payload_data_pipelined_before_the_reply() {
        let (result, _) = negotiate_with(async |peer| {
            peer.write_all(&[5, 1, 0]).await.unwrap();

            let mut method = [0_u8; 2];
            peer.read_exact(&mut method).await.unwrap();

            let mut request = CONNECT_EXAMPLE_443.to_vec();
            request.extend_from_slice(b"GET / HTTP/1.1\r\n");
            peer.write_all(&request).await.unwrap();

            method.to_vec()
        })
        .await;

        assert!(matches!(
            result,
            Err(ConnectionError::Handshake(tor_socksproto::Error::ForbiddenPipelining))
        ));
    }

    fn request(version: SocksVersion, command: SocksCmd) -> SocksRequest {
        SocksRequest::new(
            version,
            command,
            tor_socksproto::SocksAddr::Hostname("example.com".to_string().try_into().unwrap()),
            443,
            SocksAuth::NoAuth,
        )
        .unwrap()
    }

    #[test]
    fn only_socks5_connect_requests_are_served() {
        assert_eq!(validate_request(&request(SocksVersion::V5, SocksCmd::CONNECT)), Ok(()));

        assert_eq!(
            validate_request(&request(SocksVersion::V4, SocksCmd::CONNECT)),
            Err(SocksStatus::GENERAL_FAILURE)
        );

        for command in [SocksCmd::RESOLVE, SocksCmd::RESOLVE_PTR] {
            assert_eq!(
                validate_request(&request(SocksVersion::V5, command)),
                Err(SocksStatus::COMMAND_NOT_SUPPORTED),
                "command: {command}"
            );
        }
    }

    #[test]
    fn socks4_refusals_encode_as_a_socks4_reply() {
        let reply = request(SocksVersion::V4, SocksCmd::CONNECT)
            .reply(SocksStatus::GENERAL_FAILURE, None)
            .unwrap();

        assert_eq!(reply, [0, 0x5B, 0, 0, 0, 0, 0, 0]);
    }

    fn username(username: &str) -> SocksIsolationKey {
        SocksIsolationKey(SocksAuth::Username(username.as_bytes().to_vec(), b"cove".to_vec()))
    }

    #[test]
    fn streams_share_circuits_only_with_an_identical_label() {
        assert!(username("node").compatible_same_type(&username("node")));
        assert!(!username("node").compatible_same_type(&username("payjoin")));

        let no_auth = SocksIsolationKey(SocksAuth::NoAuth);
        assert!(no_auth.compatible_same_type(&SocksIsolationKey(SocksAuth::NoAuth)));
        assert!(!no_auth.compatible_same_type(&username("node")));

        // an empty-credential login is still not the same bucket as no login at all
        let empty = SocksIsolationKey(SocksAuth::Username(Vec::new(), Vec::new()));
        assert!(!empty.compatible_same_type(&no_auth));
    }

    #[test]
    fn joining_matching_labels_preserves_them() {
        assert_eq!(username("node").join_same_type(&username("node")), Some(username("node")));
        assert_eq!(username("node").join_same_type(&username("http")), None);
    }

    #[test]
    fn only_non_empty_credentials_enable_long_lived_circuits() {
        assert!(username("node").enables_long_lived_circuits());
        assert!(!SocksIsolationKey(SocksAuth::NoAuth).enables_long_lived_circuits());
        assert!(
            !SocksIsolationKey(SocksAuth::Username(Vec::new(), Vec::new()))
                .enables_long_lived_circuits()
        );
        assert!(
            SocksIsolationKey(SocksAuth::Socks4(b"label".to_vec())).enables_long_lived_circuits()
        );
    }

    #[test]
    fn descriptor_exhaustion_backs_off_instead_of_stopping_the_accept_loop() {
        #[cfg(unix)]
        for code in [libc::EMFILE, libc::ENFILE] {
            assert_eq!(
                accept_failure(&io::Error::from_raw_os_error(code)),
                AcceptFailure::DescriptorsExhausted,
                "os error: {code}"
            );
        }

        assert_eq!(accept_failure(&io::Error::other("listener is gone")), AcceptFailure::Fatal);
    }

    #[test]
    fn onion_failures_map_to_their_extended_socks_status() {
        assert_eq!(
            status_for_error(ErrorKind::OnionServiceNotFound),
            SocksStatus::HS_DESC_NOT_FOUND
        );
        assert_eq!(
            status_for_error(ErrorKind::OnionServiceConnectionFailed),
            SocksStatus::HS_INTRO_FAILED
        );
        assert_eq!(status_for_error(ErrorKind::RemoteNetworkFailed), SocksStatus::TTL_EXPIRED);
        assert_eq!(status_for_error(ErrorKind::RemoteHostNotFound), SocksStatus::GENERAL_FAILURE);
    }

    #[test]
    fn address_families_follow_the_requested_target() {
        // StreamPrefs has no accessors, so compare against prefs built the same way
        fn expected(apply: impl FnOnce(&mut StreamPrefs)) -> String {
            let mut prefs = StreamPrefs::new();
            apply(&mut prefs);

            format!("{prefs:?}")
        }

        for (addr, apply) in [
            ("127.0.0.1", &StreamPrefs::ipv4_only as &dyn Fn(&mut StreamPrefs) -> &mut StreamPrefs),
            ("::1", &StreamPrefs::ipv6_only),
            ("example.com", &StreamPrefs::ipv4_preferred),
            ("example.onion", &StreamPrefs::ipv4_preferred),
        ] {
            assert_eq!(
                format!("{:?}", stream_prefs(addr)),
                expected(|prefs| {
                    apply(prefs);
                }),
                "addr: {addr}"
            );
        }
    }
}

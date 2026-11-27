//! A SOCKS5 listener on loopback that hands each stream to a [`Connector`]
//!
//! The app's clients already speak SOCKS5 to reach an external proxy, so the built-in Tor client
//! sits behind the same interface, and both modes share one client code path. Arti's own
//! `tor-socksproto` parses the handshake

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    time::Duration,
};

use cove_http::{ProxyAddress, SocksCredentials, SocksProxy};
use subtle::ConstantTimeEq as _;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use tor_socksproto::{
    Buffer, Handshake as _, NextStep, SocksAddr, SocksAuth, SocksCmd, SocksProxyHandshake,
    SocksRequest, SocksStatus,
};
use tracing::{debug, warn};

/// How long the bridge waits for a stream to open, covering a first start that is still joining
/// the Tor network; clients give up sooner on their own timeouts
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a client has to finish the SOCKS handshake, so a local app that connects and sends
/// nothing can't hold the connection open
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens streams for the bridge
pub trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn connect(
        &self,
        target: SocksAddr,
        port: u16,
    ) -> impl Future<Output = Result<Self::Stream, SocksStatus>> + Send;
}

#[derive(Debug, thiserror::Error)]
enum HandshakeError {
    #[error(transparent)]
    Socks(#[from] tor_socksproto::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A bound loopback listener that isn't serving yet
///
/// Binding is synchronous, so the proxy address is known before any async work starts and the
/// route can point at it right away. Streams that arrive before [`Self::serve`] wait in the
/// listen backlog
#[derive(Debug)]
pub struct BoundBridge {
    listener: StdTcpListener,
    proxy: SocksProxy,
}

impl BoundBridge {
    /// Binds a random loopback port
    ///
    /// With `credentials`, only clients that send them get a stream. Other apps on the device
    /// can reach a loopback port, so a bridge into Tor always sets them
    pub fn bind(credentials: Option<SocksCredentials>) -> std::io::Result<Self> {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;

        let port = listener.local_addr()?.port();
        let address = ProxyAddress::new_ip(Ipv4Addr::LOCALHOST.into(), port);

        Ok(Self { listener, proxy: SocksProxy { address, credentials } })
    }

    pub const fn proxy(&self) -> &SocksProxy {
        &self.proxy
    }

    /// Serves streams until `cancel` fires, which also closes every open stream
    ///
    /// Must run inside a tokio runtime
    pub async fn serve<C: Connector>(self, connector: Arc<C>, cancel: CancellationToken) {
        let listener = match TcpListener::from_std(self.listener) {
            Ok(listener) => listener,
            Err(error) => {
                warn!("Tor proxy could not start listening: {error}");
                return;
            }
        };

        let credentials = Arc::new(self.proxy.credentials);
        loop {
            let accepted = tokio::select! {
                () = cancel.cancelled() => return,
                accepted = listener.accept() => accepted,
            };

            let (stream, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    warn!("Tor proxy accept failed: {error}");
                    continue;
                }
            };

            let connector = connector.clone();
            let credentials = credentials.clone();
            let cancel = cancel.clone();

            tokio::spawn(async move {
                tokio::select! {
                    () = cancel.cancelled() => {}
                    () = handle_stream(stream, peer, credentials.as_ref().as_ref(), connector.as_ref()) => {}
                }
            });
        }
    }
}

async fn handle_stream<C: Connector>(
    mut stream: TcpStream,
    peer: SocketAddr,
    credentials: Option<&SocksCredentials>,
    connector: &C,
) {
    let request = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_request(&mut stream));
    let request = match request.await {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => {
            debug!("Tor proxy refused a client at {peer}: {error}");
            return;
        }

        Err(_) => {
            debug!("Tor proxy dropped a client at {peer} that never finished the handshake");
            return;
        }
    };

    if let Some(status) = refusal(&request, credentials) {
        debug!("Tor proxy refused a request from {peer}: {status}");
        let _ = send_reply(&mut stream, &request, status).await;
        return;
    }

    let connect = connector.connect(request.addr().clone(), request.port());
    let mut remote = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(Ok(remote)) => remote,
        Ok(Err(status)) => {
            let _ = send_reply(&mut stream, &request, status).await;
            return;
        }

        Err(_) => {
            let _ = send_reply(&mut stream, &request, SocksStatus::TTL_EXPIRED).await;
            return;
        }
    };

    if send_reply(&mut stream, &request, SocksStatus::SUCCEEDED).await.is_err() {
        return;
    }

    // either side closing ends the stream, and the error only says which side went first
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
}

/// Runs the handshake up to the client's request, which still needs a reply
async fn read_request(stream: &mut TcpStream) -> Result<SocksRequest, HandshakeError> {
    let mut handshake = SocksProxyHandshake::new();
    let mut buffer = Buffer::new();

    loop {
        match handshake.step(&mut buffer)? {
            NextStep::Send(data) => stream.write_all(&data).await?,
            NextStep::Recv(mut recv) => {
                let read = stream.read(recv.buf()).await?;
                recv.note_received(read)?;
            }

            // the app's clients wait for the reply before sending, so early data is a bad client
            NextStep::Finished(finished) => return Ok(finished.into_output_forbid_pipelining()?),
        }
    }
}

/// Why `request` gets no stream, if it doesn't
///
/// The handshake accepts any username and password, and SOCKS4 or no auth when the client offers
/// only those, so the credentials are checked here
fn refusal(request: &SocksRequest, credentials: Option<&SocksCredentials>) -> Option<SocksStatus> {
    if request.command() != SocksCmd::CONNECT {
        return Some(SocksStatus::COMMAND_NOT_SUPPORTED);
    }

    let expected = credentials?;
    let SocksAuth::Username(username, password) = request.auth() else {
        return Some(SocksStatus::NOT_ALLOWED);
    };

    // both comparisons always run, so the reply time doesn't say which one failed
    let matches =
        username.ct_eq(expected.username.as_bytes()) & password.ct_eq(expected.password.as_bytes());

    (!bool::from(matches)).then_some(SocksStatus::NOT_ALLOWED)
}

async fn send_reply(
    stream: &mut TcpStream,
    request: &SocksRequest,
    status: SocksStatus,
) -> std::io::Result<()> {
    let reply = request.reply(status, None).map_err(std::io::Error::other)?;
    stream.write_all(&reply).await?;
    stream.flush().await
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, reason = "the tests dial the bridge and an echo server")]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Dials targets directly, standing in for Tor
    #[derive(Default)]
    struct DirectConnector {
        connects: AtomicUsize,
    }

    impl Connector for DirectConnector {
        type Stream = TcpStream;

        async fn connect(&self, target: SocksAddr, port: u16) -> Result<TcpStream, SocksStatus> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            let SocksAddr::Hostname(host) = target else { return Err(SocksStatus::NOT_ALLOWED) };
            TcpStream::connect((host.as_ref(), port))
                .await
                .map_err(|_| SocksStatus::HOST_UNREACHABLE)
        }
    }

    fn credentials() -> SocksCredentials {
        SocksCredentials { username: "cove".to_string(), password: "secret".to_string() }
    }

    fn start_bridge() -> (SocksProxy, Arc<DirectConnector>, CancellationToken) {
        let bridge = BoundBridge::bind(Some(credentials())).unwrap();
        let proxy = bridge.proxy().clone();
        let connector = Arc::new(DirectConnector::default());
        let cancel = CancellationToken::new();
        tokio::spawn(bridge.serve(connector.clone(), cancel.clone()));

        (proxy, connector, cancel)
    }

    async fn echo_server() -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0; 64];
                    while let Ok(read) = stream.read(&mut buffer).await {
                        if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });

        port
    }

    /// A SOCKS5 CONNECT to `host`, with username and password auth when `credentials` is set
    async fn socks_connect(
        proxy: &SocksProxy,
        credentials: Option<&SocksCredentials>,
        host: &str,
        port: u16,
    ) -> std::io::Result<TcpStream> {
        let mut stream = TcpStream::connect(proxy.address.to_string()).await?;

        let method = if credentials.is_some() { 2 } else { 0 };
        stream.write_all(&[5, 1, method]).await?;
        let mut choice = [0; 2];
        stream.read_exact(&mut choice).await?;

        if let Some(credentials) = credentials {
            let mut auth = vec![1, credentials.username.len() as u8];
            auth.extend_from_slice(credentials.username.as_bytes());
            auth.push(credentials.password.len() as u8);
            auth.extend_from_slice(credentials.password.as_bytes());
            stream.write_all(&auth).await?;
            let mut auth_reply = [0; 2];
            stream.read_exact(&mut auth_reply).await?;
        }

        let mut request = vec![5, 1, 0, 3, host.len() as u8];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await?;

        let mut reply = [0; 10];
        stream.read_exact(&mut reply).await?;
        if reply[1] != 0 {
            return Err(std::io::Error::other(format!("SOCKS reply {}", reply[1])));
        }

        Ok(stream)
    }

    #[tokio::test]
    async fn bridges_streams_until_cancelled() {
        let echo_port = echo_server().await;
        let (proxy, connector, cancel) = start_bridge();

        let mut stream =
            socks_connect(&proxy, Some(&credentials()), "localhost", echo_port).await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut echoed = [0; 4];
        stream.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");
        assert_eq!(connector.connects.load(Ordering::SeqCst), 1);

        // cancelling closes the open stream and stops new ones
        cancel.cancel();
        let mut buffer = [0; 4];
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        assert_eq!(read, 0);
        assert!(socks_connect(&proxy, Some(&credentials()), "localhost", echo_port).await.is_err());
    }

    #[tokio::test]
    async fn refuses_clients_without_the_right_credentials() {
        let echo_port = echo_server().await;
        let (proxy, connector, cancel) = start_bridge();
        let wrong = SocksCredentials { password: "guess".to_string(), ..credentials() };

        for credentials in [None, Some(&wrong)] {
            let error =
                socks_connect(&proxy, credentials, "localhost", echo_port).await.unwrap_err();
            assert!(error.to_string().contains("SOCKS reply 2"), "{error}");
        }

        assert_eq!(connector.connects.load(Ordering::SeqCst), 0, "a refused client got a stream");
        cancel.cancel();
    }

    #[tokio::test]
    async fn reports_connector_failures_to_the_client() {
        let (proxy, _connector, cancel) = start_bridge();

        let error = socks_connect(&proxy, Some(&credentials()), "127.0.0.1", 9).await.unwrap_err();
        assert!(error.to_string().contains("SOCKS reply"), "{error}");
        cancel.cancel();
    }
}

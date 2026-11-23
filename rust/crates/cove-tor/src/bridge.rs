//! A SOCKS5 listener on loopback that hands each stream to a [`Connector`]
//!
//! The app's clients already speak SOCKS5 to reach an external proxy, so the built-in Tor client
//! sits behind the same interface. Both modes then share one client code path, and the leak tests
//! that cover one cover the other

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    time::Duration,
};

use cove_http::{ProxyAddress, SocksCredentials, SocksProxy};
use rand::RngExt as _;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::socks::{self, Reply, Target};

/// How long the bridge waits for a stream to open, covering a first start that is still joining
/// the Tor network; clients give up sooner on their own timeouts
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a client has to finish the SOCKS handshake, so a local app that connects and sends
/// nothing can't hold the connection open
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens streams for the bridge
pub trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn connect(&self, target: Target) -> impl Future<Output = Result<Self::Stream, Reply>> + Send;
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
    /// Binds a random loopback port and picks fresh credentials for it
    pub fn bind() -> std::io::Result<Self> {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;

        let port = listener.local_addr()?.port();
        let address = ProxyAddress::new_ip(Ipv4Addr::LOCALHOST.into(), port);
        let proxy = SocksProxy { address, credentials: Some(random_credentials()) };

        Ok(Self { listener, proxy })
    }

    pub const fn proxy(&self) -> &SocksProxy {
        &self.proxy
    }

    /// Serves streams until `cancel` fires, which also closes every open stream
    ///
    /// Must run inside a tokio runtime
    pub async fn serve<C: Connector>(self, connector: Arc<C>, cancel: CancellationToken) {
        let Self { listener, proxy } = self;
        let credentials = proxy.credentials.expect("a bound bridge always has credentials");

        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                warn!("Tor proxy could not start listening: {error}");
                return;
            }
        };

        let credentials = Arc::new(credentials);
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
                    () = handle_stream(stream, peer, &credentials, connector.as_ref()) => {}
                }
            });
        }
    }
}

async fn handle_stream<C: Connector>(
    mut stream: TcpStream,
    peer: SocketAddr,
    credentials: &SocksCredentials,
    connector: &C,
) {
    let handshake =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, socks::accept(&mut stream, credentials));
    let target = match handshake.await {
        Ok(Ok(target)) => target,
        Ok(Err(error)) => {
            debug!("Tor proxy refused a client at {peer}: {error}");
            return;
        }

        Err(_) => {
            debug!("Tor proxy dropped a client at {peer} that never finished the handshake");
            return;
        }
    };

    let connected = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(target)).await;
    let mut remote = match connected {
        Ok(Ok(remote)) => remote,
        Ok(Err(reply)) => {
            let _ = socks::reply(&mut stream, reply).await;
            return;
        }

        Err(_) => {
            let _ = socks::reply(&mut stream, Reply::TimedOut).await;
            return;
        }
    };

    if socks::reply(&mut stream, Reply::Succeeded).await.is_err() {
        return;
    }

    // either side closing ends the stream, and the error only says which side went first
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
}

fn random_credentials() -> SocksCredentials {
    let mut password = [0u8; 24];
    rand::rng().fill(&mut password);

    SocksCredentials { username: "cove".to_string(), password: hex::encode(password) }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, reason = "the tests dial the bridge and an echo server")]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::socks::TargetHost;

    /// Dials targets directly, standing in for Tor
    #[derive(Default)]
    struct DirectConnector {
        connects: AtomicUsize,
    }

    impl Connector for DirectConnector {
        type Stream = TcpStream;

        async fn connect(&self, target: Target) -> Result<TcpStream, Reply> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            let TargetHost::Name(host) = target.host else { return Err(Reply::NotAllowed) };
            TcpStream::connect((host.as_str(), target.port))
                .await
                .map_err(|_| Reply::HostUnreachable)
        }
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

    async fn socks_connect(
        proxy: &SocksProxy,
        host: &str,
        port: u16,
    ) -> std::io::Result<TcpStream> {
        let mut stream = TcpStream::connect(proxy.address.to_string()).await?;
        let credentials = proxy.credentials.as_ref().unwrap();

        stream.write_all(&[5, 1, 2]).await?;
        let mut choice = [0; 2];
        stream.read_exact(&mut choice).await?;

        let mut auth = vec![1, credentials.username.len() as u8];
        auth.extend_from_slice(credentials.username.as_bytes());
        auth.push(credentials.password.len() as u8);
        auth.extend_from_slice(credentials.password.as_bytes());
        stream.write_all(&auth).await?;
        let mut auth_reply = [0; 2];
        stream.read_exact(&mut auth_reply).await?;

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
        let bridge = BoundBridge::bind().unwrap();
        let proxy = bridge.proxy().clone();
        let connector = Arc::new(DirectConnector::default());
        let cancel = CancellationToken::new();
        tokio::spawn(bridge.serve(connector.clone(), cancel.clone()));

        let mut stream = socks_connect(&proxy, "localhost", echo_port).await.unwrap();
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
        assert!(socks_connect(&proxy, "localhost", echo_port).await.is_err());
    }

    #[tokio::test]
    async fn reports_connector_failures_to_the_client() {
        let bridge = BoundBridge::bind().unwrap();
        let proxy = bridge.proxy().clone();
        let cancel = CancellationToken::new();
        tokio::spawn(bridge.serve(Arc::new(DirectConnector::default()), cancel.clone()));

        let error = socks_connect(&proxy, "127.0.0.1", 9).await.unwrap_err();
        assert!(error.to_string().contains("SOCKS reply"), "{error}");
        cancel.cancel();
    }
}

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use cove_tokio::FutureTimeoutExt as _;
use futures::{StreamExt as _, stream};
use tracing::debug;

use super::lan_binding::LanBinding;

/// How long a probe waits for a host to accept, long enough for a sleeping Wi-Fi device to wake
/// and short enough that hosts that don't exist don't hold up the sweep
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);

/// How many probes run at once, kept well under iOS's default limit of 256 open files
pub const MAX_CONCURRENT_PROBES: usize = 64;

/// Finds which of the addresses accept a TCP connection, returning them in no particular order
///
/// A probe only completes the TCP handshake and closes the connection: it never writes, so a
/// server learns nothing about the wallet until the person picks it and the app checks it.
/// Every probe is bound to `binding`'s network, and one that can't be bound isn't sent
pub async fn open_ports(
    binding: LanBinding,
    targets: impl IntoIterator<Item = SocketAddr>,
    timeout: Duration,
) -> Vec<SocketAddr> {
    stream::iter(targets)
        .map(|target| async move { accepts(binding, target, timeout).await.then_some(target) })
        .buffer_unordered(MAX_CONCURRENT_PROBES)
        .filter_map(std::future::ready)
        .collect()
        .await
}

async fn accepts(binding: LanBinding, target: SocketAddr, timeout: Duration) -> bool {
    let socket = match binding.tcp_socket() {
        Ok(socket) => socket,
        Err(error) => {
            debug!("Skipping probe of {target}, its socket can't be bound: {error}");
            return false;
        }
    };

    // the probe only completes the handshake, so a stream that connects is dropped at once
    matches!(socket.connect(target).with_timeout(timeout).await, Ok(Ok(_)))
}

/// Every pairing of host and port, the targets for one sweep
pub fn targets(hosts: &[IpAddr], ports: &[u16]) -> Vec<SocketAddr> {
    hosts.iter().flat_map(|host| ports.iter().map(|port| SocketAddr::new(*host, *port))).collect()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener};

    use super::*;
    use crate::node_discovery::lan_binding::tests::loopback;

    #[tokio::test]
    async fn only_ports_that_accept_are_reported() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let open = listener.local_addr().unwrap();

        // binding then dropping a listener leaves a port nothing listens on
        let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap();

        let found = open_ports(loopback(), [open, closed], CONNECT_TIMEOUT).await;

        assert_eq!(found, vec![open]);
    }

    // a probe that can't be bound to the network is never sent over the default route instead
    #[tokio::test]
    async fn a_probe_that_cannot_be_bound_is_not_sent() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let open = listener.local_addr().unwrap();
        let missing = LanBinding::InterfaceIndex { index: u32::MAX };

        assert_eq!(open_ports(missing, [open], CONNECT_TIMEOUT).await, Vec::new());
    }

    // a probe must not send anything, so the server reads end of stream right after accepting
    #[tokio::test]
    async fn a_probe_sends_nothing_to_the_server() {
        use std::io::Read as _;

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

            let mut received = Vec::new();
            stream.read_to_end(&mut received).unwrap();
            received
        });

        assert_eq!(open_ports(loopback(), [address], CONNECT_TIMEOUT).await, vec![address]);
        assert_eq!(server.join().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn targets_pair_every_host_with_every_port() {
        let hosts = [IpAddr::from([192, 168, 1, 1]), IpAddr::from([192, 168, 1, 2])];
        let targets = targets(&hosts, &[50001, 50002]);

        assert_eq!(targets.len(), 4);
        assert!(targets.contains(&"192.168.1.2:50002".parse().unwrap()));
    }
}

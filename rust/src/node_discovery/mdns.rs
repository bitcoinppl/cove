//! One-shot multicast DNS lookups for `.local` hostnames and the names of local addresses
//!
//! Queries go to the link-local mDNS group from an ordinary port, so responders answer by
//! unicast to that port (RFC 6762 section 5.1) and never leave the local network. iOS only lets
//! apps with Apple's multicast entitlement send multicast, so it looks hostnames up through the
//! system resolver, which answers `.local` names over mDNS itself, and skips reverse lookups.
//! The system resolver's queries can't be bound to the searched network, but its mDNS only uses
//! local interfaces, and the addresses it returns are only probed over that network

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use cove_tokio::FutureTimeoutExt as _;
use simple_dns::{CLASS, Name, Packet, QCLASS, QTYPE, Question, TYPE, rdata::RData};
use tokio::{net::UdpSocket, time::Instant};
use tracing::{debug, warn};

use super::{
    lan_binding::LanBinding,
    lan_subnet::{LanSubnet, is_lan_address, is_lan_ipv4},
};

/// The IPv4 mDNS group and port
const MDNS_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);

/// How long lookups wait for answers, most responders reply within a few milliseconds
pub const MDNS_WAIT: Duration = Duration::from_millis(1500);

/// Whether this platform resolves hostnames through the system resolver instead of sending its
/// own multicast queries
const USES_SYSTEM_RESOLVER: bool = cfg!(target_os = "ios");

/// What mDNS answered: the local addresses of each hostname and the hostname of each address
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MdnsAnswers {
    pub addresses: BTreeMap<String, BTreeSet<Ipv4Addr>>,
    pub names: BTreeMap<Ipv4Addr, String>,
}

/// Sends mDNS queries from one local interface
#[derive(Debug, Clone, Copy)]
pub struct MdnsClient {
    source: Ipv4Addr,
    binding: LanBinding,
    destination: SocketAddr,
    wait: Duration,
}

impl MdnsClient {
    /// A client that queries the mDNS group from the device's address on the subnet, over the
    /// subnet's own network
    pub const fn on_subnet(subnet: &LanSubnet) -> Self {
        Self {
            source: subnet.own_address(),
            binding: subnet.binding(),
            destination: SocketAddr::V4(MDNS_GROUP),
            wait: MDNS_WAIT,
        }
    }

    /// Looks up the addresses of `.local` hostnames and the hostnames of local addresses
    ///
    /// Failures are logged and give no answers, since discovery still finds nodes by probing
    pub async fn lookup(&self, hostnames: &[String], addresses: &[Ipv4Addr]) -> MdnsAnswers {
        if hostnames.is_empty() && addresses.is_empty() {
            return MdnsAnswers::default();
        }

        if USES_SYSTEM_RESOLVER {
            return system_lookup(hostnames, self.wait).await;
        }

        match self.multicast_lookup(hostnames, addresses).await {
            Ok(answers) => answers,
            Err(error) => {
                warn!("mDNS lookup failed: {error}");
                MdnsAnswers::default()
            }
        }
    }

    async fn multicast_lookup(
        &self,
        hostnames: &[String],
        addresses: &[Ipv4Addr],
    ) -> std::io::Result<MdnsAnswers> {
        let socket = self.bind()?;
        let id = rand::random();
        let queries = build_queries(id, hostnames, addresses);

        // one resend halfway through covers a lost packet on a busy Wi-Fi network
        let deadline = Instant::now() + self.wait;
        let resend_at = Instant::now() + self.wait / 2;
        let mut resent = false;
        self.send_all(&socket, &queries).await?;

        let mut answers = MdnsAnswers::default();
        let mut buffer = vec![0_u8; 9000];

        loop {
            let wake_at = if resent { deadline } else { resend_at };

            match socket.recv_from(&mut buffer).with_deadline(wake_at).await {
                Ok(Ok((length, from))) => {
                    read_reply(&buffer[..length], from, id, hostnames, addresses, &mut answers)
                }
                Ok(Err(error)) => debug!("mDNS receive failed: {error}"),
                Err(_) if resent => break,
                Err(_) => {
                    resent = true;
                    self.send_all(&socket, &queries).await?;
                }
            }
        }

        Ok(answers)
    }

    fn bind(&self) -> std::io::Result<UdpSocket> {
        let socket = self.binding.udp_socket(SocketAddrV4::new(self.source, 0))?;

        // multicast leaves through the Wi-Fi or Ethernet interface, never a VPN or cellular
        // route, and RFC 6762 asks for a hop limit of 255 so receivers can reject routed packets
        socket.set_multicast_if_v4(&self.source)?;
        socket.set_multicast_ttl_v4(255)?;

        UdpSocket::from_std(socket.into())
    }

    async fn send_all(&self, socket: &UdpSocket, queries: &[Vec<u8>]) -> std::io::Result<()> {
        for query in queries {
            socket.send_to(query, self.destination).await?;
        }

        Ok(())
    }
}

/// One query packet per name, since some responders only answer the first question
fn build_queries(id: u16, hostnames: &[String], addresses: &[Ipv4Addr]) -> Vec<Vec<u8>> {
    let forward = hostnames.iter().map(|hostname| (hostname.clone(), TYPE::A));
    let reverse = addresses.iter().map(|address| (reverse_name(*address), TYPE::PTR));

    forward
        .chain(reverse)
        .filter_map(|(name, record_type)| {
            let mut packet = Packet::new_query(id);
            let name = Name::new(&name).ok()?;
            let question =
                Question::new(name, QTYPE::TYPE(record_type), QCLASS::CLASS(CLASS::IN), false);

            packet.questions.push(question);
            packet.build_bytes_vec().ok()
        })
        .collect()
}

/// Keeps the answers a reply gives to this lookup's own questions, from a local sender
fn read_reply(
    reply: &[u8],
    from: SocketAddr,
    id: u16,
    hostnames: &[String],
    addresses: &[Ipv4Addr],
    answers: &mut MdnsAnswers,
) {
    // a loopback sender is this device itself, never another host on the network
    if !is_lan_address(from.ip()) && !from.ip().is_loopback() {
        return;
    }

    let Ok(packet) = Packet::parse(reply) else { return };
    if packet.id() != id {
        return;
    }

    for record in packet.answers.iter().chain(&packet.additional_records) {
        let name = record.name.to_string();

        match &record.rdata {
            RData::A(a) => {
                let address = Ipv4Addr::from(a.address);
                let Some(hostname) = hostnames.iter().find(|h| h.eq_ignore_ascii_case(&name))
                else {
                    continue;
                };

                if is_lan_ipv4(address) {
                    answers.addresses.entry(hostname.clone()).or_default().insert(address);
                }
            }

            RData::PTR(ptr) => {
                let Some(address) =
                    addresses.iter().find(|a| reverse_name(**a).eq_ignore_ascii_case(&name))
                else {
                    continue;
                };

                let hostname = ptr.0.to_string();
                if is_local_hostname(&hostname) {
                    answers.names.insert(*address, hostname);
                }
            }

            _ => {}
        }
    }
}

/// Resolves hostnames through the system resolver, which answers `.local` names over mDNS
async fn system_lookup(hostnames: &[String], wait: Duration) -> MdnsAnswers {
    let lookups = hostnames.iter().map(|hostname| async move {
        let found = tokio::net::lookup_host((hostname.as_str(), 0)).with_timeout(wait).await;
        let addresses: BTreeSet<Ipv4Addr> = match found {
            Ok(Ok(addresses)) => addresses
                .filter_map(|address| match address.ip() {
                    IpAddr::V4(address) if is_lan_ipv4(address) => Some(address),
                    _ => None,
                })
                .collect(),
            _ => BTreeSet::new(),
        };

        (hostname.clone(), addresses)
    });

    let addresses = futures::future::join_all(lookups)
        .await
        .into_iter()
        .filter(|(_, addresses)| !addresses.is_empty())
        .collect();

    MdnsAnswers { addresses, names: BTreeMap::new() }
}

/// The `in-addr.arpa` name a reverse lookup asks for
fn reverse_name(address: Ipv4Addr) -> String {
    let [a, b, c, d] = address.octets();
    format!("{d}.{c}.{b}.{a}.in-addr.arpa")
}

/// Only `.local` names are kept, so a reply can't label a node with an arbitrary domain
fn is_local_hostname(hostname: &str) -> bool {
    hostname.len() > ".local".len()
        && hostname.to_ascii_lowercase().ends_with(".local")
        && hostname.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket as StdUdpSocket;

    use simple_dns::{ResourceRecord, rdata};

    use super::*;
    use crate::node_discovery::lan_binding::tests::loopback;

    /// A responder on loopback that answers each query with the given records
    fn responder(answer: impl Fn(&Question) -> Vec<RData<'static>> + Send + 'static) -> SocketAddr {
        let socket = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = socket.local_addr().unwrap();

        std::thread::spawn(move || {
            let mut buffer = [0_u8; 1500];
            while let Ok((length, from)) = socket.recv_from(&mut buffer) {
                let query = Packet::parse(&buffer[..length]).unwrap();
                let mut reply = Packet::new_reply(query.id());

                for question in &query.questions {
                    for rdata in answer(question) {
                        let name = question.qname.clone().into_owned();
                        reply.answers.push(ResourceRecord::new(name, CLASS::IN, 120, rdata));
                    }
                }

                let _ = socket.send_to(&reply.build_bytes_vec().unwrap(), from);
            }
        });

        address
    }

    fn client(destination: SocketAddr) -> MdnsClient {
        MdnsClient {
            source: Ipv4Addr::LOCALHOST,
            binding: loopback(),
            destination,
            wait: Duration::from_millis(300),
        }
    }

    // a query that can't be bound to the network is never sent over the default route instead
    #[tokio::test]
    async fn a_client_that_cannot_be_bound_sends_no_query() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        let destination = responder(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        });

        let unbindable = MdnsClient {
            binding: LanBinding::InterfaceIndex { index: u32::MAX },
            ..client(destination)
        };

        let answers = unbindable.lookup(&["umbrel.local".to_string()], &[]).await;

        // a query sent over loopback would reach the responder well within this
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(answers, MdnsAnswers::default());
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn hostnames_and_reverse_names_resolve_to_local_answers_only() {
        let destination = responder(|question| match question.qname.to_string().as_str() {
            "umbrel.local" => vec![
                RData::A(rdata::A { address: Ipv4Addr::new(192, 168, 1, 20).into() }),
                // a public address in a reply must never become a probe target
                RData::A(rdata::A { address: Ipv4Addr::new(8, 8, 8, 8).into() }),
            ],
            "10.1.168.192.in-addr.arpa" => {
                vec![RData::PTR(rdata::PTR(Name::new_unchecked("bitcoin.local")))]
            }
            "11.1.168.192.in-addr.arpa" => {
                vec![RData::PTR(rdata::PTR(Name::new_unchecked("evil.example.com")))]
            }
            _ => vec![],
        });

        let answers = client(destination)
            .lookup(
                &["umbrel.local".to_string(), "start9.local".to_string()],
                &[Ipv4Addr::new(192, 168, 1, 10), Ipv4Addr::new(192, 168, 1, 11)],
            )
            .await;

        let expected = MdnsAnswers {
            addresses: BTreeMap::from([(
                "umbrel.local".to_string(),
                BTreeSet::from([Ipv4Addr::new(192, 168, 1, 20)]),
            )]),
            names: BTreeMap::from([(Ipv4Addr::new(192, 168, 1, 10), "bitcoin.local".to_string())]),
        };

        assert_eq!(answers, expected);
    }

    #[tokio::test]
    async fn a_reply_to_another_query_is_ignored() {
        let socket = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let destination = socket.local_addr().unwrap();

        std::thread::spawn(move || {
            let mut buffer = [0_u8; 1500];
            let (_, from) = socket.recv_from(&mut buffer).unwrap();
            let query = Packet::parse(&buffer).unwrap();

            let mut reply = Packet::new_reply(query.id().wrapping_add(1));
            reply.answers.push(ResourceRecord::new(
                Name::new_unchecked("umbrel.local"),
                CLASS::IN,
                120,
                RData::A(rdata::A { address: Ipv4Addr::new(192, 168, 1, 20).into() }),
            ));

            socket.send_to(&reply.build_bytes_vec().unwrap(), from).unwrap();
        });

        let answers = client(destination).lookup(&["umbrel.local".to_string()], &[]).await;

        assert_eq!(answers, MdnsAnswers::default());
    }

    #[test]
    fn reverse_names_put_the_octets_in_reverse() {
        assert_eq!(reverse_name(Ipv4Addr::new(192, 168, 1, 10)), "10.1.168.192.in-addr.arpa");
    }
}

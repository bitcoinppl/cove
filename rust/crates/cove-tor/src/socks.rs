//! The server side of SOCKS5 (RFC 1928) with username and password auth (RFC 1929), limited to
//! the CONNECT command the app's clients use

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use cove_http::SocksCredentials;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

const VERSION: u8 = 0x05;
const AUTH_VERSION: u8 = 0x01;
const METHOD_USERNAME_PASSWORD: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;
const COMMAND_CONNECT: u8 = 0x01;
const ADDRESS_IPV4: u8 = 0x01;
const ADDRESS_DOMAIN: u8 = 0x03;
const ADDRESS_IPV6: u8 = 0x04;

/// Where a client asked to connect
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: TargetHost,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetHost {
    Ip(IpAddr),
    /// A name Tor resolves at the exit, or an onion address
    Name(String),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let port = self.port;
        match &self.host {
            TargetHost::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]:{port}"),
            TargetHost::Ip(IpAddr::V4(ip)) => write!(f, "{ip}:{port}"),
            TargetHost::Name(name) => write!(f, "{name}:{port}"),
        }
    }
}

/// The reply codes the bridge sends back, from RFC 1928 section 6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Succeeded = 0x00,
    GeneralFailure = 0x01,
    NotAllowed = 0x02,
    HostUnreachable = 0x04,
    ConnectionRefused = 0x05,
    TimedOut = 0x06,
    CommandNotSupported = 0x07,
    AddressTypeNotSupported = 0x08,
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("client spoke SOCKS version {0}")]
    UnsupportedVersion(u8),

    #[error("client offered no username and password auth")]
    NoAcceptableMethod,

    #[error("client sent the wrong credentials")]
    BadCredentials,

    #[error("client asked for SOCKS command {0}")]
    UnsupportedCommand(u8),

    #[error("client sent address type {0}")]
    UnsupportedAddressType(u8),

    #[error("client sent a host name that isn't UTF-8")]
    InvalidHostName,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Runs the handshake up to the CONNECT request and returns its target
///
/// On a protocol error the client has already been told why, so the caller only closes the
/// stream. On success the caller must answer with [`reply`]
pub async fn accept<S>(
    stream: &mut S,
    credentials: &SocksCredentials,
) -> Result<Target, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    negotiate_method(stream).await?;
    authenticate(stream, credentials).await?;
    read_connect_request(stream).await
}

/// Sends the answer to a CONNECT request
pub async fn reply<S>(stream: &mut S, reply: Reply) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    // the bound address is meaningless for a proxied stream, so it is always 0.0.0.0:0
    let message = [VERSION, reply as u8, 0x00, ADDRESS_IPV4, 0, 0, 0, 0, 0, 0];
    stream.write_all(&message).await?;
    stream.flush().await
}

async fn negotiate_method<S>(stream: &mut S) -> Result<(), HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let [version, method_count] = read_array(stream).await?;
    if version != VERSION {
        return Err(HandshakeError::UnsupportedVersion(version));
    }

    let mut methods = vec![0; method_count.into()];
    stream.read_exact(&mut methods).await?;

    if !methods.contains(&METHOD_USERNAME_PASSWORD) {
        stream.write_all(&[VERSION, METHOD_NONE_ACCEPTABLE]).await?;
        return Err(HandshakeError::NoAcceptableMethod);
    }

    stream.write_all(&[VERSION, METHOD_USERNAME_PASSWORD]).await?;
    Ok(())
}

async fn authenticate<S>(
    stream: &mut S,
    credentials: &SocksCredentials,
) -> Result<(), HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let [version] = read_array(stream).await?;
    if version != AUTH_VERSION {
        return Err(HandshakeError::UnsupportedVersion(version));
    }

    let username = read_length_prefixed(stream).await?;
    let password = read_length_prefixed(stream).await?;

    // both checks always run, so the reply time doesn't say which one failed
    let username_matches = constant_time_eq(&username, credentials.username.as_bytes());
    let password_matches = constant_time_eq(&password, credentials.password.as_bytes());

    if !(username_matches & password_matches) {
        stream.write_all(&[AUTH_VERSION, 0x01]).await?;
        return Err(HandshakeError::BadCredentials);
    }

    stream.write_all(&[AUTH_VERSION, 0x00]).await?;
    Ok(())
}

async fn read_connect_request<S>(stream: &mut S) -> Result<Target, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let [version, command, _reserved, address_type] = read_array(stream).await?;
    if version != VERSION {
        return Err(HandshakeError::UnsupportedVersion(version));
    }

    if command != COMMAND_CONNECT {
        reply(stream, Reply::CommandNotSupported).await?;
        return Err(HandshakeError::UnsupportedCommand(command));
    }

    let host = match address_type {
        ADDRESS_IPV4 => TargetHost::Ip(Ipv4Addr::from(read_array::<_, 4>(stream).await?).into()),
        ADDRESS_IPV6 => TargetHost::Ip(Ipv6Addr::from(read_array::<_, 16>(stream).await?).into()),
        ADDRESS_DOMAIN => {
            let name = read_length_prefixed(stream).await?;
            let name = String::from_utf8(name).map_err(|_| HandshakeError::InvalidHostName)?;
            TargetHost::Name(name)
        }

        other => {
            reply(stream, Reply::AddressTypeNotSupported).await?;
            return Err(HandshakeError::UnsupportedAddressType(other));
        }
    };

    let port = u16::from_be_bytes(read_array(stream).await?);
    Ok(Target { host, port })
}

async fn read_array<S, const N: usize>(stream: &mut S) -> std::io::Result<[u8; N]>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = [0; N];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn read_length_prefixed<S>(stream: &mut S) -> std::io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let [length] = read_array(stream).await?;
    let mut bytes = vec![0; length.into()];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.iter().zip(right).fold(0, |difference, (l, r)| difference | (l ^ r)) == 0
}

#[cfg(test)]
mod tests {
    use tokio::io::duplex;

    use super::*;

    fn credentials() -> SocksCredentials {
        SocksCredentials { username: "cove".to_string(), password: "secret".to_string() }
    }

    fn auth_message(username: &str, password: &str) -> Vec<u8> {
        let mut message = vec![AUTH_VERSION, username.len() as u8];
        message.extend_from_slice(username.as_bytes());
        message.push(password.len() as u8);
        message.extend_from_slice(password.as_bytes());
        message
    }

    #[tokio::test]
    async fn accepts_a_domain_connect_with_the_right_credentials() {
        let (mut client, mut server) = duplex(1024);

        let server_task = tokio::spawn(async move {
            let target = accept(&mut server, &credentials()).await.unwrap();
            reply(&mut server, Reply::Succeeded).await.unwrap();
            target
        });

        client.write_all(&[VERSION, 2, 0x00, METHOD_USERNAME_PASSWORD]).await.unwrap();
        assert_eq!(read_array::<_, 2>(&mut client).await.unwrap(), [VERSION, 0x02]);

        client.write_all(&auth_message("cove", "secret")).await.unwrap();
        assert_eq!(read_array::<_, 2>(&mut client).await.unwrap(), [AUTH_VERSION, 0x00]);

        let host = b"abcdefghijklmnop.onion";
        let mut request = vec![VERSION, COMMAND_CONNECT, 0, ADDRESS_DOMAIN, host.len() as u8];
        request.extend_from_slice(host);
        request.extend_from_slice(&50001u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        let mut answer = [0; 10];
        client.read_exact(&mut answer).await.unwrap();
        assert_eq!(answer[1], Reply::Succeeded as u8);

        let target = server_task.await.unwrap();
        assert_eq!(target.host, TargetHost::Name("abcdefghijklmnop.onion".to_string()));
        assert_eq!(target.port, 50001);
    }

    #[tokio::test]
    async fn rejects_clients_without_credentials() {
        let (mut client, mut server) = duplex(1024);
        let server_task = tokio::spawn(async move { accept(&mut server, &credentials()).await });

        client.write_all(&[VERSION, 1, 0x00]).await.unwrap();
        assert_eq!(read_array::<_, 2>(&mut client).await.unwrap(), [VERSION, 0xFF]);

        let error = server_task.await.unwrap().unwrap_err();
        assert!(matches!(error, HandshakeError::NoAcceptableMethod));
    }

    #[tokio::test]
    async fn rejects_the_wrong_password() {
        let (mut client, mut server) = duplex(1024);
        let server_task = tokio::spawn(async move { accept(&mut server, &credentials()).await });

        client.write_all(&[VERSION, 1, METHOD_USERNAME_PASSWORD]).await.unwrap();
        read_array::<_, 2>(&mut client).await.unwrap();

        client.write_all(&auth_message("cove", "guess")).await.unwrap();
        assert_eq!(read_array::<_, 2>(&mut client).await.unwrap(), [AUTH_VERSION, 0x01]);

        let error = server_task.await.unwrap().unwrap_err();
        assert!(matches!(error, HandshakeError::BadCredentials));
    }
}

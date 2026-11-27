//! Keeps every socket discovery opens on the Wi-Fi or Ethernet network it is searching
//!
//! An address in the device's own subnet doesn't guarantee its traffic stays local: a VPN that
//! routes everything, as Android VPNs do unless they allow local network access, would carry
//! the sweep into the tunnel and on to whatever network sits behind it. Binding each socket to
//! the network itself keeps it off the VPN, or fails when the VPN doesn't allow that, in which
//! case discovery doesn't search rather than search through the tunnel

use std::{io, net::SocketAddrV4};

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::TcpSocket;

/// The network an interface belongs to, which each discovery socket is bound to
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Enum)]
pub enum LanBinding {
    /// An Android network, by `android.net.Network.getNetworkHandle()`
    AndroidNetwork { handle: u64 },
    /// A network interface by its index, such as `if_nametoindex("en0")` on iOS
    InterfaceIndex { index: u32 },
}

impl LanBinding {
    /// A TCP socket that can only connect over this network
    pub fn tcp_socket(self) -> io::Result<TcpSocket> {
        let socket = TcpSocket::new_v4()?;
        self.bind(SockRef::from(&socket))?;

        Ok(socket)
    }

    /// A non-blocking UDP socket bound to `source` that can only send over this network
    pub fn udp_socket(self, source: SocketAddrV4) -> io::Result<Socket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_nonblocking(true)?;
        self.bind(SockRef::from(&socket))?;
        socket.bind(&source.into())?;

        Ok(socket)
    }

    fn bind(self, socket: SockRef<'_>) -> io::Result<()> {
        match self {
            Self::AndroidNetwork { handle } => bind_to_android_network(handle, socket),
            Self::InterfaceIndex { index } => bind_to_interface(index, socket),
        }
    }
}

#[cfg(target_os = "android")]
fn bind_to_android_network(handle: u64, socket: SockRef<'_>) -> io::Result<()> {
    use std::{os::fd::AsRawFd as _, sync::OnceLock};

    // `Network.bindSocket` in the NDK, which refuses with EPERM when a VPN that doesn't allow
    // bypassing covers the app
    type SetSockNetwork = unsafe extern "C" fn(network: u64, fd: libc::c_int) -> libc::c_int;

    // looked up when first used rather than linked: the library is built against the API 21 NDK
    // stubs, which predate this API 23 function, though every Android version the app runs on has it
    static SET_SOCK_NETWORK: OnceLock<Option<SetSockNetwork>> = OnceLock::new();

    let set_sock_network = SET_SOCK_NETWORK.get_or_init(|| {
        // SAFETY: both names are NUL-terminated literals, and libandroid stays loaded for the
        // life of the process, so the symbol stays valid
        unsafe {
            let library = libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW);
            if library.is_null() {
                return None;
            }

            let symbol = libc::dlsym(library, c"android_setsocknetwork".as_ptr());
            (!symbol.is_null())
                .then(|| std::mem::transmute::<*mut libc::c_void, SetSockNetwork>(symbol))
        }
    });

    let Some(set_sock_network) = set_sock_network else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "android_setsocknetwork is missing",
        ));
    };

    // SAFETY: the function has the NDK's signature, and the descriptor is an open socket borrowed
    // for the length of the call
    let result = unsafe { set_sock_network(handle, socket.as_raw_fd()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(not(target_os = "android"))]
fn bind_to_android_network(_handle: u64, _socket: SockRef<'_>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Android networks only exist on Android"))
}

// `IP_BOUND_IF` on Apple platforms and `SO_BINDTOIFINDEX` on Linux
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
fn bind_to_interface(index: u32, socket: SockRef<'_>) -> io::Result<()> {
    // index 0 would remove the binding instead of setting one
    let index = std::num::NonZeroU32::new(index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "interface index 0"))?;

    socket.bind_device_by_index_v4(Some(index))
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
fn bind_to_interface(_index: u32, _socket: SockRef<'_>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "can't bind a socket to an interface here"))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The loopback interface, which the tests' servers listen on
    pub fn loopback() -> LanBinding {
        let index = ["lo", "lo0"]
            .into_iter()
            .map(|name| {
                let name = std::ffi::CString::new(name).unwrap();

                // SAFETY: the name is a valid NUL-terminated string for the length of the call
                unsafe { libc::if_nametoindex(name.as_ptr()) }
            })
            .find(|index| *index != 0)
            .expect("a loopback interface");

        LanBinding::InterfaceIndex { index }
    }

    #[test]
    fn an_interface_that_does_not_exist_cannot_be_bound() {
        let missing = LanBinding::InterfaceIndex { index: u32::MAX };

        assert!(missing.tcp_socket().is_err());
        assert!(LanBinding::InterfaceIndex { index: 0 }.tcp_socket().is_err());
    }

    #[test]
    fn android_networks_cannot_be_bound_off_android() {
        let error = LanBinding::AndroidNetwork { handle: 1 }.tcp_socket().unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }
}

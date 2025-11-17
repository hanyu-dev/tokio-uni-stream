#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_inherent_impl)]
#![allow(clippy::must_use_candidate)]
#![allow(unknown_lints)]

#[cfg(not(unix))]
mod default;
#[cfg(unix)]
mod unix;

use std::io;
use std::net::{SocketAddr, ToSocketAddrs as _};

pub use socket2::TcpKeepalive;
use uni_addr::{UniAddr, UniAddrInner};

#[cfg(not(unix))]
pub use self::default::{OwnedReadHalf, OwnedWriteHalf, UniStream};
#[cfg(unix)]
pub use self::unix::{OwnedReadHalf, OwnedWriteHalf, UniStream};

wrapper_lite::wrapper! {
    #[wrapper_impl(Debug)]
    #[wrapper_impl(AsRef)]
    #[wrapper_impl(AsMut)]
    #[wrapper_impl(DerefMut)]
    /// Thin wrapper around [`Socket`](socket2::Socket).
    ///
    /// The socket is always created in non-blocking mode.
    pub struct UniSocket {
        /// The underlying socket.
        inner: socket2::Socket,

        #[cfg(unix)]
        /// Whether the socket is a Unix domain socket.
        is_unix_socket: bool,
    }
}

impl UniSocket {
    /// Creates a new [`UniSocket`] and bind to the specified address.
    ///
    /// # Errors
    ///
    /// See [`socket2::Socket::new`] and [`socket2::Socket::bind`] for possible
    /// errors.
    pub fn bind(addr: &UniAddr) -> io::Result<Self> {
        let ty = socket2::Type::STREAM;

        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        let ty = ty.nonblocking();

        #[cfg(unix)]
        if let UniAddrInner::Unix(addr) = addr.as_inner() {
            let inner = socket2::Socket::new(socket2::Domain::UNIX, ty, None)?;

            #[cfg(not(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd"
            )))]
            inner.set_nonblocking(true)?;

            inner.bind(&socket2::SockAddr::unix(addr.to_os_string())?)?;

            return Ok(Self {
                inner,
                is_unix_socket: true,
            });
        }

        let (addr, domain) = match addr.as_inner() {
            UniAddrInner::Inet(addr @ SocketAddr::V4(_)) => (*addr, socket2::Domain::IPV4),
            UniAddrInner::Inet(addr @ SocketAddr::V6(_)) => (*addr, socket2::Domain::IPV6),
            UniAddrInner::Host(addr) => {
                // Note: we only take the first resolved address here.
                let addr = addr
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no addresses found"))?;

                match addr {
                    SocketAddr::V4(_) => (addr, socket2::Domain::IPV4),
                    SocketAddr::V6(_) => (addr, socket2::Domain::IPV6),
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "unsupported address type",
                ))
            }
        };

        let inner = socket2::Socket::new(domain, ty, Some(socket2::Protocol::TCP))?;

        #[cfg(not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        inner.set_nonblocking(true)?;

        // Set SO_REUSEADDR and SO_REUSEPORT to true for graceful restarts.
        inner.set_reuse_address(true)?;

        #[cfg(all(
            unix,
            not(target_os = "solaris"),
            not(target_os = "illumos"),
            not(target_os = "cygwin"),
        ))]
        inner.set_reuse_port(true)?;

        inner.bind(&socket2::SockAddr::from(addr))?;

        Ok(Self {
            inner,
            #[cfg(unix)]
            is_unix_socket: false,
        })
    }

    /// Calls `listen` on the underlying socket to prepare it to receive new
    /// connections.
    ///
    /// # Errors
    ///
    /// See [`socket2::Socket::listen`], [`tokio::net::TcpListener::from_std`],
    /// and [`tokio::net::UnixListener::from_std`] for possible errors.
    pub fn listen(self, backlog: u32) -> io::Result<UniListener> {
        #[allow(clippy::cast_possible_wrap)]
        self.inner.listen(backlog as i32)?;

        #[cfg(unix)]
        if self.is_unix_socket {
            return tokio::net::UnixListener::from_std(self.inner.into()).map(UniListener::Unix);
        }

        tokio::net::TcpListener::from_std(self.inner.into()).map(UniListener::Tcp)
    }
}

#[derive(Debug)]
/// A unified listener that can listen on both TCP and Unix domain sockets.
pub enum UniListener {
    /// [`TcpListener`](tokio::net::TcpListener)
    Tcp(tokio::net::TcpListener),

    #[cfg(unix)]
    /// [`UnixListener`](tokio::net::UnixListener)
    Unix(tokio::net::UnixListener),
}

impl UniListener {
    /// Creates a new [`UniListener`], which will be bound to the specified
    /// address.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port
    /// to this listener. The port allocated can be queried via the
    /// [`local_addr`](Self::local_addr) method.
    ///
    /// The address type can be a host. If it yields multiple addresses, `bind`
    /// will be attempted with each of the addresses until one succeeds and
    /// returns the listener. If none of the addresses succeed in creating a
    /// listener, the error returned from the last attempt (the last
    /// address) is returned.
    ///
    /// # Errors
    ///
    /// See [`tokio::net::TcpListener::bind`] and
    /// [`tokio::net::UnixListener::bind`] for possible errors.
    pub async fn bind(addr: &UniAddr) -> io::Result<Self> {
        match addr.as_inner() {
            UniAddrInner::Inet(addr) => tokio::net::TcpListener::bind(addr).await.map(Self::Tcp),
            #[cfg(unix)]
            UniAddrInner::Unix(addr) => {
                tokio::net::UnixListener::bind(addr.to_os_string()).map(Self::Unix)
            }
            UniAddrInner::Host(addr) => tokio::net::TcpListener::bind(&**addr).await.map(Self::Tcp),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "unsupported address type",
            )),
        }
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, when binding to port 0 to figure out
    /// which port was actually bound
    ///
    /// # Errors
    ///
    /// See [`tokio::net::TcpListener::local_addr`] and
    /// [`tokio::net::UnixListener::local_addr`] for possible errors.
    pub fn local_addr(&self) -> io::Result<UniAddr> {
        match self {
            UniListener::Tcp(listener) => listener.local_addr().map(UniAddr::from),
            #[cfg(unix)]
            UniListener::Unix(listener) => listener.local_addr().map(UniAddr::from),
        }
    }

    /// Returns a [`SockRef`](socket2::SockRef) to the underlying socket for
    /// configuration.
    pub fn as_socket_ref(&self) -> socket2::SockRef<'_> {
        match self {
            UniListener::Tcp(listener) => listener.into(),
            #[cfg(unix)]
            UniListener::Unix(listener) => listener.into(),
        }
    }

    /// Accepts a new incoming connection from this listener.
    ///
    /// This function will yield once a new TCP connection is established. When
    /// established, the corresponding [`UniStream`] and the remote peer's
    /// address will be returned.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If the method is used as the event in a
    /// `tokio::select!` statement and some other branch completes first, then
    /// it is guaranteed that no new connections were accepted by this
    /// method.
    ///
    /// # Errors
    ///
    /// See [`tokio::net::TcpListener::accept`] and
    /// [`tokio::net::UnixListener::accept`] for possible errors.
    pub async fn accept(&self) -> io::Result<(UniStream, UniAddr)> {
        match self {
            UniListener::Tcp(listener) => {
                let (stream, addr) = listener.accept().await?;
                Ok((UniStream::try_from(stream)?, UniAddr::from(addr)))
            }
            #[cfg(unix)]
            UniListener::Unix(listener) => {
                let (stream, addr) = listener.accept().await?;
                Ok((UniStream::try_from(stream)?, UniAddr::from(addr)))
            }
        }
    }
}

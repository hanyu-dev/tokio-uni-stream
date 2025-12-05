//! A unified stream type for both TCP and Unix domain sockets.
//!
//! This module is to keep the API consistency across different platforms.
//! On Windows, only TCP sockets are supported.

use std::future::poll_fn;
use std::mem::MaybeUninit;
use std::net::{Shutdown, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io};

use socket2::SockRef;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpSocket;
use uni_addr::{UniAddr, UniAddrInner};

/// A simple wrapper of [`tokio::net::TcpSocket`].
pub struct UniSocket {
    inner: tokio::net::TcpSocket,
}

impl fmt::Debug for UniSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniSocket").finish()
    }
}

impl UniSocket {
    #[inline]
    const fn from_inner(inner: TcpSocket) -> Self {
        Self { inner }
    }

    /// Creates a new [`UniSocket`], and applies the given initialization
    /// function to the underlying socket.
    ///
    /// The given address determines the socket type, and the caller should bind
    /// to / connect to the address later.
    pub fn new(addr: &UniAddr) -> io::Result<Self> {
        match addr.as_inner() {
            UniAddrInner::Inet(SocketAddr::V4(_)) => TcpSocket::new_v4().map(Self::from_inner),
            UniAddrInner::Inet(SocketAddr::V6(_)) => TcpSocket::new_v6().map(Self::from_inner),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "unsupported address type",
            )),
        }
    }

    /// Binds the socket to the specified address.
    ///
    /// Notes that the address must be the one used to create the socket.
    pub fn bind(self, addr: &UniAddr) -> io::Result<Self> {
        match addr.as_inner() {
            UniAddrInner::Inet(addr) => self.inner.bind(*addr)?,
            UniAddrInner::Host(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "The Host address type must be resolved before creating a socket",
                ))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "unsupported address type",
                ))
            }
        }

        Ok(self)
    }

    /// Mark a socket as ready to accept incoming connection requests using
    /// [`UniListener::accept`].
    ///
    /// This function directly corresponds to the `listen(2)` function on
    /// Windows and Unix.
    ///
    /// An error will be returned if `listen` or `connect` has already been
    /// called on this builder.
    pub fn listen(self, backlog: u32) -> io::Result<UniListener> {
        self.inner.listen(backlog).map(UniListener::from_inner)
    }

    /// Initiates and completes a connection on this socket to the specified
    /// address.
    ///
    /// This function directly corresponds to the `connect(2)` function on
    /// Windows and Unix.
    ///
    /// An error will be returned if `connect` has already been called.
    pub async fn connect(self, addr: &UniAddr) -> io::Result<UniStream> {
        match addr.as_inner() {
            UniAddrInner::Inet(addr) => self.inner.connect(*addr).await.map(UniStream::from_inner),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "unsupported address type",
            )),
        }
    }

    /// Returns the socket address of the local half of this socket.
    ///
    /// This function directly corresponds to the `getsockname(2)` function on
    /// Windows.
    ///
    /// # Notes
    ///
    /// Depending on the OS this may return an error if the socket is not
    /// [bound](Socket::bind).
    pub fn local_addr(&self) -> io::Result<UniAddr> {
        self.inner.local_addr().map(UniAddr::from)
    }

    /// Returns a [`SockRef`] to the underlying socket for configuration.
    pub fn as_socket_ref(&self) -> SockRef<'_> {
        SockRef::from(&self.inner)
    }
}

wrapper_lite::wrapper!(
    /// A simple wrapper of [`tokio::net::TcpListener`].
    pub struct UniListener(tokio::net::TcpListener);
);

impl fmt::Debug for UniListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniListener")
            .field("local_addr", &self.inner.local_addr())
            .finish()
    }
}

impl TryFrom<std::net::TcpListener> for UniListener {
    type Error = io::Error;

    /// Converts a standard library TCP listener into a unified [`UniListener`].
    fn try_from(listener: std::net::TcpListener) -> Result<Self, Self::Error> {
        listener.set_nonblocking(true)?;

        Ok(Self::from_inner(listener.try_into()?))
    }
}

impl TryFrom<tokio::net::TcpListener> for UniListener {
    type Error = io::Error;

    /// Converts a Tokio library TCP listener into a unified [`UniListener`].
    ///
    /// # Errors
    ///
    /// Actually, this is infallible and always returns `Ok`, for APIs
    /// consistency.
    fn try_from(listener: tokio::net::TcpListener) -> Result<Self, Self::Error> {
        Ok(Self::from_inner(listener))
    }
}

impl UniListener {
    /// Accepts an incoming connection to this listener, and returns the
    /// accepted stream and the peer address.
    ///
    /// This method will retry on non-deadly errors, including:
    ///
    /// - `ECONNREFUSED`.
    /// - `ECONNABORTED`.
    /// - `ECONNRESET`.
    pub async fn accept(&self) -> io::Result<(UniStream, UniAddr)> {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => {
                    return Ok((UniStream::from_inner(stream), UniAddr::from(addr)))
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                    ) => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Accepts an incoming connection to this listener, and returns the
    /// accepted stream and the peer address.
    ///
    /// Notes that on multiple calls to [`poll_accept`](Self::poll_accept), only
    /// the waker from the [`Context`] passed to the most recent call is
    /// scheduled to receive a wakeup. Unless you are implementing your own
    /// future accepting connections, you probably want to use the asynchronous
    /// [`accept`](Self::accept) method instead.
    pub fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<io::Result<(UniStream, UniAddr)>> {
        loop {
            match self.inner.poll_accept(cx) {
                Poll::Ready(Ok((stream, addr))) => {
                    return Poll::Ready(Ok((UniStream::from_inner(stream), UniAddr::from(addr))))
                }
                Poll::Ready(Err(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                    ) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// Returns the socket address of the local half of this socket.
    ///
    /// This function directly corresponds to the `getsockname(2)` function on
    /// Windows.
    ///
    /// # Notes
    ///
    /// Depending on the OS this may return an error if the socket is not
    /// [bound](Socket::bind).
    pub fn local_addr(&self) -> io::Result<UniAddr> {
        self.inner.local_addr().map(UniAddr::from)
    }

    /// Returns a [`SockRef`] to the underlying socket for configuration.
    pub fn as_socket_ref(&self) -> SockRef<'_> {
        SockRef::from(&self.inner)
    }
}

wrapper_lite::wrapper!(
    /// A simple wrapper of [`tokio::net::TcpStream`].
    pub struct UniStream(tokio::net::TcpStream);
);

impl TryFrom<tokio::net::TcpStream> for UniStream {
    type Error = io::Error;

    /// Converts a Tokio TCP stream into a [`UniStream`].
    ///
    /// # Errors
    ///
    /// This is infallible and always returns `Ok`, for APIs consistency.
    fn try_from(value: tokio::net::TcpStream) -> Result<Self, Self::Error> {
        Ok(Self::from_inner(value))
    }
}

impl TryFrom<std::net::TcpStream> for UniStream {
    type Error = std::io::Error;

    /// Converts a standard library TCP stream into a [`Stream`].
    ///
    /// # Panics
    ///
    /// This function panics if it is not called from within a runtime with
    /// IO enabled.
    ///
    /// The runtime is usually set implicitly when this function is called
    /// from a future driven by a tokio runtime, otherwise runtime can be
    /// set explicitly with `Runtime::enter` function.
    fn try_from(stream: std::net::TcpStream) -> Result<Self, Self::Error> {
        stream.set_nonblocking(true)?;

        Ok(Self::from_inner(stream.try_into()?))
    }
}

impl UniStream {
    /// Initiates and completes a connection on this socket to the specified
    /// address.
    ///
    /// This function directly corresponds to the `connect(2)` function on
    /// Windows and Unix.
    ///
    /// An error will be returned if `connect` has already been called.
    pub async fn connect(self, addr: &UniAddr) -> io::Result<UniStream> {
        match addr.as_inner() {
            UniAddrInner::Inet(addr) => tokio::net::TcpStream::connect(*addr)
                .await
                .map(UniStream::from_inner),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "unsupported address type",
            )),
        }
    }

    /// Returns the socket address of the local half of this socket.
    ///
    /// This function directly corresponds to the `getsockname(2)` function on
    /// Windows and Unix.
    ///
    /// # Notes
    ///
    /// Depending on the OS this may return an error if the socket is not
    /// [bound](Socket::bind).
    pub fn local_addr(&self) -> io::Result<UniAddr> {
        self.inner.local_addr().map(UniAddr::from)
    }

    /// Returns the socket address of the remote peer of this socket.
    ///
    /// This function directly corresponds to the `getpeername(2)` function on
    /// Windows and Unix.
    ///
    /// # Notes
    ///
    /// This returns an error if the socket is not
    /// [`connect`ed](Socket::connect).
    pub fn peer_addr(&self) -> io::Result<UniAddr> {
        self.inner.peer_addr().map(UniAddr::from)
    }

    /// Receives data on the socket from the remote adress to which it is
    /// connected, without removing that data from the queue. On success,
    /// returns the number of bytes peeked.
    ///
    /// Successive calls return the same data. This is accomplished by passing
    /// `MSG_PEEK` as a flag to the underlying `recv` system call.
    pub async fn peek(&self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        buf.fill(MaybeUninit::new(0));

        #[allow(unsafe_code)]
        let buf = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len()) };

        self.inner.peek(buf).await
    }

    /// Receives data on the socket from the remote adress to which it is
    /// connected, without removing that data from the queue. On success,
    /// returns the number of bytes peeked.
    ///
    /// Successive calls return the same data. This is accomplished by passing
    /// `MSG_PEEK` as a flag to the underlying `recv` system call.
    ///
    /// Notes that on multiple calls to [`poll_peek`](Self::poll_peek), only
    /// the waker from the [`Context`] passed to the most recent call is
    /// scheduled to receive a wakeup. Unless you are implementing your own
    /// future accepting connections, you probably want to use the asynchronous
    /// [`accept`](Self::accept) method instead.
    pub fn poll_peek(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<usize>> {
        self.get_mut().inner.poll_peek(cx, buf)
    }

    /// Returns a [`SockRef`] to the underlying socket for configuration.
    pub fn as_socket_ref(&self) -> SockRef<'_> {
        SockRef::from(&self.inner)
    }

    #[inline]
    /// Receives data on the socket from the remote address to which it is
    /// connected.
    pub async fn read(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        let mut this = Pin::new(&mut self.inner);

        let buf = &mut ReadBuf::uninit(buf);

        poll_fn(|cx| this.as_mut().poll_read(cx, buf)).await?;

        Ok(buf.filled().len())
    }

    #[inline]
    /// Sends data on the socket to a connected peer.
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut this = Pin::new(&mut self.inner);

        poll_fn(|cx| this.as_mut().poll_write(cx, buf)).await
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified
    /// portions to return immediately with an appropriate value.
    pub fn shutdown(&self, shutdown: Shutdown) -> io::Result<()> {
        match self.as_socket_ref().shutdown(shutdown) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotConnected => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// See [`tokio::net::TcpStream::into_split`].
    pub fn into_split(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        self.inner.into_split()
    }
}

impl AsyncRead for UniStream {
    #[inline]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for UniStream {
    #[inline]
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    #[inline]
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// Re-export split halves
pub use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

#[cfg(windows)]
mod sys {
    use std::os::windows::io::{AsRawSocket, AsSocket, BorrowedSocket, RawSocket};

    use super::{UniListener, UniSocket, UniStream};

    impl AsSocket for UniSocket {
        fn as_socket(&self) -> BorrowedSocket<'_> {
            self.inner.as_socket()
        }
    }

    impl AsRawSocket for UniSocket {
        fn as_raw_socket(&self) -> RawSocket {
            self.inner.as_raw_socket()
        }
    }

    impl AsSocket for UniListener {
        fn as_socket(&self) -> BorrowedSocket<'_> {
            self.inner.as_socket()
        }
    }

    impl AsRawSocket for UniListener {
        fn as_raw_socket(&self) -> RawSocket {
            self.inner.as_raw_socket()
        }
    }

    impl AsSocket for UniStream {
        fn as_socket(&self) -> BorrowedSocket<'_> {
            self.inner.as_socket()
        }
    }

    impl AsRawSocket for UniStream {
        fn as_raw_socket(&self) -> RawSocket {
            self.inner.as_raw_socket()
        }
    }
}

#![allow(missing_docs)]
#![allow(non_snake_case)]

use std::mem::MaybeUninit;
use std::time::Duration;

use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
// use uni_socket::windows::UniSocket;
use uni_addr::UniAddr;
use uni_socket::{UniListener, UniSocket};

#[cfg(any(
    target_os = "ios",
    target_os = "visionos",
    target_os = "macos",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "illumos",
    target_os = "solaris",
    target_os = "linux",
    target_os = "android",
    target_os = "fuchsia",
))]
#[rstest]
#[tokio::test]
async fn test_UniSocket_bind_device(
    #[values("0.0.0.0:0", "[::]:0")] addr: &str,
    #[values(true, false)] device: bool,
) {
    use network_interface::NetworkInterfaceConfig;
    use uni_socket::unix::UniSocket;

    let addr = UniAddr::new(addr).unwrap();

    let socket = UniSocket::new(&addr).unwrap();

    let if_name = if device {
        let interfaces = match network_interface::NetworkInterface::show() {
            Ok(ifaces) => ifaces,
            Err(e) => {
                println!("Failed to get network interfaces: {e}");

                return;
            }
        };

        let Some(if_name) = interfaces
            .iter()
            .find(|n| n.mac_addr.is_some() && !n.addr.is_empty())
        else {
            println!("No possible interface for test? All known interfaces: {interfaces:#?}");

            return;
        };

        Some(if_name.name.clone())
    } else {
        None
    };

    socket.bind_device(&addr, if_name.as_deref()).unwrap();
}

#[rstest]
#[case("127.0.0.1:0")]
#[case("[::1]:0")]
#[cfg_attr(unix, case("unix:///tmp/test_echo.sock"))]
#[cfg_attr(
    any(target_os = "android", target_os = "linux"),
    case("unix://@test_echo.socket")
)]
#[tokio::test]
async fn test_echo(#[case] addr: &str) {
    let listener = {
        let addr = UniAddr::new(addr).unwrap();

        UniSocket::new(&addr)
            .unwrap()
            .bind(&addr)
            .unwrap()
            .listen(128)
            .unwrap()
    };

    let connected = {
        let server_addr = listener.local_addr().unwrap();

        UniSocket::new(&server_addr)
            .unwrap()
            .connect(&server_addr)
            .await
            .unwrap()
    };

    let server = tokio::spawn(async move {
        let (mut socket, addr) = listener.accept().await.unwrap();

        println!("[SERVER] Accepted connection from {:?}", addr);

        loop {
            let mut buf = [MaybeUninit::zeroed(); 1024];

            let n = socket.read(&mut buf).await.unwrap();

            if n == 0 {
                println!("[SERVER] Read EOF");

                break;
            }

            println!("[SERVER] Read {n} bytes. Echoing back...");

            #[allow(unsafe_code)]
            let buf = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), n) };

            let mut write_offset = 0;

            while write_offset < n {
                let written = socket.write(&buf[write_offset..]).await.unwrap();

                if written == 0 {
                    println!("[SERVER] Write EOF");
                    break;
                }

                write_offset += written;
            }

            assert_eq!(
                write_offset, n,
                "[SERVER] Failed to echo back all bytes: expected to write {n}, wrote \
                 {write_offset}"
            );

            println!("[SERVER] Echoed back {write_offset} bytes");
        }
    });

    let client = tokio::spawn(async move {
        let mut connected = connected;

        let msg = "Hello, world! This is a test message.";

        println!("[CLIENT] Sent message");

        let mut write_offset = 0;

        while write_offset < msg.len() {
            let written = connected
                .write(&msg.as_bytes()[write_offset..])
                .await
                .unwrap();

            if written == 0 {
                println!("[CLIENT] Write EOF");
                break;
            }

            write_offset += written;
        }

        assert_eq!(
            write_offset,
            msg.len(),
            "[CLIENT] Failed to send all bytes: expected to write {}, wrote {write_offset}",
            msg.len(),
        );

        let mut buf: Vec<u8> = Vec::with_capacity(msg.len());

        let mut remaining = msg.len();

        while remaining > 0 {
            let n = connected
                .read(&mut buf.spare_capacity_mut()[..remaining])
                .await
                .unwrap();

            assert_ne!(n, 0, "[CLIENT] Read early EOF");

            remaining -= n;

            #[allow(unsafe_code)]
            unsafe {
                buf.set_len(buf.len() + n)
            };
        }

        println!("[CLIENT] Received echo");

        assert_eq!(
            msg.as_bytes(),
            &buf[..],
            "[CLIENT] Echoed message does not match sent message: expected {:?}, got {:?}",
            msg,
            String::from_utf8_lossy(&buf[..])
        );
    });

    tokio::try_join!(server, client).unwrap();
}

#[rstest]
#[case("127.0.0.1:0")]
#[case("[::1]:0")]
#[cfg_attr(unix, case("unix:///tmp/test_echo_poll.sock"))]
#[cfg_attr(
    any(target_os = "android", target_os = "linux"),
    case("unix://@test_echo_poll.socket")
)]
#[tokio::test]
async fn test_echo_poll(#[case] addr: &str) {
    let listener = {
        let addr = UniAddr::new(addr).unwrap();

        UniSocket::new(&addr)
            .unwrap()
            .bind(&addr)
            .unwrap()
            .listen(128)
            .unwrap()
    };

    let connected = {
        let server_addr = listener.local_addr().unwrap();

        UniSocket::new(&server_addr)
            .unwrap()
            .connect(&server_addr)
            .await
            .unwrap()
    };

    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut socket, addr) = listener.accept().await.unwrap();

        println!("[SERVER] Accepted connection from {:?}", addr);

        loop {
            let mut buf = [0u8; 1024];

            let n = AsyncReadExt::read(&mut socket, &mut buf).await.unwrap();

            if n == 0 {
                println!("[SERVER] Read EOF");

                break;
            }

            println!("[SERVER] Read {n} bytes. Echoing back...");

            AsyncWriteExt::write_all(&mut socket, &buf[..n])
                .await
                .unwrap();

            println!("[SERVER] Echoed back {n} bytes");
        }
    });

    let client = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut connected = connected;

        let msg = b"Hello, world! This is a test message.";

        connected.write_all(msg).await.unwrap();

        println!("[CLIENT] Sent message");

        let mut buf = vec![0u8; msg.len()];

        connected.read_exact(&mut buf).await.unwrap();

        println!("[CLIENT] Received echo");

        assert_eq!(
            msg,
            &buf[..],
            "[CLIENT] Echoed message does not match sent message: expected {:?}, got {:?}",
            msg,
            String::from_utf8_lossy(&buf[..])
        );
    });

    tokio::try_join!(server, client).unwrap();
}

#[rstest]
#[case(1024)]
#[case(1024 * 16)]
#[case(1024 * 256)]
#[case(1024 * 1024)]
#[case(1024 * 1024 * 4)]
#[case(1024 * 1024 * 16)]
#[tokio::test]
async fn test_UniStream_read_write(#[case] bytes: usize) {
    let listener = {
        let addr = "127.0.0.1:0".parse().unwrap();

        UniSocket::new(&addr)
            .unwrap()
            .bind(&addr)
            .unwrap()
            .listen(u32::MAX)
            .unwrap()
    };

    let server_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(test_UniStream_read_write_server(listener, bytes));
    let client = tokio::spawn(test_UniStream_read_write_client(server_addr, bytes));

    tokio::select! {
        res = server => {
            res.unwrap();
        }
        res = client => {
            res.unwrap();
        }
        _ = tokio::time::sleep(Duration::from_secs(10)) => {
            panic!("Test timed out!");
        }
    }
}

async fn test_UniStream_read_write_server(listener: UniListener, bytes: usize) {
    loop {
        let (mut accepted, peer_addr) = listener.accept().await.unwrap();

        println!("[SERVER] Accepted connection from {peer_addr:?}");

        tokio::spawn(async move {
            let mut buf = vec![0u8; bytes];

            accepted.read_exact(&mut buf).await.unwrap();
            println!("[SERVER] Received {bytes} bytes, echoing back...");
            accepted.write_all(&buf).await.unwrap();
            println!("[SERVER] Echoed back {bytes} bytes");
        });
    }
}

async fn test_UniStream_read_write_client(echo_server_addr: UniAddr, bytes: usize) {
    let mut socket = UniSocket::new(&echo_server_addr)
        .unwrap()
        .connect(&echo_server_addr)
        .await
        .unwrap();

    let rand = &mut Rand::new();

    let mut input = Vec::with_capacity(bytes);

    input.resize_with(bytes, || rand.random_u32() as u8);

    socket.write_all(&input).await.unwrap();

    println!("[CLIENT] Sent {bytes} bytes, waiting for echo...");

    let mut buf = vec![0u8; bytes];

    socket.read_exact(&mut buf).await.unwrap();

    println!("[CLIENT] Received echo");

    assert_eq!(buf, input);
}

// https://github.com/engusmaze/frand, APACHE-2.0 Licensed
struct Rand {
    seed: u64,
}

impl Rand {
    #[inline]
    fn new() -> Self {
        #[allow(unsafe_code)]
        let [seed, b]: [u64; 2] = unsafe { std::mem::transmute(std::time::Instant::now()) };
        let mut rand = Self { seed };
        rand.mix(b);
        rand
    }

    #[inline]
    fn mix(&mut self, value: u64) {
        self.seed = hash64(self.seed.wrapping_add(value) ^ value << 10);
    }

    #[inline]
    fn random_u32(&mut self) -> u32 {
        let value = self.seed.wrapping_add(12964901029718341801);
        self.seed = value;
        (value.wrapping_mul(18162115696561729952 ^ value) >> 32) as u32
    }
}

#[inline(always)]
const fn hash64(mut hash: u64) -> u64 {
    hash = (hash ^ hash >> 32).wrapping_mul(4997996261773036203);
    hash = (hash ^ hash >> 32).wrapping_mul(4997996261773036203);
    hash ^ hash >> 32
}

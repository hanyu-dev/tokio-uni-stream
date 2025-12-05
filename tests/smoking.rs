#![allow(missing_docs)]
#![allow(non_snake_case)]

use std::mem::MaybeUninit;

use rstest::rstest;
// use uni_socket::windows::UniSocket;
use uni_addr::UniAddr;
use uni_socket::UniSocket;

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

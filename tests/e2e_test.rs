use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use bore_cli::{client::Client, server::Server};
use lazy_static::lazy_static;
use rstest::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time;

lazy_static! {
    /// Guard to make sure that tests are run serially, not concurrently.
    static ref SERIAL_GUARD: Mutex<()> = Mutex::new(());
}

/// Spawn the server, giving some time for the control port TcpListener to start.
async fn spawn_server(secret: Option<&str>) {
    tokio::spawn(Server::new(1024, secret).listen());
    time::sleep(Duration::from_millis(50)).await;
}

/// Spawns a client with randomly assigned ports, returning the listener and remote address.
async fn spawn_client(secret: Option<&str>) -> Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind("localhost:0").await?;
    let client = Client::new(listener.local_addr()?.port(), "localhost", 0, secret).await?;
    let remote_addr = ([0, 0, 0, 0], client.remote_port()).into();
    tokio::spawn(client.listen());
    Ok((listener, remote_addr))
}

#[rstest]
#[tokio::test]
async fn basic_proxy(#[values(None, Some(""), Some("abc"))] secret: Option<&str>) -> Result<()> {
    let _guard = SERIAL_GUARD.lock().await;

    spawn_server(secret).await;
    let (listener, addr) = spawn_client(secret).await?;

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = [0u8; 11];
        stream.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"hello world");

        stream.write_all(b"I can send a message too!").await?;
        anyhow::Ok(())
    });

    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(b"hello world").await?;

    let mut buf = [0u8; 25];
    stream.read_exact(&mut buf).await?;
    assert_eq!(&buf, b"I can send a message too!");

    // Ensure that the client end of the stream is closed now.
    assert_eq!(stream.read(&mut buf).await?, 0);

    // Also ensure that additional connections do not produce any data.
    let mut stream = TcpStream::connect(addr).await?;
    assert_eq!(stream.read(&mut buf).await?, 0);

    Ok(())
}

#[rstest]
#[case(None, Some("my secret"))]
#[case(Some("my secret"), None)]
#[tokio::test]
async fn mismatched_secret(
    #[case] server_secret: Option<&str>,
    #[case] client_secret: Option<&str>,
) {
    let _guard = SERIAL_GUARD.lock().await;

    spawn_server(server_secret).await;
    assert!(spawn_client(client_secret).await.is_err());
}

#[tokio::test]
async fn invalid_address() -> Result<()> {
    // We don't need the serial guard for this test because it doesn't create a server.
    async fn check_address(to: &str, use_secret: bool) -> Result<()> {
        match Client::new(5000, to, 0, use_secret.then(|| "a secret")).await {
            Ok(_) => Err(anyhow!("expected error for {to}, use_secret={use_secret}")),
            Err(_) => Ok(()),
        }
    }
    tokio::try_join!(
        check_address("google.com", false),
        check_address("google.com", true),
        check_address("nonexistent.domain.for.demonstration", false),
        check_address("nonexistent.domain.for.demonstration", true),
        check_address("malformed !$uri$%", false),
        check_address("malformed !$uri$%", true),
    )?;
    Ok(())
}

//! Server implementation for the `bore` service.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::shared::{proxy, ClientMessage, ServerMessage, CONTROL_PORT};

/// State structure for the server.
pub struct Server {
    /// The minimum TCP port that can be forwarded.
    pub min_port: u16,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16) -> Server {
        Server {
            min_port,
            conns: Arc::new(DashMap::new()),
        }
    }

    /// Start the server, listening for new connections.
    pub async fn listen(self) -> Result<()> {
        let this = Arc::new(self);
        let addr = SocketAddr::from(([0, 0, 0, 0], CONTROL_PORT));
        let listener = TcpListener::bind(&addr).await?;
        info!(?addr, "server listening");

        loop {
            let (stream, addr) = listener.accept().await?;
            let this = Arc::clone(&this);
            tokio::spawn(
                async move {
                    info!("incoming connection");
                    if let Err(err) = this.handle_connection(stream).await {
                        warn!(%err, "connection exited with error");
                    } else {
                        info!("connection exited");
                    }
                }
                .instrument(info_span!("control", ?addr)),
            );
        }
    }

    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let mut stream = BufReader::new(stream);

        let mut buf = Vec::new();
        let msg = next_mp(&mut stream, &mut buf).await?;

        match msg {
            Some(ClientMessage::Hello(port)) => {
                if port < self.min_port {
                    warn!(?port, "client port number too low");
                    return Ok(());
                }
                info!(?port, "new client");
                let listener = TcpListener::bind(("::", port)).await?;
                loop {
                    if send_mp(&mut stream, ServerMessage::Heartbeat)
                        .await
                        .is_err()
                    {
                        // Assume that the TCP connection has been dropped.
                        return Ok(());
                    }
                    const TIMEOUT: Duration = Duration::from_millis(500);
                    if let Ok(result) = timeout(TIMEOUT, listener.accept()).await {
                        let (stream2, addr) = result?;
                        info!(?addr, ?port, "new connection");

                        let id = Uuid::new_v4();
                        let conns = Arc::clone(&self.conns);
                        conns.insert(id, stream2);
                        tokio::spawn(async move {
                            // Remove stale entries to avoid memory leaks.
                            sleep(Duration::from_secs(10)).await;
                            if conns.remove(&id).is_some() {
                                warn!(?id, "removed stale connection");
                            }
                        });
                        send_mp(&mut stream, ServerMessage::Connection(id)).await?;
                    }
                }
            }
            Some(ClientMessage::Accept(id)) => {
                info!(?id, "forwarding connection");
                match self.conns.remove(&id) {
                    Some((_, stream2)) => proxy(stream, stream2).await?,
                    None => warn!(?id, "missing connection ID"),
                }
                Ok(())
            }
            None => {
                warn!("unexpected EOF");
                Ok(())
            }
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Server::new(1024)
    }
}

/// Read the next null-delimited MessagePack instruction from a stream.
async fn next_mp<T: DeserializeOwned>(
    reader: &mut (impl AsyncBufRead + Unpin),
    buf: &mut Vec<u8>,
) -> Result<Option<T>> {
    buf.clear();
    reader.read_until(0, buf).await?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&0) {
        buf.pop();
    }
    Ok(rmp_serde::from_slice(buf).context("failed to parse MessagePack")?)
}

/// Send a null-terminated MessagePack instruction on a stream.
async fn send_mp<T: Serialize>(writer: &mut (impl AsyncWrite + Unpin), msg: T) -> Result<()> {
    let msg = rmp_serde::to_vec(&msg)?;
    writer.write_all(&msg).await?;
    writer.write_all(&[0]).await?;
    Ok(())
}

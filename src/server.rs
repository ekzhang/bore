//! Server implementation for the `bore` service.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth;
use crate::shared::{proxy, recv_json, send_json, ClientMessage, ServerMessage, CONTROL_PORT};

/// State structure for the server.
pub struct Server {
    /// The minimum TCP port that can be forwarded.
    min_port: u16,

    /// Optional secret data to authenticate clients.
    key: Option<auth::Key>,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16) -> Self {
        Server {
            min_port,
            conns: Arc::new(DashMap::new()),
            key: None,
        }
    }

    /// Create a new server with a specified minimum port number an a secret.
    pub fn new_with_secret(min_port: u16, secret: &str) -> Result<Self> {
        let key = auth::key_from_sec(secret)?;
        Ok(Server {
            min_port,
            conns: Arc::new(DashMap::new()),
            key: Some(key),
        })
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
        let msg = recv_json(&mut stream, &mut buf).await?;

        match msg {
            Some(ClientMessage::Hello((port, secret))) => {
                if let Err(e) = self.authenticate(secret) {
                    send_json(&mut stream, ServerMessage::ClientError(e.into())).await?;
                    return Ok(());
                }

                if port != 0 && port < self.min_port {
                    warn!(?port, "client port number too low");
                    return Ok(());
                }
                info!(?port, "new client");
                let listener = match TcpListener::bind(("::", port)).await {
                    Ok(listener) => listener,
                    Err(_) => {
                        warn!(?port, "could not bind to local port");
                        send_json(
                            &mut stream,
                            ServerMessage::Error("port already in use".into()),
                        )
                        .await?;
                        return Ok(());
                    }
                };
                let port = listener.local_addr()?.port();
                send_json(&mut stream, ServerMessage::Hello(port)).await?;

                loop {
                    if send_json(&mut stream, ServerMessage::Heartbeat)
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
                                warn!(%id, "removed stale connection");
                            }
                        });
                        send_json(&mut stream, ServerMessage::Connection(id)).await?;
                    }
                }
            }
            Some(ClientMessage::Accept(id)) => {
                info!(%id, "forwarding connection");
                match self.conns.remove(&id) {
                    Some((_, stream2)) => proxy(stream, stream2).await?,
                    None => warn!(%id, "missing connection"),
                }
                Ok(())
            }
            None => {
                warn!("unexpected EOF");
                Ok(())
            }
        }
    }

    fn authenticate(&self, secret: Option<String>) -> Result<(), &'static str> {
        match (&self.key, secret) {
            (Some(key), Some(cln_sec)) => {
                if auth::secrets_match(&key, &cln_sec).is_err() {
                    warn!("incorrect secret provided");
                    Err("incorrect secret".into())
                } else {
                    Ok(())
                }
            }
            (None, Some(_)) => {
                warn!("secret provided but not configured on server");
                Err("incorrect secret".into())
            }
            (Some(_), None) => {
                warn!("no secret provided");
                Err("secret is required".into())
            }
            (None, None) => Ok(()),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Server::new(1024)
    }
}

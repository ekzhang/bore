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
    conns: Arc<DashMap<Uuid, (TcpStream, Option<auth::ChallengeNonce>)>>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16, secret: &Option<String>) -> Self {
        let key = secret.as_ref().map(|s| auth::key_from_sec(s));
        Server {
            min_port,
            conns: Arc::new(DashMap::new()),
            key,
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
        let msg = recv_json(&mut stream, &mut buf).await?;

        match msg {
            Some(ClientMessage::Hello(port)) => {
                if let Some(key) = &self.key {
                    if let Err(e) = auth::challenge(key, &mut stream).await {
                        send_json(&mut stream, ServerMessage::Unauthenticated(format!("{e}")))
                            .await?;
                        return Ok(());
                    }
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

                        let chal_nonce = self.key.as_ref().map(|_| auth::gen_nonce());

                        conns.insert(id, (stream2, chal_nonce));
                        tokio::spawn(async move {
                            // Remove stale entries to avoid memory leaks.
                            sleep(Duration::from_secs(10)).await;
                            if conns.remove(&id).is_some() {
                                warn!(%id, "removed stale connection");
                            }
                        });

                        let chal_nonce = chal_nonce.map(base64::encode);
                        send_json(&mut stream, ServerMessage::Connection(id, chal_nonce)).await?;
                    }
                }
            }
            Some(ClientMessage::ChallengeAnswer(_)) => {
                warn!("unexpected challenge answer");
                Ok(())
            }
            Some(ClientMessage::Accept(id, resp)) => {
                info!(%id, "forwarding connection");
                match self.conns.remove(&id) {
                    Some((_, (stream2, nonce))) => {
                        match auth::is_good_accept(&self.key, nonce, &id, &resp) {
                            Ok(_) => proxy(stream, stream2).await?,
                            Err(e) => {
                                warn!("client connection challenge, MITM attempt? {e}");
                                send_json(
                                    &mut stream,
                                    ServerMessage::Unauthenticated("invalid accept".into()),
                                )
                                .await?;
                            }
                        }
                    }
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
}

impl Default for Server {
    fn default() -> Self {
        Server::new(1024, &None)
    }
}

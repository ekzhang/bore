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

use crate::auth::Authenticator;
use crate::shared::{
    proxy, recv_json_timeout, send_json, ClientMessage, ServerMessage, CONTROL_PORT,
};

/// State structure for the server.
pub struct Server {
    /// The minimum TCP port that can be forwarded.
    min_port: u16,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16, secret: Option<&str>) -> Self {
        Server {
            min_port,
            conns: Arc::new(DashMap::new()),
            auth: secret.map(Authenticator::new),
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

    /// Create new TcpListener and return it, if port is set to 0 then in sequence search for unused port in range
    async fn create_listener(port:u16,min_port:u16) -> Option<TcpListener> {
        if port == 0 {
            for port in min_port..65535 {
                match TcpListener::bind(("::", port)).await {
                    Ok(l) => return Some(l),
                    Err(_) => continue
                }
            }
            // Check the last port as loop does not include it
            match TcpListener::bind(("::", 65535)).await {
                Ok(l) => return Some(l),
                Err(_) => return None
            }
        } else {
            match TcpListener::bind(("::", port)).await {
                Ok(l) => return Some(l),
                Err(_) => {}
            }
        }
        return None;
    }

    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let mut stream = BufReader::new(stream);
        if let Some(auth) = &self.auth {
            if let Err(err) = auth.server_handshake(&mut stream).await {
                warn!(%err, "server handshake failed");
                send_json(&mut stream, ServerMessage::Error(err.to_string())).await?;
                return Ok(());
            }
        }

        match recv_json_timeout(&mut stream).await? {
            Some(ClientMessage::Authenticate(_)) => {
                warn!("unexpected authenticate");
                Ok(())
            }
            Some(ClientMessage::Hello(port)) => {
                if port != 0 && port < self.min_port {
                    warn!(?port, "client port number too low");
                    return Ok(());
                }
                info!(?port, "new client");
                let listener = match Server::create_listener(port, self.min_port).await {
                    Some(listener) => listener,
                    None => {
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
}

impl Default for Server {
    fn default() -> Self {
        Server::new(1024, None)
    }
}

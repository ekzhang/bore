//! Server implementation for the `bore` service.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use rand::Rng;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{proxy, ClientMessage, Delimited, ServerMessage, CONTROL_PORT};

/// State structure for the server.
pub struct Server {
    /// The minimum TCP port that can be forwarded.
    min_port: u16,

    /// The maximum TCP port that can be forwarded.
    max_port: u16,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16, max_port: u16, secret: Option<&str>) -> Self {
        Server {
            min_port,
            max_port,
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

    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        let mut stream = Delimited::new(stream);
        if let Some(auth) = &self.auth {
            if let Err(err) = auth.server_handshake(&mut stream).await {
                warn!(%err, "server handshake failed");
                stream.send(ServerMessage::Error(err.to_string())).await?;
                return Ok(());
            }
        }

        match stream.recv_timeout().await? {
            Some(ClientMessage::Authenticate(_)) => {
                warn!("unexpected authenticate");
                Ok(())
            }
            Some(ClientMessage::Hello(port)) => {
                if port != 0 && (port < self.min_port || port > self.max_port) {
                    warn!(?port, "client port number not in range [{}, {}]", self.min_port, self.max_port);
                    return Ok(());
                }
                info!(?port, "new client");

                let mut listener = None;
                if port == 0 {
                    let initial_state = {
                        let mut rng = rand::thread_rng();
                        rng.gen_range(self.min_port..=self.max_port)
                    };
                    
                    let mut bound = false;
                    for port in RandomPortGenerator::new(initial_state, self.min_port, self.max_port) {
                        match TcpListener::bind(("0.0.0.0", port)).await {
                            Ok(bound_listener) => {
                                listener = Some(bound_listener);
                                bound = true;
                                break
                            },
                            Err(_) => {
                                warn!(?port, "could not bind to random local port");
                            }
                        };
                    }
                    
                    if !bound {
                        let error = format!("no ports available in range [{}, {}]", self.min_port, self.max_port);
                        warn!("{}", error);
                        stream
                            .send(ServerMessage::Error(error))
                            .await?;
                        return Ok(());
                    }
                } else {
                    listener = match TcpListener::bind(("0.0.0.0", port)).await {
                        Ok(bound_listener) => Some(bound_listener),
                        Err(_) => {
                            warn!(?port, "could not bind to local port");
                            stream
                                .send(ServerMessage::Error("port already in use".into()))
                                .await?;
                            return Ok(());
                        }
                    };
                }

                // safe unwrap because we always return if listener is None
                let listener = listener.unwrap();

                let port = listener.local_addr()?.port();
                stream.send(ServerMessage::Hello(port)).await?;
                info!(?port, "client listener initialized");

                loop {
                    if stream.send(ServerMessage::Heartbeat).await.is_err() {
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
                        stream.send(ServerMessage::Connection(id)).await?;
                    }
                }
            }
            Some(ClientMessage::Accept(id)) => {
                info!(%id, "forwarding connection");
                match self.conns.remove(&id) {
                    Some((_, mut stream2)) => {
                        let parts = stream.into_parts();
                        debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
                        stream2.write_all(&parts.read_buf).await?;
                        proxy(parts.io, stream2).await?
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
        Server::new(1024, u16::MAX, None)
    }
}

struct RandomPortGenerator {
    state: u16,
    consumed: u16,
    min_port: u16,
    max_port: u16,
}

impl RandomPortGenerator {
    fn new(initial_state: u16, min_port: u16, max_port: u16) -> Self {
        Self {
            state: initial_state,
            consumed: 0,
            min_port,
            max_port
        }
    }

    fn step(&mut self) -> u16 {
        if self.done() {
            return 0
        }

        let bit  = ((self.state >> 0) ^ 
                    (self.state >> 2) ^ 
                    (self.state >> 3) ^ 
                    (self.state >> 5)) & 1;

        let state = (self.state >> 1) | (bit << 15);
        self.state =  state;
        self.consumed += 1;

        state
    }

    fn done(&mut self) -> bool {
        self.consumed == u16::MAX || self.state == 0
    }
}

impl Iterator for RandomPortGenerator {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        if self.consumed == u16::MAX {
            return None
        }

        // from the current state, sample until we get a port in the range or we're done
        let mut sample = self.state;
        while sample < self.min_port || sample > self.max_port {
            sample = self.step();

            if self.done() {
                return None
            }
        }
        // prime the next sample by taking a step
        self.step();

        Some(sample)
    }
}
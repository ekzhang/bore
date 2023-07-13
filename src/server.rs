//! Server implementation for the `bore` service.

use std::{io, net::SocketAddr, ops::RangeInclusive, sync::Arc, time::Duration};
use socket2::{Socket, Type, SockAddr};

use anyhow::Result;
use dashmap::DashMap;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{proxy, ClientMessage, Delimited, ServerMessage, CONTROL_PORT};
/// State structure for the server.
pub struct Server {
    /// Range of TCP ports that can be forwarded.
    port_range: RangeInclusive<u16>,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,

    /// Listen Addr
    listen_addr: String,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(port_range: RangeInclusive<u16>, secret: Option<&str>, listen_addr: String) -> Self {
        assert!(!port_range.is_empty(), "must provide at least one port");
        Server {
            port_range,
            conns: Arc::new(DashMap::new()),
            auth: secret.map(Authenticator::new),
            listen_addr,
        }
    }
    /// Create a TcpListener using socket2
    pub async fn tcp_listen(&self, listen_addr: &String, listen_port: u16) -> Result<TcpListener, &'static str> {
        let addr_str: String = format!("{}:{}", listen_addr, listen_port);
        let addr = addr_str.parse::<SocketAddr>();
        if let Err(_) = addr {
            return Err("failed to parse ip address");
        }
        let addr: SockAddr = addr.unwrap().into();
        // Create socket
        let socket = Socket::new(addr.domain(),Type::STREAM, None).unwrap();
        // Make socket dual-stack before binding
        if addr.is_ipv6()  {
            if Socket::only_v6(&socket).unwrap() {
            let _= Socket::set_only_v6(&socket,false);
            }
        }
        let a = socket.bind(&addr)
                    .map_err(|err| match err.kind() {
                    io::ErrorKind::AddrInUse => "port already in use",
                    io::ErrorKind::PermissionDenied => "permission denied",
                    _ => "failed to bind socket",
                });
        if let Err(i) = a {
            return Err(i);
        }
        
        let _= socket.listen(128);
        let std_listener: std::net::TcpListener = socket.into();
        let _= std_listener.set_nonblocking(true);
        let listener = TcpListener::from_std(std_listener);
        match listener {
            Ok(listener) => {
                return Ok(listener);
            },
            Err(_) => {
                return Err("tcp listener error");
            }
        };

    }
    /// Start the server, listening for new connections.
    pub async fn listen(self) -> Result<()> {
        let this: Arc<Server> = Arc::new(self);
        let listener = this.tcp_listen(&this.listen_addr,CONTROL_PORT).await;
        match listener {
            Ok(listener) => {
                info!("{} {}:{}", "server listening:", this.listen_addr, CONTROL_PORT);
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
            },
            Err(i) => {
                error!("failed to create tcp listener: {}",i);
                return Err(anyhow::anyhow!("failed to start server"));
            }
        };

    }
    async fn create_listener(&self, port: u16) -> Result<TcpListener, &'static str> {
        let try_bind = |port: u16| async move {
            self.tcp_listen(&self.listen_addr, port)
                .await
        };
        if port > 0 {
            // Client requests a specific port number.
            if !self.port_range.contains(&port) {
                return Err("client port number not in allowed range");
            }
            try_bind(port).await
        } else {
            // Client requests any available port in range.
            //
            // In this case, we bind to 150 random port numbers. We choose this value because in
            // order to find a free port with probability at least 1-δ, when ε proportion of the
            // ports are currently available, it suffices to check approximately -2 ln(δ) / ε
            // independently and uniformly chosen ports (up to a second-order term in ε).
            //
            // Checking 150 times gives us 99.999% success at utilizing 85% of ports under these
            // conditions, when ε=0.15 and δ=0.00001.
            for _ in 0..150 {
                let port = fastrand::u16(self.port_range.clone());
                match try_bind(port).await {
                    Ok(listener) => return Ok(listener),
                    Err(_) => continue,
                }
            }
            Err("failed to find an available port")
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
                let listener = match self.create_listener(port).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        stream.send(ServerMessage::Error(err.into())).await?;
                        return Ok(());
                    }
                };
                let port = listener.local_addr()?.port();
                info!(?port, "new client");
                stream.send(ServerMessage::Hello(port)).await?;

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
            None => Ok(()),
        }
    }
}

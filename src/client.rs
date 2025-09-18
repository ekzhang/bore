//! Client implementation for the `bore` service.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::{io::AsyncWriteExt, net::TcpStream, time::{sleep, timeout}};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{ClientMessage, Delimited, ServerMessage, CONTROL_PORT, NETWORK_TIMEOUT};
use crate::socket_util::create_configured_connection;

/// State structure for the client.
pub struct Client {
    /// Control connection to the server.
    conn: Option<Delimited<TcpStream>>,

    /// Destination address of the server.
    to: String,

    // Local host that is forwarded.
    local_host: String,

    /// Local port that is forwarded.
    local_port: u16,

    /// Port that is publicly available on the remote.
    remote_port: u16,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,
}

impl Client {
    /// Create a new client.
    pub async fn new(
        local_host: &str,
        local_port: u16,
        to: &str,
        port: u16,
        secret: Option<&str>,
    ) -> Result<Self> {
        let mut stream = Delimited::new(connect_with_retry(to, CONTROL_PORT, 5).await?);
        let auth = secret.map(Authenticator::new);
        if let Some(auth) = &auth {
            auth.client_handshake(&mut stream).await?;
        }

        stream.send(ClientMessage::Hello(port)).await?;
        let remote_port = match stream.recv_timeout().await? {
            Some(ServerMessage::Hello(remote_port)) => remote_port,
            Some(ServerMessage::Error(message)) => bail!("server error: {message}"),
            Some(ServerMessage::Challenge(_)) => {
                bail!("server requires authentication, but no client secret was provided");
            }
            Some(_) => bail!("unexpected initial non-hello message"),
            None => bail!("unexpected EOF"),
        };
        info!(remote_port, "connected to server");
        info!("listening at {to}:{remote_port}");

        Ok(Client {
            conn: Some(stream),
            to: to.to_string(),
            local_host: local_host.to_string(),
            local_port,
            remote_port,
            auth,
        })
    }

    /// Returns the port publicly available on the remote.
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Start the client, listening for new connections with automatic reconnection.
    pub async fn listen(self) -> Result<()> {
        let this = Arc::new(self);
        let mut reconnect_delay = Duration::from_millis(500);
        let max_delay = Duration::from_secs(30);
        
        loop {
            match this.listen_with_connection().await {
                Ok(_) => {
                    info!("connection closed normally");
                    break;
                }
                Err(err) => {
                    error!("connection lost: {}, attempting to reconnect in {:?}", err, reconnect_delay);
                    sleep(reconnect_delay).await;
                    
                    // Exponential backoff with jitter for reconnection
                    reconnect_delay = std::cmp::min(reconnect_delay * 2, max_delay);
                    reconnect_delay += Duration::from_millis(fastrand::u64(0..=reconnect_delay.as_millis() as u64 / 4));
                }
            }
        }
        Ok(())
    }
    
    /// Handle a single connection session
    async fn listen_with_connection(self: &Arc<Self>) -> Result<()> {
        // Establish connection to server
        let mut stream = Delimited::new(connect_with_retry(&self.to, CONTROL_PORT, 5).await?);
        
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut stream).await?;
        }

        // Send hello message with the same port preference
        stream.send(ClientMessage::Hello(self.remote_port)).await?;
        match stream.recv_timeout().await? {
            Some(ServerMessage::Hello(remote_port)) => {
                if remote_port != self.remote_port {
                    warn!("server assigned different port: {} (requested: {})", remote_port, self.remote_port);
                }
                info!(remote_port, "reconnected to server");
            }
            Some(ServerMessage::Error(message)) => bail!("server error: {message}"),
            Some(ServerMessage::Challenge(_)) => {
                bail!("server requires authentication, but no client secret was provided");
            }
            Some(_) => bail!("unexpected initial non-hello message"),
            None => bail!("unexpected EOF during handshake"),
        };

        // Main message loop
        loop {
            match stream.recv().await? {
                Some(ServerMessage::Hello(_)) => warn!("unexpected hello"),
                Some(ServerMessage::Challenge(_)) => warn!("unexpected challenge"),
                Some(ServerMessage::Heartbeat) => (),
                Some(ServerMessage::Connection(id)) => {
                    let this = Arc::clone(self);
                    tokio::spawn(
                        async move {
                            info!("new connection");
                            match this.handle_connection(id).await {
                                Ok(_) => info!("connection exited"),
                                Err(err) => warn!(%err, "connection exited with error"),
                            }
                        }
                        .instrument(info_span!("proxy", %id)),
                    );
                }
                Some(ServerMessage::Error(err)) => {
                    error!(%err, "server error");
                    bail!("server error: {}", err);
                }
                None => {
                    warn!("control connection closed by server");
                    bail!("control connection closed");
                }
            }
        }
    }

    async fn handle_connection(&self, id: Uuid) -> Result<()> {
        let mut remote_conn =
            Delimited::new(connect_with_retry(&self.to[..], CONTROL_PORT, 3).await?);
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut remote_conn).await?;
        }
        remote_conn.send(ClientMessage::Accept(id)).await?;
        let mut local_conn = connect_with_timeout(&self.local_host, self.local_port).await?;
        let mut parts = remote_conn.into_parts();
        debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
        local_conn.write_all(&parts.read_buf).await?; // mostly of the cases, this will be empty
        tokio::io::copy_bidirectional(&mut local_conn, &mut parts.io).await?;
        Ok(())
    }
}

async fn connect_with_timeout(to: &str, port: u16) -> Result<TcpStream> {
    let addr = format!("{}:{}", to, port).parse()?;
    let stream = match timeout(NETWORK_TIMEOUT, create_configured_connection(addr)).await {
        Ok(res) => res?,
        Err(_) => bail!("connection timeout"),
    };
    Ok(stream)
}

/// Connect with exponential backoff retry logic
async fn connect_with_retry(to: &str, port: u16, max_retries: u32) -> Result<TcpStream> {
    let mut retry_delay = Duration::from_millis(500);
    let max_delay = Duration::from_secs(30);
    
    for attempt in 0..=max_retries {
        match connect_with_timeout(to, port).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                if attempt == max_retries {
                    return Err(err).with_context(|| {
                        format!("failed to connect to {}:{} after {} attempts", to, port, max_retries + 1)
                    });
                }
                
                warn!("connection attempt {} failed: {}, retrying in {:?}", attempt + 1, err, retry_delay);
                sleep(retry_delay).await;
                
                // Exponential backoff with jitter
                retry_delay = std::cmp::min(retry_delay * 2, max_delay);
                retry_delay += Duration::from_millis(fastrand::u64(0..=retry_delay.as_millis() as u64 / 4));
            }
        }
    }
    
    unreachable!()
}

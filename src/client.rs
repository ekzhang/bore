//! Client implementation for the `bore` service.

use std::io;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use tokio::io::AsyncWriteExt;
use tokio::{net::TcpStream, time::timeout};
use tokio_rustls::{rustls, TlsConnector};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{
    proxy, ClientMessage, Delimited, ServerMessage, StreamTrait, CONTROL_PORT, NETWORK_TIMEOUT,
};

/// State structure for the client.
pub struct Client {
    /// Control connection to the server.
    conn: Option<Delimited<Box<dyn StreamTrait>>>,

    /// Config structure for the client.
    config: ClientConfig,
}

/// Config structure for the client.
struct ClientConfig {
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

    /// Optional tls configuration
    tls: Option<TlsConnector>,
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
        Client::new_with_tls(local_host, local_port, to, port, secret, None).await
    }

    /// Create a new client with tls is configurable.
    pub async fn new_with_tls(
        local_host: &str,
        local_port: u16,
        to: &str,
        port: u16,
        secret: Option<&str>,
        tls: Option<TlsConnector>,
    ) -> Result<Self> {
        let mut stream = Delimited::new(connect_with_timeout(to, CONTROL_PORT, &tls).await?);
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
            config: ClientConfig {
                to: to.to_string(),
                local_host: local_host.to_string(),
                local_port,
                remote_port,
                auth,
                tls,
            },
        })
    }

    /// Returns the port publicly available on the remote.
    pub fn remote_port(&self) -> u16 {
        self.config.remote_port
    }

    /// Start the client, listening for new connections.
    pub async fn listen(mut self) -> Result<()> {
        let mut conn = self.conn.take().unwrap();
        let config = Arc::new(self.config);
        loop {
            match conn.recv().await? {
                Some(ServerMessage::Hello(_)) => warn!("unexpected hello"),
                Some(ServerMessage::Challenge(_)) => warn!("unexpected challenge"),
                Some(ServerMessage::Heartbeat) => (),
                Some(ServerMessage::Connection(id)) => {
                    let config = Arc::clone(&config);
                    tokio::spawn(
                        async move {
                            info!("new connection");
                            match handle_connection(&config, id).await {
                                Ok(_) => info!("connection exited"),
                                Err(err) => warn!(%err, "connection exited with error"),
                            }
                        }
                        .instrument(info_span!("proxy", %id)),
                    );
                }
                Some(ServerMessage::Error(err)) => error!(%err, "server error"),
                None => return Ok(()),
            }
        }
    }
}

async fn handle_connection(config: &ClientConfig, id: Uuid) -> Result<()> {
    let mut remote_conn =
        Delimited::new(connect_with_timeout(&config.to[..], CONTROL_PORT, &config.tls).await?);
    if let Some(auth) = &config.auth {
        auth.client_handshake(&mut remote_conn).await?;
    }
    remote_conn.send(ClientMessage::Accept(id)).await?;
    let mut local_conn = connect_with_timeout(&config.local_host, config.local_port, &None).await?;
    let parts = remote_conn.into_parts();
    debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
    local_conn.write_all(&parts.read_buf).await?; // mostly of the cases, this will be empty
    proxy(local_conn, parts.io).await?;
    Ok(())
}

async fn connect_with_timeout(
    to: &str,
    port: u16,
    tls: &Option<TlsConnector>,
) -> Result<Box<dyn StreamTrait>> {
    let stream = match timeout(NETWORK_TIMEOUT, TcpStream::connect((to, port))).await {
        Ok(res) => res,
        Err(err) => Err(err.into()),
    }
    .with_context(|| format!("could not connect to {to}:{port}"))?;
    match tls {
        Some(connector) => {
            let domain = rustls::ServerName::try_from(to)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid dnsname"))?;

            let stream = connector.connect(domain, stream).await?;
            Ok(Box::new(stream))
        }
        None => Ok(Box::new(stream)),
    }
}

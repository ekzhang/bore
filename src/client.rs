//! Client implementation for the `bore` service.

use std::sync::Arc;

use crate::auth;
use anyhow::{bail, Context, Result};
use tokio::{io::BufReader, net::TcpStream};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::shared::{proxy, recv_json, send_json, ClientMessage, ServerMessage, CONTROL_PORT};

/// State structure for the client.
pub struct Client {
    /// Control connection to the server.
    conn: Option<BufReader<TcpStream>>,

    /// Destination address of the server.
    to: String,

    /// Local port that is forwarded.
    local_port: u16,

    /// Port that is publicly available on the remote.
    remote_port: u16,
}

impl Client {
    /// Create a new client.
    pub async fn new(local_port: u16, to: &str, port: u16, secret: Option<String>) -> Result<Self> {
        let stream = TcpStream::connect((to, CONTROL_PORT)).await?;
        let mut stream = BufReader::new(stream);

        let secret = match secret {
            Some(s) => match auth::encrypt_encode_secret(&s) {
                Ok(s) => Some(s),
                Err(e) => bail!("{e}"),
            },
            None => None,
        };

        send_json(&mut stream, ClientMessage::Hello((port, secret))).await?;
        let remote_port = match recv_json(&mut stream, &mut Vec::new()).await? {
            Some(ServerMessage::Hello(remote_port)) => remote_port,
            Some(ServerMessage::Error(message)) => bail!("server error: {message}"),
            Some(ServerMessage::ClientError(message)) => bail!("client error: {message}"),
            Some(_) => bail!("unexpected initial non-hello message"),
            None => bail!("unexpected EOF"),
        };
        info!(remote_port, "connected to server");
        info!("listening at {to}:{remote_port}");

        Ok(Client {
            conn: Some(stream),
            to: to.to_string(),
            local_port,
            remote_port,
        })
    }

    /// Returns the port publicly available on the remote.
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Start the client, listening for new connections.
    pub async fn listen(mut self) -> Result<()> {
        let mut conn = self.conn.take().unwrap();
        let this = Arc::new(self);
        let mut buf = Vec::new();
        loop {
            let msg = recv_json(&mut conn, &mut buf).await?;
            match msg {
                Some(ServerMessage::Hello(_)) => warn!("unexpected hello"),
                Some(ServerMessage::Heartbeat) => (),
                Some(ServerMessage::Connection(id)) => {
                    let this = Arc::clone(&this);
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
                Some(ServerMessage::Error(err)) => error!(%err, "server error"),
                Some(ServerMessage::ClientError(err)) => error!(%err, "client error"),
                None => return Ok(()),
            }
        }
    }

    async fn handle_connection(&self, id: Uuid) -> Result<()> {
        let local_conn = TcpStream::connect(("localhost", self.local_port))
            .await
            .context("failed TCP connection to local port")?;
        let mut remote_conn = TcpStream::connect((&self.to[..], CONTROL_PORT))
            .await
            .context("failed TCP connection to remote port")?;

        send_json(&mut remote_conn, ClientMessage::Accept(id)).await?;
        proxy(local_conn, remote_conn).await?;
        Ok(())
    }
}

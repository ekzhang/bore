//! Client implementation for the `bore` service.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use futures::SinkExt;

use tokio::{net::TcpStream, time::timeout};
use tokio_util::codec::{AnyDelimiterCodec, Framed};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{
    proxy, recv_json, recv_json_timeout, send_json, ClientMessage, ServerMessage, CONTROL_PORT,
    NETWORK_TIMEOUT,
};

/// State structure for the client.
pub struct Client {
    /// Control connection to the server.
    conn: Option<Framed<TcpStream, AnyDelimiterCodec>>,

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
        let mut stream = Framed::new(
            connect_with_timeout(to, CONTROL_PORT).await?,
            AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
        );
        let auth = secret.map(Authenticator::new);
        if let Some(auth) = &auth {
            auth.client_handshake(&mut stream).await?;
        }

        send_json(&mut stream, ClientMessage::Hello(port)).await?;
        let remote_port = match recv_json_timeout(&mut stream).await? {
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

    /// Start the client, listening for new connections.
    pub async fn listen(mut self) -> Result<()> {
        let mut conn = self.conn.take().unwrap();
        let this = Arc::new(self);
        while let Ok(msg) = recv_json(&mut conn).await {
            match msg {
                Some(ServerMessage::Hello(_)) => warn!("unexpected hello"),
                Some(ServerMessage::Challenge(_)) => warn!("unexpected challenge"),
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
                None => return Ok(()),
            }
        }
        Ok(())
    }

    async fn handle_connection(&self, id: Uuid) -> Result<()> {
        let mut remote_con = Framed::new(
            connect_with_timeout(&self.to[..], CONTROL_PORT).await?,
            AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
        );
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut remote_con).await?;
        }
        remote_con
            .send(&serde_json::to_string(&ClientMessage::Accept(id)).unwrap())
            .await?;
        let local_conn = connect_with_timeout(&self.local_host, self.local_port).await?;
        proxy(local_conn, remote_con.get_mut()).await?;
        Ok(())
    }
}

async fn connect_with_timeout(to: &str, port: u16) -> Result<TcpStream> {
    match timeout(NETWORK_TIMEOUT, TcpStream::connect((to, port))).await {
        Ok(res) => res,
        Err(err) => Err(err.into()),
    }
    .with_context(|| format!("could not connect to {to}:{port}"))
}

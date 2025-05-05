//! Client implementation for the `bore` service.

use std::io::Read;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tokio::{io::AsyncWriteExt, net::TcpStream, time::timeout};
use tracing::{error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::shared::{
    proxy, ClientMessage, Delimited, ServerMessage, CONTROL_PORT, NETWORK_TIMEOUT,
    BORE_KEEPINTERVAL, tcp_keepalive, parse_envvar_u64,
};

/// State structure for the client.
pub struct Client {
    /// Control connection to the server.
    conn: Option<Delimited<TcpStream>>,

    /// Destination address of the server.
    to: String,

    /// Local host that is forwarded.
    local_host: String,

    /// Local host identity string.
    host_id: String,

    /// Local port that is forwarded.
    local_port: u16,

    /// Port that is publicly available on the remote.
    remote_port: u16,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,
}

fn random_hostid(idlen: usize) -> String {
    if idlen <= 4 {
        if idlen == 0 { "".to_string() } else { std::iter::repeat_with(fastrand::alphanumeric).take(idlen).collect() }
    } else {
        let mut newid = String::with_capacity(idlen + 1);
        let idlen = idlen - 4;
        newid.push_str("TMP_");
        let rid: String = std::iter::repeat_with(fastrand::alphanumeric).take(idlen).collect();
        newid.push_str(&rid);
        newid
    }
}

fn read_hostid_fromfile(idlen: usize) -> String {
    // Get the text file containing `BORE_HOSTID
    let idfile: std::ffi::OsString = match std::env::var_os("BORE_IDFILE") {
        Some(fpath) => fpath,
        None => std::ffi::OsString::from("/tmp/bore_hostid.txt"),
    };

    let hfile = std::fs::OpenOptions::new().read(true)
        .write(false).create(false).open(&idfile);
    if hfile.is_err() {
        return random_hostid(idlen);
    }

    let mut hfile = hfile.unwrap();
    let mut idbuf = vec![0u8; idlen];
    let rlen = hfile.read(&mut idbuf[..]).unwrap_or(0);
    if rlen == 0 {
        return random_hostid(idlen);
    }

    let idstr = String::from_utf8_lossy(&idbuf[..rlen]);
    let hostid: &str = idstr.trim();
    if hostid.is_empty() { random_hostid(idlen) } else { hostid.to_string() }
}

impl Client {
    /// Create a new client.
    pub async fn new(
        local_host: &str,
        local_port: u16,
        id_str: &str,
        to: &str,
        port: u16,
        secret: Option<&str>,
    ) -> Result<Self> {
        let kval = parse_envvar_u64(BORE_KEEPINTERVAL, 120);
        let mut stream = Delimited::new(connect_with_timeout(to, CONTROL_PORT, kval).await?);
        let auth = secret.map(Authenticator::new);
        if let Some(auth) = &auth {
            auth.client_handshake(&mut stream).await?;
        }

        // Determine host ID for remote bore server
        let hostid = if id_str.is_empty() { read_hostid_fromfile(16) } else { id_str.to_string() };
        info!(hostid, "Using client IDString");

        stream.send(ClientMessage::Hello(port, hostid.clone())).await?;
        let remote_port = match stream.recv_timeout().await? {
            Some(ServerMessage::Hello(remote_port, _)) => remote_port,
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
            host_id: hostid,
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
        loop {
            match conn.recv().await? {
                Some(ServerMessage::Hello(_, _)) => warn!("unexpected hello"),
                Some(ServerMessage::Challenge(_)) => warn!("unexpected challenge"),
                Some(ServerMessage::Heartbeat) => (),
                Some(ServerMessage::Connection(id)) => {
                    let this = Arc::clone(&this);
                    let hostid: String = this.host_id.clone();
                    tokio::spawn(
                        async move {
                            info!(hostid, "new connection");
                            match this.handle_connection(id).await {
                                Ok(_) => info!(hostid, "connection exited"),
                                Err(err) => warn!(hostid, %err, "connection exited with error"),
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

    async fn handle_connection(&self, id: Uuid) -> Result<()> {
        let kval = parse_envvar_u64("BORE_KEEPINTERVAL", 120);
        let mut remote_conn =
            Delimited::new(connect_with_timeout(&self.to[..], CONTROL_PORT, kval).await?);
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut remote_conn).await?;
        }
        remote_conn.send(ClientMessage::Accept(id)).await?;
        let mut local_conn = connect_with_timeout(&self.local_host, self.local_port, kval).await?;
        let parts = remote_conn.into_parts();
        debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
        local_conn.write_all(&parts.read_buf).await?; // mostly of the cases, this will be empty
        proxy(local_conn, parts.io).await?;
        Ok(())
    }
}

async fn connect_with_timeout(to: &str, port: u16, keepival: u64) -> Result<TcpStream> {
    match timeout(NETWORK_TIMEOUT, TcpStream::connect((to, port))).await {
        Ok(res) => if res.is_ok() { Ok(tcp_keepalive(res.unwrap(), 3, keepival)) } else { res },
        Err(err) => Err(err.into()),
    }
    .with_context(|| format!("could not connect to {to}:{port}"))
}

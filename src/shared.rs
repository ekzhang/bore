//! Shared data structures, utilities, and protocol definitions.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio::time::timeout;
use tokio_util::codec::{AnyDelimiterCodec, Framed, FramedParts};
use tracing::{trace, warn};
use uuid::Uuid;

/// TCP port used for control connections with the server.
pub const CONTROL_PORT: u16 = 7835;

/// Maximum byte length for a JSON frame in the stream.
pub const MAX_FRAME_LENGTH: usize = 256;

/// Timeout for network connections and initial protocol messages.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(3);

/// Environment variable name for TCP keep-alive settings
pub const BORE_KEEPINTERVAL: &str = "BORE_KEEPINTERVAL";

/// Get and parse an environment variable as u64
pub fn parse_envvar_u64(name: &str, dftval: u64) -> u64 {
    match std::env::var(name) {
        Ok(val) => {
            if let Ok(enval) = str::parse::<u64>(&val) { enval } else { dftval }
        },
        Err(_) => dftval,
    }
}

/// Change default TCP KEEPALIVE settings for a given TcpStream
pub fn tcp_keepalive(tcps: tokio::net::TcpStream,
    retries: u32, keep_ival: u64) -> tokio::net::TcpStream {
    // Reference it as an socket2 object
    let tcpr = socket2::SockRef::from(&tcps);

    // enable or disable TCP keepalive
    let errn = if retries == 0 || keep_ival == 0 {
        tcpr.set_keepalive(false)
    } else {
        let kaopt = socket2::TcpKeepalive::new()
            .with_retries(retries)
            .with_time(std::time::Duration::from_secs(keep_ival))
            .with_interval(std::time::Duration::from_secs(keep_ival));
        let _ = tcpr.set_keepalive(true);
        tcpr.set_tcp_keepalive(&kaopt)
    };

    if let Err(err) = errn {
        warn!(%err, "failed to enable/disable TCP keepalive.");
    }
    let _ = tcpr.set_nonblocking(true);
    tcps
}

/// A message from the client on the control connection.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Response to an authentication challenge from the server.
    Authenticate(String),

    /// Initial client message specifying a port to forward.
    Hello(u16, String),

    /// Accepts an incoming TCP connection, using this stream as a proxy.
    Accept(Uuid),
}

/// A message from the server on the control connection.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Authentication challenge, sent as the first message, if enabled.
    Challenge(Uuid),

    /// Response to a client's initial message, with actual public port.
    Hello(u16, String),

    /// No-op used to test if the client is still reachable.
    Heartbeat,

    /// Asks the client to accept a forwarded TCP connection.
    Connection(Uuid),

    /// Indicates a server error that terminates the connection.
    Error(String),
}

/// Transport stream with JSON frames delimited by null characters.
pub struct Delimited<U>(Framed<U, AnyDelimiterCodec>);

impl<U: AsyncRead + AsyncWrite + Unpin> Delimited<U> {
    /// Construct a new delimited stream.
    pub fn new(stream: U) -> Self {
        let codec = AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], MAX_FRAME_LENGTH);
        Self(Framed::new(stream, codec))
    }

    /// Read the next null-delimited JSON instruction from a stream.
    pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        trace!("waiting to receive json message");
        if let Some(next_message) = self.0.next().await {
            let byte_message = next_message.context("frame error, invalid byte length")?;
            let serialized_obj =
                serde_json::from_slice(&byte_message).context("unable to parse message")?;
            Ok(serialized_obj)
        } else {
            Ok(None)
        }
    }

    /// Read the next null-delimited JSON instruction, with a default timeout.
    ///
    /// This is useful for parsing the initial message of a stream for handshake or
    /// other protocol purposes, where we do not want to wait indefinitely.
    pub async fn recv_timeout<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        timeout(NETWORK_TIMEOUT, self.recv())
            .await
            .context("timed out waiting for initial message")?
    }

    /// Send a null-terminated JSON instruction on a stream.
    pub async fn send<T: Serialize>(&mut self, msg: T) -> Result<()> {
        trace!("sending json message");
        self.0.send(serde_json::to_string(&msg)?).await?;
        Ok(())
    }

    /// Consume this object, returning current buffers and the inner transport.
    pub fn into_parts(self) -> FramedParts<U, AnyDelimiterCodec> {
        self.0.into_parts()
    }
}

/// Copy data mutually between two read/write streams.
pub async fn proxy<S1, S2>(stream1: S1, stream2: S2) -> io::Result<()>
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let (mut s1_read, mut s1_write) = io::split(stream1);
    let (mut s2_read, mut s2_write) = io::split(stream2);
    tokio::select! {
        res = io::copy(&mut s1_read, &mut s2_write) => res,
        res = io::copy(&mut s2_read, &mut s1_write) => res,
    }?;
    Ok(())
}

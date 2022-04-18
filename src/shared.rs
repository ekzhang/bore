//! Shared data structures, utilities, and protocol definitions.

use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{self, AsyncRead, AsyncWrite};

use tokio::time::timeout;
use tokio_util::codec::{AnyDelimiterCodec, Framed};
use tracing::trace;
use uuid::Uuid;

/// TCP port used for control connections with the server.
pub const CONTROL_PORT: u16 = 7835;

/// TCP max message length server.
pub const MAX_FRAME_LENGTH: usize = 200;

/// Timeout for network connections and initial protocol messages.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(3);

/// A message from the client on the control connection.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Response to an authentication challenge from the server.
    Authenticate(String),

    /// Initial client message specifying a port to forward.
    Hello(u16),

    /// Accepts an incoming TCP connection, using this stream as a proxy.
    Accept(Uuid),
}

/// A message from the server on the control connection.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Authentication challenge, sent as the first message, if enabled.
    Challenge(Uuid),

    /// Response to a client's initial message, with actual public port.
    Hello(u16),

    /// No-op used to test if the client is still reachable.
    Heartbeat,

    /// Asks the client to accept a forwarded TCP connection.
    Connection(Uuid),

    /// Indicates a server error that terminates the connection.
    Error(String),
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

/// Read the next null-delimited JSON instruction from a stream.
pub async fn recv_json<T: DeserializeOwned, U: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut Framed<U, AnyDelimiterCodec>,
) -> Result<Option<T>> {
    trace!("waiting to receive json message");
    if let Some(frame) = reader.next().await {
        Ok(frame
            .map(|op| {
                serde_json::from_slice(&op.to_vec())
                    .context("unable to parse message")
                    .unwrap()
            })
            .context("frame error, invalid chunck length")?)
    } else {
        Ok(None)
    }
}

/// Read the next null-delimited JSON instruction, with a default timeout.
///
/// This is useful for parsing the initial message of a stream for handshake or
/// other protocol purposes, where we do not want to wait indefinitely.
pub async fn recv_json_timeout<T: DeserializeOwned, U: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut Framed<U, AnyDelimiterCodec>,
) -> Result<Option<T>> {
    timeout(NETWORK_TIMEOUT, recv_json(reader))
        .await
        .context("timed out waiting for initial message")?
}

/// Send a null-terminated JSON instruction on a stream.
pub async fn send_json<T: Serialize, U: AsyncRead + AsyncWrite + Unpin>(
    writer: &mut Framed<U, AnyDelimiterCodec>,
    msg: T,
) -> Result<()> {
    trace!("sending json message");
    writer.send(serde_json::to_string(&msg)?).await?;
    Ok(())
}

///
pub fn get_framed_stream<T: AsyncRead + AsyncWrite + Unpin>(
    stream: T,
) -> Framed<T, AnyDelimiterCodec> {
    Framed::new(
        stream,
        AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], MAX_FRAME_LENGTH),
    )
}

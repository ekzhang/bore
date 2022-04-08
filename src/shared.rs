//! Shared data structures, utilities, and protocol definitions.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{self, AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

/// TCP port used for control connections with the server.
pub const CONTROL_PORT: u16 = 7835;

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
    tokio::try_join!(
        io::copy(&mut s1_read, &mut s2_write),
        io::copy(&mut s2_read, &mut s1_write),
    )?;
    Ok(())
}

/// Read the next null-delimited JSON instruction from a stream.
pub async fn recv_json<T: DeserializeOwned>(
    reader: &mut (impl AsyncBufRead + Unpin),
    buf: &mut Vec<u8>,
) -> Result<Option<T>> {
    buf.clear();
    reader.read_until(0, buf).await?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&0) {
        buf.pop();
    }
    Ok(serde_json::from_slice(buf).context("failed to parse JSON")?)
}

/// Send a null-terminated JSON instruction on a stream.
pub async fn send_json<T: Serialize>(writer: &mut (impl AsyncWrite + Unpin), msg: T) -> Result<()> {
    let msg = serde_json::to_vec(&msg)?;
    writer.write_all(&msg).await?;
    writer.write_all(&[0]).await?;
    Ok(())
}

//! Shared data structures, utilities, and protocol definitions.

use serde::{Deserialize, Serialize};
use tokio::io::{self, AsyncRead, AsyncWrite};
use uuid::Uuid;

/// TCP port used for control connections with the server.
pub const CONTROL_PORT: u16 = 7835;

/// A message from the client on the control connection.
#[derive(Serialize, Deserialize)]
pub enum ClientMessage {
    /// Initial client message specifying a port to forward.
    Hello(u16),

    /// Accepts an incoming TCP connection, using this stream as a proxy.
    Accept(Uuid),
}

/// A message from the server on the control connection.
#[derive(Serialize, Deserialize)]
pub enum ServerMessage {
    /// No-op used to test if the client is still reachable.
    Heartbeat,

    /// Asks the client to accept a forwarded TCP connection.
    Connection(Uuid),
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

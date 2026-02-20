//! Shared data structures, utilities, and protocol definitions.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::{AnyDelimiterCodec, Framed, FramedParts};
use tracing::trace;
use uuid::Uuid;

/// TCP port used for control connections with the server.
pub const CONTROL_PORT: u16 = 7835;

/// Maximum byte length for a JSON frame in the stream.
pub const MAX_FRAME_LENGTH: usize = 256;

/// Timeout for network connections and initial protocol messages.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(3);

/// Timeout for detecting a dead control connection.
///
/// The server sends heartbeats every 500ms. If no message is received within
/// this duration, the connection is considered dead.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(8);

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

    /// Get a reference to the underlying transport stream.
    pub fn get_ref(&self) -> &U {
        self.0.get_ref()
    }

    /// Consume this object, returning current buffers and the inner transport.
    pub fn into_parts(self) -> FramedParts<U, AnyDelimiterCodec> {
        self.0.into_parts()
    }
}

/// Simple exponential backoff with jitter for reconnection delays.
pub struct ExponentialBackoff {
    current: Duration,
    base: Duration,
    max: Duration,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff starting at `base` delay, capped at `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            current: base,
            base,
            max,
        }
    }

    /// Get the next delay and advance the backoff state.
    /// Includes random jitter of +/- 25% to prevent thundering herd.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        // Add jitter: multiply by random factor between 0.75 and 1.25
        let jitter_factor = 0.75 + fastrand::f64() * 0.5;
        delay.mul_f64(jitter_factor)
    }

    /// Reset backoff to initial delay (call after successful connection).
    pub fn reset(&mut self) {
        self.current = self.base;
    }
}

/// Configure TCP keepalive on a stream for faster dead connection detection.
///
/// This sets the OS to start probing after 30s of idle, probe every 10s,
/// and give up after 3 failed probes (~60s total to detect a dead connection).
pub fn set_tcp_keepalive(stream: &TcpStream) -> Result<()> {
    let sock_ref = SockRef::from(stream);
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10))
        .with_retries(3);
    sock_ref
        .set_tcp_keepalive(&keepalive)
        .context("failed to set TCP keepalive")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_sequence() {
        let mut backoff = ExponentialBackoff::new(Duration::from_secs(1), Duration::from_secs(30));
        // Delays should roughly double: 1, 2, 4, 8, 16, 30 (capped), 30, ...
        // With jitter, each delay is between 0.75x and 1.25x the base
        for expected_base in [1, 2, 4, 8, 16, 30, 30] {
            let delay = backoff.next_delay();
            let min = Duration::from_secs(expected_base).mul_f64(0.75);
            let max = Duration::from_secs(expected_base).mul_f64(1.25);
            assert!(
                delay >= min && delay <= max,
                "delay {delay:?} out of range [{min:?}, {max:?}]"
            );
        }
    }

    #[test]
    fn test_backoff_reset() {
        let mut backoff = ExponentialBackoff::new(Duration::from_secs(1), Duration::from_secs(60));
        backoff.next_delay(); // 1s
        backoff.next_delay(); // 2s
        backoff.next_delay(); // 4s
        backoff.reset();
        let delay = backoff.next_delay();
        // After reset, should be back to ~1s (with jitter)
        assert!(delay < Duration::from_secs(2));
    }
}

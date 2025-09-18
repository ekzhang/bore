//! Enhanced client implementation with adaptive backoff, health monitoring, and configurable behavior.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::{io::AsyncWriteExt, net::TcpStream, time::{sleep, timeout}};
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::backoff::{AdaptiveBackoff, categorize_error};
use crate::config::ClientConfig;
use crate::health::get_health_monitor;
use crate::shared::{ClientMessage, Delimited, ServerMessage, CONTROL_PORT};
use crate::socket_util::create_configured_connection_with_config;

/// Enhanced client with adaptive reconnection and health monitoring
pub struct EnhancedClient {
    /// Destination address of the server
    to: String,
    /// Local host that is forwarded
    local_host: String,
    /// Local port that is forwarded
    local_port: u16,
    /// Port that is publicly available on the remote
    remote_port: u16,
    /// Optional secret used to authenticate clients
    auth: Option<Authenticator>,
    /// Client configuration
    config: ClientConfig,
    /// Adaptive backoff strategy
    backoff: AdaptiveBackoff,
}

impl EnhancedClient {
    /// Create a new enhanced client
    pub async fn new(
        local_host: &str,
        local_port: u16,
        to: &str,
        port: u16,
        secret: Option<&str>,
        config: ClientConfig,
    ) -> Result<Self> {
        let auth = secret.map(Authenticator::new);
        let backoff = AdaptiveBackoff::new(
            config.initial_retry_delay(),
            config.max_retry_delay(),
        );
        
        // Initial connection to determine remote port
        let mut client = Self {
            to: to.to_string(),
            local_host: local_host.to_string(),
            local_port,
            remote_port: port,
            auth,
            config,
            backoff,
        };
        
        // Establish initial connection to get assigned port
        let remote_port = client.establish_initial_connection().await?;
        client.remote_port = remote_port;
        
        info!(remote_port, "Enhanced client created");
        Ok(client)
    }
    
    /// Get the remote port
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }
    
    /// Start the enhanced client with full reconnection and health monitoring
    pub async fn listen(self) -> Result<()> {
        let this = Arc::new(self);
        
        // Initialize health monitoring
        let health_monitor = get_health_monitor();
        if this.config.health_check_interval > 0 {
            health_monitor.clone().start_health_reporting(this.config.health_check_duration());
        }
        
        // Main reconnection loop
        loop {
            match this.listen_with_connection().await {
                Ok(_) => {
                    info!("Connection closed normally");
                    break;
                }
                Err(err) => {
                    let failure_type = categorize_error(&err);
                    error!("Connection error: {} (type: {:?})", err, failure_type);
                    
                    health_monitor.record_failure();
                    health_monitor.record_connection_lost();
                    
                    if !this.config.enable_reconnection {
                        return Err(err);
                    }
                    
                    // Check if we should give up
                    if this.backoff.should_circuit_break(this.config.max_reconnect_attempts) {
                        error!("Circuit breaker activated, giving up after {} attempts", 
                               this.backoff.attempt_count());
                        return Err(err);
                    }
                    
                    // Calculate adaptive delay
                    let mut backoff_clone = this.backoff.clone();
                    let delay = backoff_clone.record_failure(failure_type);
                    
                    info!("Reconnecting in {:?} (attempt {})", delay, backoff_clone.attempt_count());
                    sleep(delay).await;
                }
            }
        }
        
        Ok(())
    }
    
    /// Establish initial connection to determine remote port
    async fn establish_initial_connection(&mut self) -> Result<u16> {
        let health_monitor = get_health_monitor();
        
        for attempt in 1..=self.config.max_reconnect_attempts {
            health_monitor.record_attempt();
            let start_time = Instant::now();
            
            match self.try_initial_connection().await {
                Ok(port) => {
                    let latency = start_time.elapsed();
                    health_monitor.record_success(latency);
                    self.backoff.record_success();
                    return Ok(port);
                }
                Err(err) => {
                    health_monitor.record_failure();
                    
                    if attempt == self.config.max_reconnect_attempts {
                        return Err(err);
                    }
                    
                    let failure_type = categorize_error(&err);
                    let delay = self.backoff.record_failure(failure_type);
                    
                    warn!("Initial connection attempt {} failed: {}, retrying in {:?}", 
                          attempt, err, delay);
                    sleep(delay).await;
                }
            }
        }
        
        bail!("Failed to establish initial connection")
    }
    
    /// Try to establish initial connection
    async fn try_initial_connection(&self) -> Result<u16> {
        let mut stream = Delimited::new(
            self.connect_with_timeout_and_config(&self.to, CONTROL_PORT).await?
        );
        
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut stream).await?;
        }

        stream.send(ClientMessage::Hello(self.remote_port)).await?;
        match timeout(self.config.network_timeout(), stream.recv()).await? {
            Ok(Some(ServerMessage::Hello(remote_port))) => Ok(remote_port),
            Ok(Some(ServerMessage::Error(message))) => bail!("server error: {message}"),
            Ok(Some(ServerMessage::Challenge(_))) => {
                bail!("server requires authentication, but no client secret was provided");
            }
            Ok(Some(_)) => bail!("unexpected initial non-hello message"),
            Ok(None) => bail!("unexpected EOF"),
            Err(_) => bail!("timeout during handshake"),
        }
    }
    
    /// Handle a single connection session with health monitoring
    async fn listen_with_connection(self: &Arc<Self>) -> Result<()> {
        let health_monitor = get_health_monitor();
        let start_time = Instant::now();
        
        // Establish connection
        health_monitor.record_attempt();
        let mut stream = Delimited::new(
            self.connect_with_timeout_and_config(&self.to, CONTROL_PORT).await?
        );
        
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut stream).await?;
        }

        // Handshake
        stream.send(ClientMessage::Hello(self.remote_port)).await?;
        match timeout(self.config.network_timeout(), stream.recv()).await? {
            Ok(Some(ServerMessage::Hello(remote_port))) => {
                if remote_port != self.remote_port {
                    warn!("Server assigned different port: {} (requested: {})", 
                          remote_port, self.remote_port);
                }
                
                let latency = start_time.elapsed();
                health_monitor.record_success(latency);
                health_monitor.record_reconnection(latency);
                info!(remote_port, "Connected to server");
            }
            Ok(Some(ServerMessage::Error(message))) => bail!("server error: {message}"),
            Ok(Some(ServerMessage::Challenge(_))) => {
                bail!("server requires authentication, but no client secret was provided");
            }
            Ok(Some(_)) => bail!("unexpected initial non-hello message"),
            Ok(None) => bail!("unexpected EOF during handshake"),
            Err(_) => bail!("timeout during handshake"),
        }

        // Main message loop with health monitoring
        let mut last_heartbeat = Instant::now();
        let heartbeat_timeout = Duration::from_secs(60); // 60 seconds without heartbeat = problem
        
        loop {
            // Check for heartbeat timeout
            if last_heartbeat.elapsed() > heartbeat_timeout {
                warn!("No heartbeat received for {:?}, connection may be stale", 
                      last_heartbeat.elapsed());
                bail!("heartbeat timeout");
            }
            
            match timeout(Duration::from_secs(5), stream.recv()).await {
                Ok(Ok(Some(ServerMessage::Hello(_)))) => warn!("unexpected hello"),
                Ok(Ok(Some(ServerMessage::Challenge(_)))) => warn!("unexpected challenge"),
                Ok(Ok(Some(ServerMessage::Heartbeat))) => {
                    debug!("Received heartbeat");
                    last_heartbeat = Instant::now();
                }
                Ok(Ok(Some(ServerMessage::Connection(id)))) => {
                    let this = Arc::clone(self);
                    tokio::spawn(
                        async move {
                            debug!("New connection: {}", id);
                            match this.handle_connection(id, None).await {
                                Ok(bytes) => {
                                    info!("Connection {} completed, {} bytes transferred", id, bytes);
                                    get_health_monitor().add_bytes_transferred(bytes);
                                }
                                Err(err) => warn!("Connection {} failed: {}", id, err),
                            }
                        }
                        .instrument(info_span!("proxy", %id)),
                    );
                }
                Ok(Ok(Some(ServerMessage::ConnectionWithAddr { id, client_addr, server_addr }))) => {
                    let this = Arc::clone(self);
                    tokio::spawn(
                        async move {
                            debug!("New connection with address info: {} ({} -> {})", id, client_addr, server_addr);
                            match this.handle_connection(id, Some((client_addr, server_addr))).await {
                                Ok(bytes) => {
                                    info!("Connection {} completed, {} bytes transferred", id, bytes);
                                    get_health_monitor().add_bytes_transferred(bytes);
                                }
                                Err(err) => warn!("Connection {} failed: {}", id, err),
                            }
                        }
                        .instrument(info_span!("proxy", %id)),
                    );
                }
                Ok(Ok(Some(ServerMessage::Error(err)))) => {
                    error!("Server error: {}", err);
                    bail!("server error: {}", err);
                }
                Ok(Ok(None)) => {
                    warn!("Control connection closed by server");
                    bail!("control connection closed");
                }
                Ok(Err(e)) => {
                    error!("Stream error: {}", e);
                    bail!("stream error: {}", e);
                }
                Err(_) => {
                    // Timeout - continue loop to check heartbeat timeout
                    continue;
                }
            }
        }
    }
    
    /// Handle individual tunnel connection with byte counting
    async fn handle_connection(&self, id: Uuid, addr_info: Option<(std::net::SocketAddr, std::net::SocketAddr)>) -> Result<u64> {
        let mut remote_conn = Delimited::new(
            self.connect_with_timeout_and_config(&self.to, CONTROL_PORT).await?
        );
        
        if let Some(auth) = &self.auth {
            auth.client_handshake(&mut remote_conn).await?;
        }
        
        remote_conn.send(ClientMessage::Accept(id)).await?;
        let mut local_conn = self.connect_with_timeout_and_config(&self.local_host, self.local_port).await?;
        
        let mut parts = remote_conn.into_parts();
        debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
        
        // Inject PROXY protocol header if address info is provided and config enables it
        if let Some((client_addr, server_addr)) = addr_info {
            if self.config.enable_proxy_protocol {
                use crate::proxy_protocol::create_proxy_header;
                let proxy_header = create_proxy_header(client_addr, server_addr);
                local_conn.write_all(&proxy_header).await?;
                info!("Injected PROXY header: {} -> {}", client_addr, server_addr);
            }
        }
        
        local_conn.write_all(&parts.read_buf).await?;
        
        // Copy data bidirectionally and count bytes
        let (bytes_to_remote, bytes_to_local) = tokio::io::copy_bidirectional(&mut local_conn, &mut parts.io).await?;
        let total_bytes = bytes_to_remote + bytes_to_local;
        
        Ok(total_bytes)
    }
    
    /// Connect with timeout and configuration
    async fn connect_with_timeout_and_config(&self, host: &str, port: u16) -> Result<TcpStream> {
        let addr = format!("{}:{}", host, port).parse()?;
        let stream = timeout(
            self.config.network_timeout(),
            create_configured_connection_with_config(addr, &self.config)
        ).await
        .context("connection timeout")?
        .context("failed to create connection")?;
        
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_client_config_creation() {
        let config = ClientConfig::default();
        assert_eq!(config.keepalive_interval, 30);
        assert_eq!(config.max_reconnect_attempts, 5);
        
        let mobile_config = ClientConfig::mobile_optimized();
        assert_eq!(mobile_config.keepalive_interval, 120);
        assert!(mobile_config.initial_retry_delay_ms > config.initial_retry_delay_ms);
    }
}

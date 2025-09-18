//! Configuration structures for bore client and server.

use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Client configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// TCP keep-alive interval in seconds (default: 30)
    pub keepalive_interval: u64,
    
    /// TCP keep-alive retry count (default: 3)
    pub keepalive_retries: u32,
    
    /// Maximum number of reconnection attempts (default: 5)
    pub max_reconnect_attempts: u32,
    
    /// Initial retry delay in milliseconds (default: 500)
    pub initial_retry_delay_ms: u64,
    
    /// Maximum retry delay in seconds (default: 30)
    pub max_retry_delay_secs: u64,
    
    /// Enable aggressive reconnection (default: true)
    pub enable_reconnection: bool,
    
    /// Enable TCP keep-alive (default: true)
    pub enable_keepalive: bool,
    
    /// Enable socket reuse options (default: true)
    pub enable_socket_reuse: bool,
    
    /// Connection health check interval in seconds (default: 60)
    pub health_check_interval: u64,
    
    /// Network timeout in seconds (default: 3)
    pub network_timeout_secs: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            keepalive_interval: 30,
            keepalive_retries: 3,
            max_reconnect_attempts: 5,
            initial_retry_delay_ms: 500,
            max_retry_delay_secs: 30,
            enable_reconnection: true,
            enable_keepalive: true,
            enable_socket_reuse: true,
            health_check_interval: 60,
            network_timeout_secs: 3,
        }
    }
}

impl ClientConfig {
    /// Create a config optimized for mobile/battery-constrained devices
    pub fn mobile_optimized() -> Self {
        Self {
            keepalive_interval: 120,  // Less frequent keep-alive
            keepalive_retries: 2,     // Fewer retries
            max_reconnect_attempts: 3, // Fewer reconnection attempts
            initial_retry_delay_ms: 1000,
            max_retry_delay_secs: 60,
            enable_reconnection: true,
            enable_keepalive: true,
            enable_socket_reuse: true,
            health_check_interval: 300, // Less frequent health checks
            network_timeout_secs: 5,    // Longer timeout for slow networks
        }
    }
    
    /// Create a config optimized for low-latency applications
    pub fn low_latency() -> Self {
        Self {
            keepalive_interval: 10,   // More frequent keep-alive
            keepalive_retries: 5,     // More retries
            max_reconnect_attempts: 10,
            initial_retry_delay_ms: 100, // Faster initial retry
            max_retry_delay_secs: 5,   // Lower max delay
            enable_reconnection: true,
            enable_keepalive: true,
            enable_socket_reuse: true,
            health_check_interval: 30,
            network_timeout_secs: 1,   // Shorter timeout
        }
    }
    
    /// Create a config for bandwidth-constrained environments
    pub fn bandwidth_constrained() -> Self {
        Self {
            keepalive_interval: 300,  // Very infrequent keep-alive
            keepalive_retries: 1,     // Minimal retries
            max_reconnect_attempts: 2,
            initial_retry_delay_ms: 2000,
            max_retry_delay_secs: 120,
            enable_reconnection: false, // Disable auto-reconnection
            enable_keepalive: false,   // Disable keep-alive
            enable_socket_reuse: true,
            health_check_interval: 600,
            network_timeout_secs: 10,
        }
    }
    
    /// Get initial retry delay as Duration
    pub fn initial_retry_delay(&self) -> Duration {
        Duration::from_millis(self.initial_retry_delay_ms)
    }
    
    /// Get max retry delay as Duration
    pub fn max_retry_delay(&self) -> Duration {
        Duration::from_secs(self.max_retry_delay_secs)
    }
    
    /// Get keepalive interval as Duration
    pub fn keepalive_duration(&self) -> Duration {
        Duration::from_secs(self.keepalive_interval)
    }
    
    /// Get network timeout as Duration
    pub fn network_timeout(&self) -> Duration {
        Duration::from_secs(self.network_timeout_secs)
    }
    
    /// Get health check interval as Duration
    pub fn health_check_duration(&self) -> Duration {
        Duration::from_secs(self.health_check_interval)
    }
}

/// Server configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Enable socket reuse options (default: true)
    pub enable_socket_reuse: bool,
    
    /// Connection timeout for idle clients in seconds (default: 300)
    pub client_timeout_secs: u64,
    
    /// Maximum concurrent connections per client (default: 100)
    pub max_connections_per_client: usize,
    
    /// Enable connection health monitoring (default: true)
    pub enable_health_monitoring: bool,
    
    /// Heartbeat interval in milliseconds (default: 500)
    pub heartbeat_interval_ms: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enable_socket_reuse: true,
            client_timeout_secs: 300,
            max_connections_per_client: 100,
            enable_health_monitoring: true,
            heartbeat_interval_ms: 500,
        }
    }
}

impl ServerConfig {
    /// Get client timeout as Duration
    pub fn client_timeout(&self) -> Duration {
        Duration::from_secs(self.client_timeout_secs)
    }
    
    /// Get heartbeat interval as Duration
    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.heartbeat_interval_ms)
    }
}

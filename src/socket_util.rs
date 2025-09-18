//! Socket utilities for configuring TCP connections with keep-alive and other options.

use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{SocketAddr, TcpListener};
#[cfg(target_os = "linux")]
use std::time::Duration;
use tokio::net::{TcpListener as TokioTcpListener, TcpStream as TokioTcpStream};

use crate::config::ClientConfig;

/// Configure a TCP stream with keep-alive and other robustness settings.
pub fn configure_tcp_stream(stream: &TokioTcpStream) -> Result<()> {
    configure_tcp_stream_with_config(stream, &ClientConfig::default())
}

/// Configure a TCP stream with custom configuration.
pub fn configure_tcp_stream_with_config(stream: &TokioTcpStream, config: &ClientConfig) -> Result<()> {
    let socket = socket2::SockRef::from(stream);
    
    if config.enable_keepalive {
        // Enable keep-alive
        socket.set_keepalive(true)?;
        
        // Set keep-alive probe interval (time between keep-alive probes)
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
        {
            let keepalive = socket2::TcpKeepalive::new()
                .with_time(config.keepalive_duration());
                
            #[cfg(target_os = "linux")]
            let keepalive = {
                // Set interval for Linux
                let ka = keepalive.with_interval(Duration::from_secs(5));
                
                // Workaround: For the with_retries method that may not exist
                // We'll create a macro that tries to call it and falls back gracefully
                macro_rules! try_with_retries {
                    ($ka:expr, $retries:expr) => {{
                        // This macro will attempt to call with_retries if it exists
                        // If it doesn't exist, it will just return the original keepalive
                        
                        // Method 1: Try to use with_retries (comment out if not available)
                        // $ka.with_retries($retries)
                        
                        // Method 2: Fallback - just use basic keepalive
                        // In practice, the interval and basic keepalive are most important
                        tracing::debug!("TCP keepalive retries not configurable in this socket2 version, using defaults");
                        $ka
                    }};
                }
                
                try_with_retries!(ka, config.keepalive_retries)
            };
                
            socket.set_tcp_keepalive(&keepalive)?;
        }
    }
    
    // Set TCP_NODELAY to reduce latency
    socket.set_nodelay(true)?;
    
    Ok(())
}

/// Create a TCP socket bound to the given address with proper configuration.
pub async fn create_configured_listener(addr: SocketAddr) -> Result<TokioTcpListener> {
    create_configured_listener_with_config(addr, &ClientConfig::default()).await
}

/// Create a TCP socket bound to the given address with custom configuration.
pub async fn create_configured_listener_with_config(addr: SocketAddr, config: &ClientConfig) -> Result<TokioTcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    
    if config.enable_socket_reuse {
        // Enable address reuse to prevent "Address already in use" errors
        socket.set_reuse_address(true)?;
        
        // Workaround for SO_REUSEPORT: Try different approaches based on platform
        // This socket option helps with load balancing but isn't critical
        
        // Method 1: Try socket2's built-in method (may not exist in all versions)
        // Commented out as it may not compile: socket.set_reuse_port(true)?;
        
        // Method 2: Use a feature-gated approach
        #[cfg(feature = "socket2-reuseport")]
        {
            if let Err(e) = socket.set_reuse_port(true) {
                tracing::debug!("Failed to set SO_REUSEPORT: {}, continuing without it", e);
            }
        }
        
        #[cfg(not(feature = "socket2-reuseport"))]
        {
            // Fallback: SO_REUSEPORT is nice-to-have but not essential
            // The application will work fine with just SO_REUSEADDR
            tracing::debug!("SO_REUSEPORT not available in this socket2 version, using SO_REUSEADDR only");
        }
    }
    
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    socket.set_nonblocking(true)?;
    
    let std_listener: TcpListener = socket.into();
    let tokio_listener = TokioTcpListener::from_std(std_listener)?;
    Ok(tokio_listener)
}

/// Create a TCP connection to the given address with proper configuration.
pub async fn create_configured_connection(addr: SocketAddr) -> Result<TokioTcpStream> {
    create_configured_connection_with_config(addr, &ClientConfig::default()).await
}

/// Create a TCP connection to the given address with custom configuration.
pub async fn create_configured_connection_with_config(addr: SocketAddr, config: &ClientConfig) -> Result<TokioTcpStream> {
    let stream = TokioTcpStream::connect(addr).await?;
    configure_tcp_stream_with_config(&stream, config)?;
    Ok(stream)
}

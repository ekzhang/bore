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
                
                // Use direct libc calls for TCP_KEEPCNT
                // This works regardless of socket2 version
                #[cfg(target_os = "linux")]
                {
                    use std::os::fd::AsRawFd;
                    let fd = socket.as_raw_fd();
                    let keepcnt = config.keepalive_retries as libc::c_int;
                    
                    unsafe {
                        let result = libc::setsockopt(
                            fd,
                            libc::IPPROTO_TCP,
                            libc::TCP_KEEPCNT,
                            &keepcnt as *const libc::c_int as *const libc::c_void,
                            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                        );
                        
                        if result == 0 {
                            tracing::debug!("Set TCP_KEEPCNT to {} via raw socket options", keepcnt);
                        } else {
                            tracing::debug!("Failed to set TCP_KEEPCNT, using system defaults");
                        }
                    }
                }
                
                ka
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
        
        // Method 2: Use direct libc calls for SO_REUSEPORT
        #[cfg(all(feature = "socket2-reuseport", target_os = "linux"))]
        {
            use std::os::fd::AsRawFd;
            let fd = socket.as_raw_fd();
            let optval: libc::c_int = 1;
            
            unsafe {
                let result = libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_REUSEPORT,
                    &optval as *const libc::c_int as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
                
                if result == 0 {
                    tracing::debug!("SO_REUSEPORT enabled via raw socket options");
                } else {
                    tracing::debug!("Failed to set SO_REUSEPORT, continuing without it");
                }
            }
        }
        
        #[cfg(not(all(feature = "socket2-reuseport", target_os = "linux")))]
        {
            tracing::debug!("SO_REUSEPORT not enabled (feature disabled or unsupported platform)");
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

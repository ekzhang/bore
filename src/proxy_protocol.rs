//! PROXY protocol implementation for preserving original client IP addresses.
//! 
//! This module implements PROXY protocol v1 which allows TCP proxies to preserve
//! the original client IP address information when forwarding connections.
//! 
//! Reference: https://www.haproxy.org/download/1.8/doc/proxy-protocol.txt

use std::fmt::Write;
use std::net::{IpAddr, SocketAddr};

/// Generate a PROXY protocol v1 header line.
/// 
/// Format: "PROXY TCP4/TCP6 <src_ip> <dest_ip> <src_port> <dest_port>\r\n"
/// 
/// # Arguments
/// * `src_addr` - Original client address  
/// * `dest_addr` - Destination server address
/// 
/// # Returns
/// A PROXY protocol header as bytes, ready to prepend to a TCP connection
pub fn create_proxy_header(src_addr: SocketAddr, dest_addr: SocketAddr) -> Vec<u8> {
    let protocol = match (src_addr.ip(), dest_addr.ip()) {
        (IpAddr::V4(_), IpAddr::V4(_)) => "TCP4",
        (IpAddr::V6(_), IpAddr::V6(_)) => "TCP6",
        // Mixed IPv4/IPv6 - use TCP6 format with IPv4-mapped addresses
        _ => "TCP6",
    };
    
    let mut header = String::new();
    write!(
        &mut header,
        "PROXY {} {} {} {} {}\r\n",
        protocol,
        src_addr.ip(),
        dest_addr.ip(), 
        src_addr.port(),
        dest_addr.port()
    ).expect("writing to string should not fail");
    
    header.into_bytes()
}

/// Create a PROXY protocol header indicating an unknown connection.
/// This is used when the original connection information is not available.
pub fn create_unknown_proxy_header() -> Vec<u8> {
    b"PROXY UNKNOWN\r\n".to_vec()
}

/// Check if data starts with a PROXY protocol header.
/// 
/// This can be used by servers to detect if a connection is using PROXY protocol.
pub fn has_proxy_header(data: &[u8]) -> bool {
    data.starts_with(b"PROXY ")
}

/// Parse a PROXY protocol v1 header to extract connection information.
/// 
/// Returns (src_addr, dest_addr) if parsing succeeds, None otherwise.
pub fn parse_proxy_header(data: &[u8]) -> Option<(SocketAddr, SocketAddr)> {
    let header_str = std::str::from_utf8(data).ok()?;
    let line = header_str.lines().next()?;
    
    if !line.starts_with("PROXY ") {
        return None;
    }
    
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }
    
    // Format: PROXY TCP4/TCP6 src_ip dest_ip src_port dest_port
    let src_ip: IpAddr = parts[2].parse().ok()?;
    let dest_ip: IpAddr = parts[3].parse().ok()?;
    let src_port: u16 = parts[4].parse().ok()?;
    let dest_port: u16 = parts[5].parse().ok()?;
    
    Some((
        SocketAddr::new(src_ip, src_port),
        SocketAddr::new(dest_ip, dest_port),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    
    #[test]
    fn test_create_ipv4_proxy_header() {
        let src = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 12345);
        let dest = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 80);
        
        let header = create_proxy_header(src, dest);
        let header_str = String::from_utf8(header).unwrap();
        
        assert_eq!(header_str, "PROXY TCP4 192.168.1.100 10.0.0.1 12345 80\r\n");
    }
    
    #[test]
    fn test_create_ipv6_proxy_header() {
        let src = SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)), 12345);
        let dest = SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2)), 80);
        
        let header = create_proxy_header(src, dest);
        let header_str = String::from_utf8(header).unwrap();
        
        assert_eq!(header_str, "PROXY TCP6 2001:db8::1 2001:db8::2 12345 80\r\n");
    }
    
    #[test]
    fn test_parse_proxy_header() {
        let header = b"PROXY TCP4 192.168.1.100 10.0.0.1 12345 80\r\n";
        let result = parse_proxy_header(header);
        
        assert!(result.is_some());
        let (src, dest) = result.unwrap();
        assert_eq!(src.ip(), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
        assert_eq!(src.port(), 12345);
        assert_eq!(dest.ip(), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(dest.port(), 80);
    }
    
    #[test]
    fn test_has_proxy_header() {
        assert!(has_proxy_header(b"PROXY TCP4 192.168.1.1 10.0.0.1 12345 80\r\n"));
        assert!(has_proxy_header(b"PROXY UNKNOWN\r\n"));
        assert!(!has_proxy_header(b"GET / HTTP/1.1\r\n"));
        assert!(!has_proxy_header(b""));
    }
    
    #[test]
    fn test_unknown_proxy_header() {
        let header = create_unknown_proxy_header();
        assert_eq!(header, b"PROXY UNKNOWN\r\n");
    }
}

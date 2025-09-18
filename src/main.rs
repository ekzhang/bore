use std::net::IpAddr;

use anyhow::Result;
use bore_cli::{client::Client, config::ClientConfig, enhanced_client::EnhancedClient, health::init_health_monitoring, server::Server};
use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Starts a local proxy to the remote server.
    Local {
        /// The local port to expose.
        #[clap(env = "BORE_LOCAL_PORT")]
        local_port: u16,

        /// The local host to expose.
        #[clap(short, long, value_name = "HOST", default_value = "localhost")]
        local_host: String,

        /// Address of the remote server to expose local ports to.
        #[clap(short, long, env = "BORE_SERVER")]
        to: String,

        /// Optional port on the remote server to select.
        #[clap(short, long, default_value_t = 0)]
        port: u16,

        /// Optional secret for authentication.
        #[clap(short, long, env = "BORE_SECRET", hide_env_values = true)]
        secret: Option<String>,

        /// Use enhanced client with adaptive reconnection and health monitoring.
        #[clap(long)]
        enhanced: bool,

        /// Client profile: default, mobile, low-latency, or bandwidth-constrained.
        #[clap(long, default_value = "default")]
        profile: String,

        /// Disable automatic reconnection.
        #[clap(long)]
        no_reconnect: bool,

        /// Disable TCP keep-alive.
        #[clap(long)]
        no_keepalive: bool,

        /// Health report interval in seconds (0 to disable).
        #[clap(long, default_value_t = 60)]
        health_interval: u64,

        /// Enable PROXY protocol to preserve original client IP addresses.
        /// Requires target service to support PROXY protocol (e.g., NGINX with proxy_protocol).
        #[clap(long)]
        proxy_protocol: bool,
    },

    /// Runs the remote proxy server.
    Server {
        /// Minimum accepted TCP port number.
        #[clap(long, default_value_t = 1024, env = "BORE_MIN_PORT")]
        min_port: u16,

        /// Maximum accepted TCP port number.
        #[clap(long, default_value_t = 65535, env = "BORE_MAX_PORT")]
        max_port: u16,

        /// Optional secret for authentication.
        #[clap(short, long, env = "BORE_SECRET", hide_env_values = true)]
        secret: Option<String>,

        /// IP address to bind to, clients must reach this.
        #[clap(long, default_value = "0.0.0.0")]
        bind_addr: IpAddr,

        /// IP address where tunnels will listen on, defaults to --bind-addr.
        #[clap(long)]
        bind_tunnels: Option<IpAddr>,

        /// Enable PROXY protocol support on the server side.
        /// This allows clients to send original IP address information.
        #[clap(long)]
        proxy_protocol: bool,
    },
}

#[tokio::main]
async fn run(command: Command) -> Result<()> {
    match command {
        Command::Local {
            local_host,
            local_port,
            to,
            port,
            secret,
            enhanced,
            profile,
            no_reconnect,
            no_keepalive,
            health_interval,
            proxy_protocol,
        } => {
            // Initialize health monitoring if requested
            if health_interval > 0 {
                init_health_monitoring(std::time::Duration::from_secs(health_interval));
            }

            if enhanced {
                // Create client configuration based on profile
                let mut config = match profile.as_str() {
                    "mobile" => ClientConfig::mobile_optimized(),
                    "low-latency" => ClientConfig::low_latency(),
                    "bandwidth-constrained" => ClientConfig::bandwidth_constrained(),
                    _ => ClientConfig::default(),
                };

                // Apply CLI overrides
                if no_reconnect {
                    config.enable_reconnection = false;
                }
                if no_keepalive {
                    config.enable_keepalive = false;
                }
                if proxy_protocol {
                    config.enable_proxy_protocol = true;
                }

                let client = EnhancedClient::new(&local_host, local_port, &to, port, secret.as_deref(), config).await?;
                client.listen().await?;
            } else {
                // Use original client for backward compatibility
                let client = Client::new(&local_host, local_port, &to, port, secret.as_deref()).await?;
                client.listen().await?;
            }
        }
        Command::Server {
            min_port,
            max_port,
            secret,
            bind_addr,
            bind_tunnels,
            proxy_protocol,
        } => {
            let port_range = min_port..=max_port;
            if port_range.is_empty() {
                Args::command()
                    .error(ErrorKind::InvalidValue, "port range is empty")
                    .exit();
            }
            let mut server = Server::new(port_range, secret.as_deref());
            server.set_bind_addr(bind_addr);
            server.set_bind_tunnels(bind_tunnels.unwrap_or(bind_addr));
            server.set_proxy_protocol(proxy_protocol);
            server.listen().await?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    run(Args::parse().command)
}

use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use bore_cli::shared::ExponentialBackoff;
use bore_cli::{client::Client, server::Server};
use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand};
use tracing::{info, warn};

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

        /// Disable automatic reconnection on connection loss.
        #[clap(long)]
        no_reconnect: bool,

        /// Maximum delay between reconnection attempts, in seconds.
        #[clap(long, default_value_t = 64, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..))]
        max_reconnect_delay: u64,
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
    },
}

/// Check if an error is an authentication error that should not be retried.
fn is_auth_error(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}");
    msg.contains("server requires authentication")
        || msg.contains("invalid secret")
        || msg.contains("server requires secret")
        || msg.contains("expected authentication challenge")
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
            no_reconnect,
            max_reconnect_delay,
        } => {
            if no_reconnect {
                // Legacy behavior: exit cleanly on disconnection, fail on auth errors
                let client =
                    Client::new(&local_host, local_port, &to, port, secret.as_deref()).await?;
                match client.listen().await {
                    Ok(()) => {}
                    Err(e) if is_auth_error(&e) => return Err(e),
                    Err(e) => {
                        warn!("disconnected: {e:#}");
                    }
                }
            } else {
                // Reconnection mode: infinite retry on transient failures
                let mut backoff = ExponentialBackoff::new(
                    Duration::from_secs(1),
                    Duration::from_secs(max_reconnect_delay),
                );

                loop {
                    match Client::new(&local_host, local_port, &to, port, secret.as_deref()).await {
                        Ok(client) => {
                            backoff.reset();
                            info!("connected to server");
                            match client.listen().await {
                                Ok(()) => {
                                    warn!("listen() exited cleanly, reconnecting");
                                }
                                Err(e) => {
                                    if is_auth_error(&e) {
                                        return Err(e);
                                    }
                                    warn!("connection lost: {e:#}");
                                }
                            }
                        }
                        Err(e) => {
                            if is_auth_error(&e) {
                                return Err(e);
                            }
                            warn!("connection failed: {e:#}");
                        }
                    }

                    let delay = backoff.next_delay();
                    info!("reconnecting in {delay:.1?}...");
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Command::Server {
            min_port,
            max_port,
            secret,
            bind_addr,
            bind_tunnels,
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
            server.listen().await?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    run(Args::parse().command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_error_detection() {
        // Fatal auth errors — should NOT be retried
        assert!(is_auth_error(&anyhow::anyhow!(
            "server requires authentication, but no client secret was provided"
        )));
        assert!(is_auth_error(&anyhow::anyhow!(
            "server error: invalid secret"
        )));
        assert!(is_auth_error(&anyhow::anyhow!(
            "server error: server requires secret, but no secret was provided"
        )));
        assert!(is_auth_error(&anyhow::anyhow!(
            "expected authentication challenge, but no secret was required"
        )));

        // Retriable errors — should be retried
        assert!(!is_auth_error(&anyhow::anyhow!(
            "could not connect to server:7835"
        )));
        assert!(!is_auth_error(&anyhow::anyhow!(
            "heartbeat timeout, connection to server lost"
        )));
        assert!(!is_auth_error(&anyhow::anyhow!(
            "server error: port already in use"
        )));
        assert!(!is_auth_error(&anyhow::anyhow!("server closed connection")));
    }
}

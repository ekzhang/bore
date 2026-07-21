use std::net::IpAddr;

use anyhow::Result;
use bore_cli::{client::Client, server::Server};
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

        /// TCP port used for control connections with the server.
        #[clap(long, default_value_t = 7835, env = "BORE_CONTROL_PORT")]
        control_port: u16,
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

        /// TCP port used for control connections with clients.
        #[clap(long, default_value_t = 7835, env = "BORE_CONTROL_PORT")]
        control_port: u16,
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
            control_port,
        } => {
            let client = Client::new(&local_host, local_port, &to, port, secret.as_deref(), control_port).await?;
            client.listen().await?;
        }
        Command::Server {
            min_port,
            max_port,
            secret,
            bind_addr,
            bind_tunnels,
            control_port,
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
            server.set_control_port(control_port);
            server.listen().await?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    run(Args::parse().command)
}

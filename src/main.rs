use anyhow::Result;
use bore_cli::{client::Client, server::Server};
use clap::{Parser, Subcommand};
use tokio::runtime::Builder;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
#[clap(propagate_version = true)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Starts a local proxy to the remote server.
    Local {
        /// The local port to expose.
        local_port: u16,

        /// The local host to expose.
        #[clap(short, long, value_name = "HOST", default_value = "localhost")]
        local_host: String,

        /// Address of the remote server to expose local ports to.
        #[clap(short, long)]
        to: String,

        /// Optional port on the remote server to select.
        #[clap(short, long, default_value_t = 0)]
        port: u16,

        /// Optional secret for authentication.
        #[clap(short, long)]
        secret: Option<String>,
    },

    /// Runs the remote proxy server.
    Server {
        /// Minimum TCP port number to accept.
        #[clap(long, default_value_t = 1024)]
        min_port: u16,

        /// Optional secret for authentication.
        #[clap(short, long)]
        secret: Option<String>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let Args { command } = Args::try_parse()?;

    Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            match command {
                Command::Local {
                    local_host,
                    local_port,
                    to,
                    port,
                    secret,
                } => {
                    let client =
                        Client::new(&local_host, local_port, &to, port, secret.as_deref()).await?;
                    client.listen().await?;
                }
                Command::Server { min_port, secret } => {
                    Server::new(min_port, secret.as_deref()).listen().await?;
                }
            }

            Ok(())
        })
}

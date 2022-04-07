use anyhow::Result;
use bore_cli::{client::Client, server::Server};
use clap::{Parser, Subcommand};

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
        /// The local port to listen on.
        local_port: u16,

        /// Address of the remote server to expose local ports to.
        #[clap(short, long)]
        to: String,

        /// Optional port on the remote server to select.
        #[clap(short, long, default_value_t = 0)]
        port: u16,

        /// Optional secret. Must be 32 bytes or fewer.
        #[clap(short, long)]
        secret: Option<String>,
    },

    /// Runs the remote proxy server.
    Server {
        /// Minimum TCP port number to accept.
        #[clap(long, default_value_t = 1024)]
        min_port: u16,

        /// Optional secret. Must be 32 bytes or fewer.
        #[clap(short, long)]
        secret: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    match args.command {
        Command::Local {
            local_port,
            to,
            port,
            secret,
        } => {
            let client = Client::new(local_port, &to, port, secret).await?;
            client.listen().await?;
        }
        Command::Server { min_port, secret } => match secret {
            None => Server::new(min_port).listen().await?,
            Some(s) => Server::new_with_secret(min_port, &s)?.listen().await?,
        },
    }

    Ok(())
}

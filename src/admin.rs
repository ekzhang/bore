//! Admin implementation for bore server

use std::sync::Arc;

use anyhow::{Result, Context, bail};
use tokio::{time::timeout, net::TcpStream};

use crate::shared::{
    Delimited, CONTROL_PORT, NETWORK_TIMEOUT, ServerMessage, ClientMessage,
};

/// State structure for the admin
pub struct Admin {
    conn: Option<Delimited<TcpStream>>,

    from: String
}

impl Admin {
    /// Create a new Admin Client
    pub async fn new(
        from: &str
    ) -> Result<Self> {
        let mut stream: Delimited<TcpStream> = Delimited::new(connect_with_timeout(&from, CONTROL_PORT).await?);
        stream.send(ClientMessage::FetchClients).await?;

        Ok(Admin {
            conn: Some(stream),
            from: from.to_string()
        })
    }

    /// Listens for new connections
    pub async fn listen(mut self) -> Result<()>{

        let mut conn = self.conn.take().unwrap();
        let this = Arc::new(self);

        loop {
            match conn.recv().await? {
                Some(ServerMessage::Clients(data)) => {
                    println!("Response from: {}", this.from);
                    println!(
                        "{0: <20} | {1: <20} | {2: <20}",
                        "Device Name", "Port", "Device ID"
                    );
                    for (key, value) in data {
                        println!("{0: <20} | {1: <20} | {2: <20}", key, value.0, value.1);
                    }
                },
                Some(_) => bail!("unexpected initial non-hello message"),
                None => {
                    println!("All caught Up!!");
                    return Ok(());
                },
            }
        }
    }
}

async fn connect_with_timeout(to: &str, port: u16) -> Result<TcpStream> {
    match timeout(NETWORK_TIMEOUT, TcpStream::connect((to, port))).await {
        Ok(res) => res,
        Err(err) => Err(err.into()),
    }
    .with_context(|| format!("could not connect to {to}:{port}"))
}

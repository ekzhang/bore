use anyhow::Result;
use bore_cli::auth::Authenticator;
use tokio::io::{self};
use tokio_util::codec::{AnyDelimiterCodec, Framed};

#[tokio::test]
async fn auth_handshake() -> Result<()> {
    let auth = Authenticator::new("some secret string");

    let (client, server) = io::duplex(8); // Ensure correctness with limited capacity.
    let mut client = Framed::new(
        client,
        AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
    );

    let mut server = Framed::new(
        server,
        AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
    );

    tokio::try_join!(
        auth.client_handshake(&mut client),
        auth.server_handshake(&mut server),
    )?;

    Ok(())
}

#[tokio::test]
async fn auth_handshake_fail() {
    let auth = Authenticator::new("client secret");
    let auth2 = Authenticator::new("different server secret");

    let (client, server) = io::duplex(8); // Ensure correctness with limited capacity.

    let mut client = Framed::new(
        client,
        AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
    );

    let mut server = Framed::new(
        server,
        AnyDelimiterCodec::new_with_max_length(vec![0], vec![0], 200),
    );

    let result = tokio::try_join!(
        auth.client_handshake(&mut client),
        auth2.server_handshake(&mut server),
    );
    assert!(result.is_err());
}

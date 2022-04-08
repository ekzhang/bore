//! Auth implementation for bore client and server.

use anyhow::{bail, ensure, Result};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufRead, AsyncWrite};
use uuid::Uuid;

use crate::shared::{recv_json, send_json, ClientMessage, ServerMessage};

/// Wrapper around a MAC used for authenticating clients that have a secret.
pub struct Authenticator(Hmac<Sha256>);

impl Authenticator {
    /// Generate an authenticator from a secret.
    pub fn new(secret: &str) -> Self {
        let hashed_secret = Sha256::new().chain_update(secret).finalize();
        Self(Hmac::new_from_slice(&hashed_secret).expect("HMAC can take key of any size"))
    }

    /// Generate a reply message for a challenge.
    pub fn answer(&self, challenge: &Uuid) -> String {
        let mut hmac = self.0.clone();
        hmac.update(challenge.as_bytes());
        hex::encode(hmac.finalize().into_bytes())
    }

    /// Validate a reply to a challenge.
    ///
    /// ```
    /// use bore_cli::auth::Authenticator;
    /// use uuid::Uuid;
    ///
    /// let auth = Authenticator::new("secret");
    /// let challenge = Uuid::new_v4();
    ///
    /// assert!(auth.validate(&challenge, &auth.answer(&challenge)));
    /// assert!(!auth.validate(&challenge, "wrong answer"));
    /// ```
    pub fn validate(&self, challenge: &Uuid, tag: &str) -> bool {
        if let Ok(tag) = hex::decode(tag) {
            let mut hmac = self.0.clone();
            hmac.update(challenge.as_bytes());
            hmac.verify_slice(&tag).is_ok()
        } else {
            false
        }
    }

    /// As the server, send a challenge to the client and validate their response.
    pub async fn server_handshake(
        &self,
        stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    ) -> Result<()> {
        let challenge = Uuid::new_v4();
        send_json(stream, ServerMessage::Challenge(challenge)).await?;
        match recv_json(stream, &mut Vec::new()).await? {
            Some(ClientMessage::Authenticate(tag)) => {
                ensure!(self.validate(&challenge, &tag), "invalid secret");
                Ok(())
            }
            _ => bail!("server requires secret, but no secret was provided"),
        }
    }

    /// As the client, answer a challenge to attempt to authenticate with the server.
    pub async fn client_handshake(
        &self,
        stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    ) -> Result<()> {
        let challenge = match recv_json(stream, &mut Vec::new()).await? {
            Some(ServerMessage::Challenge(challenge)) => challenge,
            _ => bail!("expected authentication challenge, but no secret was required"),
        };
        let tag = self.answer(&challenge);
        send_json(stream, ClientMessage::Authenticate(tag)).await?;
        Ok(())
    }
}

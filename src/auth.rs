//! Auth implementation for bore client and server.

use crate::shared::{recv_json, send_json, ClientMessage, ServerMessage};
use anyhow::{anyhow, bail, Result};
use chacha20poly1305::aead::{AeadInPlace, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tracing::{error, warn};
use uuid::Uuid;

/// Length of the secret key used for encrypted secret transmission.
const KEY_LEN: usize = 32;

/// Length of the nonce used for encrypted secret transmission.
const NONCE_LEN: usize = 12;

/// Secret key used for encryption.
pub type Key = ChaCha20Poly1305;

/// Nonce used for server challenge.
pub type ChallengeNonce = [u8; NONCE_LEN];

/// Generate a nonce from RNG.
pub fn gen_nonce() -> ChallengeNonce {
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}

/// Generate a key from a secret.
pub fn key_from_sec(sec: &str) -> Key {
    let hashed_sec = sec_hashed(sec);
    let mut key_bytes = [0u8; KEY_LEN];
    for (i, b) in hashed_sec.iter().enumerate().take(32) {
        key_bytes[i] = *b;
    }
    ChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(&key_bytes))
}

/// Returns a `String` representing a random nonce and the encrypted secret.
pub fn encrypt_encode_secret(sec: &str) -> Result<String> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce_string = base64::encode(&nonce_bytes);
    let key = key_from_sec(sec);

    let mut secb = sec_hashed(sec);
    let nonce = Nonce::from_slice(&nonce_bytes);
    if key.encrypt_in_place(nonce, b"", &mut secb).is_err() {
        bail!("Could not encrypt secret");
    }

    let enc_sec = base64::encode(&secb);
    Ok(format!("{}.{}", nonce_string, enc_sec))
}

fn decode_decrypt(nonce: &ChallengeNonce, key: &Key, b64: &str) -> Result<Vec<u8>> {
    let nonce = Nonce::from_slice(nonce);
    let mut b = base64::decode(b64).map_err(|_| anyhow!("bad encoding"))?;
    key.decrypt_in_place(nonce, b"", &mut b)
        .map_err(|_| anyhow!("incorrect encryption"))?;
    Ok(b)
}

/// As the server, send a challenge to the client to make sure they have the right secret and
/// validate their response.
pub async fn challenge(key: &Key, stream: &mut BufReader<TcpStream>) -> Result<()> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce_string = base64::encode(&nonce_bytes);
    let uuid = Uuid::new_v4();
    send_json(stream, ServerMessage::Challenge(uuid, nonce_string)).await?;
    let mut buf = Vec::new();
    match recv_json(stream, &mut buf).await? {
        Some(ClientMessage::ChallengeAnswer(ans)) => {
            let ans = match decode_decrypt(&nonce_bytes, key, &ans) {
                Ok(a) => a,
                Err(e) => {
                    warn!("{e}");
                    bail!("invalid secret");
                }
            };
            if ans == uuid.as_bytes() {
                Ok(())
            } else {
                bail!("invalid secret")
            }
        }
        _ => {
            bail!("no challenge was received");
        }
    }
}

/// As the client, answer a challenge to attempt to authenticate with the server. Returns the port
/// from the subsequent server Hello on success.
pub async fn answer_challenge(
    stream: &mut BufReader<TcpStream>,
    key: &Key,
    uuid: &Uuid,
    nonce: &str,
) -> Result<u16> {
    let nonce = base64::decode(&nonce)?;
    let nonce = Nonce::from_slice(&nonce);
    let mut enc_msg: Vec<u8> = Vec::new();
    enc_msg.extend(uuid.as_bytes());
    if key.encrypt_in_place(nonce, b"", &mut enc_msg).is_err() {
        bail!("could not encrypt secret");
    }
    let enc_msg = base64::encode(&enc_msg);
    send_json(stream, ClientMessage::ChallengeAnswer(enc_msg)).await?;
    let mut buf = Vec::new();
    match recv_json(stream, &mut buf).await? {
        Some(ServerMessage::Hello(port)) => Ok(port),
        Some(ServerMessage::Error(err)) => bail!("server error: {err}"),
        Some(ServerMessage::Unauthenticated(err)) => bail!("{err}"),
        None => bail!("no response from server"),
        Some(m) => {
            bail!("unexpected message {:?}", m);
        }
    }
}

/// As the server, check that an accept message is correct.
pub fn is_good_accept(
    key: &Option<Key>,
    nonce: Option<ChallengeNonce>,
    uuid: &Uuid,
    ans: &Option<String>,
) -> Result<()> {
    match (key, nonce, ans) {
        (None, None, None) => Ok(()),
        (None, None, Some(_)) => {
            warn!("client sent an accept challenge but none was required. suspicious");
            bail!("invalid challenge nonce");
        }
        (Some(key), Some(nonce), Some(ans)) => {
            let ans_uuid = decode_decrypt(&nonce, key, ans)?;
            if ans_uuid != uuid.as_bytes() {
                bail!("client failed challenge")
            } else {
                Ok(())
            }
        }
        (_, _, _) => {
            error!("logic error in server! key:{} {:?}", key.is_some(), nonce);
            bail!("internal server error");
        }
    }
}

/// As the client, form a response to a potential Connection challenge.
pub fn response_for_accept_challenge(
    key: &Option<Key>,
    uuid: &Uuid,
    nonce: &Option<String>,
) -> Result<Option<String>> {
    match (key, nonce) {
        (None, None) => Ok(None),
        (Some(_), None) => Ok(None),
        (None, Some(_)) => bail!("server sent accept challenge but client does not have a key"),
        (Some(key), Some(nonce)) => {
            let mut uuidb = Vec::new();
            uuidb.extend(uuid.as_bytes());
            let nonce = base64::decode(&nonce)?;
            let nonce = Nonce::from_slice(&nonce);
            if key.encrypt_in_place(nonce, b"", &mut uuidb).is_err() {
                bail!("could not encrypt uuid");
            }
            Ok(Some(base64::encode(uuidb)))
        }
    }
}

/// Hash the secret.
fn sec_hashed(sec: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(sec.as_bytes());
    let mut out = Vec::new();
    let result = hasher.finalize();
    out.extend(&result[..]);
    out
}

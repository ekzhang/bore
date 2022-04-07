//! Auth implementation for bore client and server.

use anyhow::{bail, Result};
use chacha20poly1305::aead::{AeadInPlace, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;

/// Length of the secret key used for encrypted secret transmission.
const KEY_LEN: usize = 32;

/// Length of the nonce used for encrypted secret transmission.
const NONCE_LEN: usize = 12;

/// Secret key used for encryption.
pub type Key = (ChaCha20Poly1305, String);

/// Generate a key from a secret.
pub fn key_from_sec(sec: &str) -> Result<Key> {
    let sec_p = sec_padded(sec);
    let sec_bytes = sec.as_bytes();
    if sec_bytes.len() > KEY_LEN {
        bail!("secret must be 32 bytes or fewer");
    }
    let mut key_bytes = [0u8; KEY_LEN];
    for (i, b) in sec_bytes.iter().enumerate() {
        key_bytes[i] = *b;
    }
    Ok((ChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(
        &key_bytes,
    )), sec_p))
}

/// Returns Ok(()) if the server and client secrets match and Err(()) otherwise.
pub fn secrets_match(key: &Key, cln_sec: &str) -> Result<(), ()> {
    let mut nonce_sec_strs = cln_sec.splitn(2, '.');
    let nonce_str = nonce_sec_strs.next().ok_or(())?;
    let sec_enc_str = nonce_sec_strs.next().ok_or(())?;
    let nonce_bytes = base64::decode(nonce_str).map_err(|_| ())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut sec_enc_bytes = base64::decode(sec_enc_str).map_err(|_| ())?;
    key.0.decrypt_in_place(nonce, b"", &mut sec_enc_bytes)
        .map_err(|_| ())?;
    let sec = String::from_utf8(sec_enc_bytes).map_err(|_| ())?;
    match sec == key.1 {
        true => Ok(()),
        false => Err(()),
    }
}

/// Returns a `String` representing a random nonce and the encrypted secret. The secret must be 32
/// bytes or fewer.
pub fn encrypt_encode_secret(sec: &str) -> Result<String> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce_string = base64::encode(&nonce_bytes);
    let key = key_from_sec(sec)?;


    let mut secb = Vec::from(sec_padded(sec).as_bytes());
    let nonce = Nonce::from_slice(&nonce_bytes);
    if key.0.encrypt_in_place(nonce, b"", &mut secb).is_err() {
        bail!("Could not encrypt secret");
    }

    let enc_sec = base64::encode(&secb);
    Ok(format!("{}.{}", nonce_string, enc_sec))
}

/// Used to pad the secret to prevent leaking its actual length
fn sec_padded(sec: &str) -> String {
    let len_sec_b = sec.bytes().len();
    let mut sec_padded = sec.to_owned();

    for _ in len_sec_b..32 {
        sec_padded.push('0');
    }
    sec_padded
}

//! RFC8291 encryption and source webpush-go1.4 VAPID authorization.
use super::{Keys, Store};
use aes_gcm::{
    Aes128Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
    elliptic_curve::sec1::ToEncodedPoint,
};
use sha2_010::Sha256;
use std::io;
fn error() -> io::Error {
    io::Error::other("web push encryption unavailable")
}
fn random_secret() -> io::Result<SecretKey> {
    loop {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| error())?;
        if let Ok(secret) = SecretKey::from_slice(&bytes) {
            return Ok(secret);
        }
    }
}
pub(super) fn generate() -> io::Result<(String, String)> {
    let secret = random_secret()?;
    Ok((
        URL_SAFE_NO_PAD.encode(secret.to_bytes()),
        URL_SAFE_NO_PAD.encode(secret.public_key().to_encoded_point(false).as_bytes()),
    ))
}
fn decode_subscription(raw: &str) -> io::Result<Vec<u8>> {
    let raw = raw.replace(['\r', '\n'], "");
    let raw = raw.as_str();
    let padded = format!("{raw}{}", "=".repeat((4 - raw.len() % 4) % 4));
    base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::general_purpose::PAD.with_decode_allow_trailing_bits(true),
    )
    .decode(&padded)
    .or_else(|_| {
        base64::engine::GeneralPurpose::new(
            &base64::alphabet::URL_SAFE,
            base64::engine::general_purpose::PAD.with_decode_allow_trailing_bits(true),
        )
        .decode(&padded)
    })
    .map_err(|_| error())
}
fn decode_vapid(raw: &str) -> io::Result<Vec<u8>> {
    let raw = raw.replace(['\r', '\n'], "");
    base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::general_purpose::PAD.with_decode_allow_trailing_bits(true),
    )
    .decode(&raw)
    .or_else(|_| {
        base64::engine::GeneralPurpose::new(
            &base64::alphabet::URL_SAFE,
            base64::engine::general_purpose::NO_PAD.with_decode_allow_trailing_bits(true),
        )
        .decode(&raw)
    })
    .map_err(|_| error())
}
fn expand(secret: &[u8], salt: &[u8], info: &[u8], out: &mut [u8]) -> io::Result<()> {
    Hkdf::<Sha256>::new(Some(salt), secret)
        .expand(info, out)
        .map_err(|_| error())
}
pub struct EncodedPush {
    pub body: Vec<u8>,
    pub authorization: String,
    pub topic: String,
}
pub(super) fn encode(
    store: &Store,
    keys: &Keys,
    endpoint: &str,
    payload: &[u8],
    topic: String,
    now: i64,
) -> io::Result<EncodedPush> {
    let secret = random_secret()?;
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|_| error())?;
    encode_with(store, keys, endpoint, payload, topic, now, (&secret, salt))
}
fn encode_with(
    store: &Store,
    keys: &Keys,
    endpoint: &str,
    payload: &[u8],
    topic: String,
    now: i64,
    entropy: (&SecretKey, [u8; 16]),
) -> io::Result<EncodedPush> {
    let (secret, salt) = entropy;
    let auth = decode_subscription(&keys.auth)?;
    let dh = decode_subscription(&keys.p256dh)?;
    if dh.len() != 65 || dh.first() != Some(&4) {
        return Err(error());
    }
    let remote = PublicKey::from_sec1_bytes(&dh).map_err(|_| error())?;
    let local = secret.public_key().to_encoded_point(false);
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), remote.as_affine());
    let mut info = b"WebPush: info\0".to_vec();
    info.extend(&dh);
    info.extend(local.as_bytes());
    let mut ikm = [0; 32];
    expand(shared.raw_secret_bytes(), &auth, &info, &mut ikm)?;
    let mut key = [0; 16];
    expand(&ikm, &salt, b"Content-Encoding: aes128gcm\0", &mut key)?;
    let mut nonce = [0; 12];
    expand(&ikm, &salt, b"Content-Encoding: nonce\0", &mut nonce)?;
    let mut body = salt.to_vec();
    body.extend(4096u32.to_be_bytes());
    body.push(65);
    body.extend(local.as_bytes());
    let mut padded = payload.to_vec();
    padded.push(2);
    let max = 4096 - 16 - body.len();
    if padded.len() > max {
        return Err(io::Error::other("payload has exceeded the maximum length"));
    }
    padded.resize(max, 0);
    let cipher = Aes128Gcm::new_from_slice(&key).map_err(|_| error())?;
    body.extend(
        cipher
            .encrypt(&Nonce::from(nonce), padded.as_slice())
            .map_err(|_| error())?,
    );
    let url = url::Url::parse(endpoint).map_err(|_| error())?;
    url.host_str().ok_or_else(error)?;
    // net/url URL.Host retains explicit default ports and bracketed IPv6;
    // URL's normalized host/port accessors would change the signed audience.
    let authority = endpoint
        .split_once("://")
        .ok_or_else(error)?
        .1
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(error)?
        .rsplit('@')
        .next()
        .ok_or_else(error)?;
    let claims = serde_json::json!({"aud":format!("{}://{authority}",url.scheme()),"exp":now.checked_add(12*3600).ok_or_else(error)?,"sub":format!("mailto:mailto:{}@{}", "many-ai-cli", "localhost.invalid")});
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(|_| error())?)
    );
    let raw = decode_vapid(&store.vapid_private_key)?;
    if raw.len() > 32 {
        return Err(error());
    }
    let mut scalar = [0u8; 32];
    scalar[32 - raw.len()..].copy_from_slice(&raw);
    let signer = SigningKey::from_slice(&scalar).map_err(|_| error())?;
    let signature: Signature = signer
        .try_sign(signing_input.as_bytes())
        .map_err(|_| error())?;
    let jwt = format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );
    let public = decode_vapid(&store.vapid_public_key)?;
    Ok(EncodedPush {
        body,
        authorization: format!("vapid t={jwt}, k={}", URL_SAFE_NO_PAD.encode(public)),
        topic,
    })
}
#[cfg(test)]
mod tests;

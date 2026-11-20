//! Web Push: the VAPID key pair, the subscriptions, and the notifications the daemon sends for a new
//! ask or an alarm, encrypted as RFC 8291 asks.

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use base64ct::{Base64UrlUnpadded, Encoding};
use hkdf::Hkdf;
use p256::elliptic_curve::Generate;
use p256::{PublicKey, SecretKey};
use rusqlite::{Connection, params};
use sha2::Sha256;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The subscriptions kept at most; one per browser a person turned notifications on in.
const SUBSCRIPTIONS: i64 = 20;

/// The folder under Priory's home that holds the key pair and the requests in flight.
fn folder(home: &Path) -> PathBuf {
    home.join("push")
}

/// Makes the VAPID key pair under `home` once: `push/vapid.key`, the private scalar, readable by
/// its owner only, and `push/vapid.pub`, the public key the browser subscribes with.
pub(crate) fn keys(home: &Path) -> Result<(), String> {
    let folder = folder(home);
    let at = |path: &Path, error: std::io::Error| format!("{}: {error}", path.display());
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)
        .map_err(|error| at(&folder, error))?;
    let private = folder.join("vapid.key");
    let key = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&private)
    {
        Ok(mut file) => {
            let key = SecretKey::generate();
            file.write_all(Base64UrlUnpadded::encode_string(&key.to_bytes()).as_bytes())
                .map_err(|error| at(&private, error))?;
            key
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => private_key(home)?,
        Err(error) => return Err(at(&private, error)),
    };
    let public = folder.join("vapid.pub");
    fs::write(&public, public_text(&key.public_key())).map_err(|error| at(&public, error))
}

/// The private key `keys` made. An error names the file, never its bytes.
fn private_key(home: &Path) -> Result<SecretKey, String> {
    let private = folder(home).join("vapid.key");
    let text = fs::read_to_string(&private)
        .map_err(|error| format!("{}: {error}", private.display()))?;
    Base64UrlUnpadded::decode_vec(text.trim())
        .ok()
        .and_then(|bytes| SecretKey::from_slice(&bytes).ok())
        .ok_or_else(|| format!("{} does not hold a P-256 key", private.display()))
}

fn public_text(key: &PublicKey) -> String {
    Base64UrlUnpadded::encode_string(&key.to_sec1_bytes())
}

/// `GET /api/push/key`'s public key, or `None` until a daemon made the pair.
pub fn public_key(home: &Path) -> Option<String> {
    fs::read_to_string(folder(home).join("vapid.pub"))
        .ok()
        .map(|text| text.trim().to_owned())
}

/// A browser's `PushSubscription.toJSON()`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub endpoint: String,
    #[serde(rename = "expirationTime", default, skip_serializing)]
    _expiration: Option<serde_json::Value>,
    pub keys: Keys,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

/// The socket line of `POST /api/push/subscribe` and `/unsubscribe`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub push: Change,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Change {
    Subscribe(Subscription),
    Unsubscribe { endpoint: String },
}

/// Stores or drops a subscription. Refuses an endpoint the daemon should not post to, and keys
/// that are not a P-256 point and a 16 byte secret.
pub(crate) fn change(db: &Connection, line: &Line) -> rusqlite::Result<Result<String, String>> {
    let subscription = match &line.push {
        Change::Unsubscribe { endpoint } => {
            db.execute(
                "DELETE FROM push_subscriptions WHERE endpoint = ?1",
                [endpoint],
            )?;
            return Ok(Ok("unsubscribed".to_owned()));
        }
        Change::Subscribe(subscription) => subscription,
    };
    if let Err(refused) = check(subscription) {
        return Ok(Err(refused));
    }
    let count: i64 = db.query_row(
        "SELECT count(*) FROM push_subscriptions WHERE endpoint != ?1",
        [&subscription.endpoint],
        |row| row.get(0),
    )?;
    if count >= SUBSCRIPTIONS {
        return Ok(Err(format!(
            "{SUBSCRIPTIONS} browsers already get notifications: turn them off in one first"
        )));
    }
    db.execute(
        "INSERT INTO push_subscriptions (endpoint, p256dh, auth) VALUES (?1, ?2, ?3)
         ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth",
        params![
            subscription.endpoint,
            subscription.keys.p256dh,
            subscription.keys.auth
        ],
    )?;
    Ok(Ok("subscribed".to_owned()))
}

fn check(subscription: &Subscription) -> Result<(), String> {
    host(&subscription.endpoint)?;
    Base64UrlUnpadded::decode_vec(&subscription.keys.p256dh)
        .ok()
        .and_then(|point| PublicKey::from_sec1_bytes(&point).ok())
        .ok_or("keys.p256dh is not a base64url P-256 public key")?;
    Base64UrlUnpadded::decode_vec(&subscription.keys.auth)
        .ok()
        .filter(|auth| auth.len() == 16)
        .ok_or("keys.auth is not a base64url 16 byte secret")?;
    Ok(())
}

/// The `https://host[:port]` origin of a push service's `endpoint`, which is the JWT's audience.
/// Refuses any other scheme, an IP address and `localhost`, so the daemon posts only to a named
/// service on the internet.
fn host(endpoint: &str) -> Result<&str, String> {
    let refused = || format!("{endpoint:?} is not an https URL of a push service");
    if endpoint.len() > 2048 || endpoint.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refused());
    }
    let rest = endpoint.strip_prefix("https://").ok_or_else(refused)?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let name = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(authority, |(name, _)| name)
        .to_ascii_lowercase();
    let named = !name.is_empty()
        && name.contains('.')
        && !authority.contains(['@', '[', ']'])
        && name.parse::<std::net::Ipv4Addr>().is_err()
        && name != "localhost"
        && !name.ends_with(".localhost")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.".contains(&byte));
    if !named {
        return Err(refused());
    }
    Ok(&endpoint[..8 + authority.len()])
}

/// The body RFC 8291 sends: the aes128gcm header, with `from`'s public key, then `message`
/// encrypted for the subscription's key `to` and its `auth` secret.
fn encrypt(
    to: &PublicKey,
    auth: &[u8],
    from: &SecretKey,
    salt: &[u8; 16],
    message: &[u8],
) -> Result<Vec<u8>, String> {
    let shared = p256::ecdh::diffie_hellman(from.to_nonzero_scalar(), to.as_affine());
    let (to, from) = (to.to_sec1_bytes(), from.public_key().to_sec1_bytes());
    let info = [b"WebPush: info\0".as_slice(), &to, &from].concat();
    let ikm: [u8; 32] = expand(auth, shared.raw_secret_bytes(), &info)?;
    let key: [u8; 16] = expand(salt, &ikm, b"Content-Encoding: aes128gcm\0")?;
    let nonce: [u8; 12] = expand(salt, &ikm, b"Content-Encoding: nonce\0")?;
    // One record, so the padding delimiter is 2, the last record's.
    let record = [message, &[2]].concat();
    let sealed = Aes128Gcm::new(&key.into())
        .encrypt(&nonce.into(), record.as_slice())
        .map_err(|_| "the push message could not be encrypted".to_owned())?;
    let size = u32::try_from(RECORD).expect("the record size fits");
    let length = u8::try_from(from.len()).expect("a SEC1 key is under 256 bytes");
    Ok([salt.as_slice(), &size.to_be_bytes(), &[length], &from, &sealed].concat())
}

/// The record size the aes128gcm header names; a push message fits in one record.
const RECORD: usize = 4096;

/// HKDF-SHA-256 of `secret` with `salt`, expanded with `info` to `N` bytes.
fn expand<const N: usize>(salt: &[u8], secret: &[u8], info: &[u8]) -> Result<[u8; N], String> {
    let mut out = [0; N];
    Hkdf::<Sha256>::new(Some(salt), secret)
        .expand(info, &mut out)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64ct::{Base64UrlUnpadded, Encoding};

    fn bytes(text: &str) -> Vec<u8> {
        Base64UrlUnpadded::decode_vec(&text.replace(' ', "")).unwrap()
    }

    #[test]
    fn encryption_matches_the_rfc_8291_example() {
        let to = PublicKey::from_sec1_bytes(&bytes(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        ))
        .unwrap();
        let from =
            SecretKey::from_slice(&bytes("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = bytes("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();

        let body = encrypt(
            &to,
            &bytes("BTBZMqHH6r4Tts7J_aSIgg"),
            &from,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();

        let mut expected = bytes(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml \
             mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        expected.extend(bytes(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs bI_0LpXMuGvnzQ",
        ));
        assert_eq!(body, expected);
    }
}

//! Web Push: the VAPID key pair, the subscriptions, and the notifications the daemon sends for a new
//! ask or an alarm, encrypted as RFC 8291 asks.

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use base64ct::{Base64UrlUnpadded, Encoding};
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use p256::elliptic_curve::array::Array;
use p256::elliptic_curve::consts::U16;
use p256::{PublicKey, SecretKey};
use priory_runtime::runtime;
use rusqlite::{Connection, params};
use sha2::Sha256;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Child;

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
    // Written whole beside it, then linked into place, so a crash never leaves a half key.
    let draft = folder.join(format!("vapid.key.{}", std::process::id()));
    let _ = fs::remove_file(&draft);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&draft)
        .and_then(|mut file| {
            file.write_all(
                Base64UrlUnpadded::encode_string(&SecretKey::generate().to_bytes()).as_bytes(),
            )?;
            file.sync_all()
        })
        .map_err(|error| at(&draft, error))?;
    let linked = fs::hard_link(&draft, &private);
    let _ = fs::remove_file(&draft);
    match linked {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(at(&private, error)),
    }
    let key = private_key(home)?;
    let public = folder.join("vapid.pub");
    fs::write(&public, public_text(&key.public_key())).map_err(|error| at(&public, error))
}

/// The private key `keys` made. An error names the file, never its bytes.
fn private_key(home: &Path) -> Result<SecretKey, String> {
    let private = folder(home).join("vapid.key");
    let text =
        fs::read_to_string(&private).map_err(|error| format!("{}: {error}", private.display()))?;
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

/// The push services browsers subscribe through: Apple, Google (Chrome, Edge on Android),
/// Mozilla and Microsoft. The daemon posts nowhere else, so a subscription cannot aim it at this
/// machine or its network.
const SERVICES: &[&str] = &[
    "push.apple.com",
    "fcm.googleapis.com",
    "push.services.mozilla.com",
    "notify.windows.com",
];

/// The `https://host` origin of a push service's `endpoint`, which is the JWT's audience. Refuses
/// any other scheme, a port, user info, and a host that is not one of `SERVICES` or under it.
fn host(endpoint: &str) -> Result<String, String> {
    let refused = || format!("{endpoint:?} is not an https URL of a known push service");
    let rest = endpoint.strip_prefix("https://").ok_or_else(refused)?;
    let name = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let valid = endpoint.len() <= 2048
        && !endpoint
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.".contains(&byte))
        && SERVICES.iter().any(|service| {
            let name = name.to_ascii_lowercase();
            name == *service || name.ends_with(&format!(".{service}"))
        });
    if !valid {
        return Err(refused());
    }
    Ok(format!("https://{}", name.to_ascii_lowercase()))
}

/// The notification in flight: its row in `push_queue` and one `curl` per subscription.
#[derive(Default)]
pub(crate) struct Sending {
    row: Option<i64>,
    requests: Vec<Request>,
}

struct Request {
    endpoint: String,
    child: Child,
    files: [PathBuf; 2],
}

/// How long a push service keeps a notification for a phone that is off: a day.
const TTL: u32 = 86_400;
/// The VAPID `sub` claim: who runs the sender. Apple refuses some `mailto:` forms, so a URL.
const SUBJECT: &str = "https://github.com/maxence-lefebvre/priory";
/// The longest title a notification carries, in characters.
const TITLE: usize = 120;

/// Sends the oldest queued notification to every subscription, one notification at a time, as the
/// OTLP exporter sends spans. A failed send is logged and dropped: the desk still shows the item.
/// A subscription the push service calls gone (404 or 410) is removed.
pub(crate) fn tick(db: &Connection, home: &Path, sending: &mut Sending) -> rusqlite::Result<()> {
    let mut still = Vec::new();
    for mut request in sending.requests.drain(..) {
        let status = match request.child.try_wait() {
            Ok(None) => {
                still.push(request);
                continue;
            }
            Ok(Some(status)) => Ok(status),
            Err(error) => Err(error),
        };
        let read = |pipe: Option<&mut dyn Read>| {
            let mut text = String::new();
            pipe.map(|pipe| pipe.read_to_string(&mut text));
            text.trim().to_owned()
        };
        let code = read(request.child.stdout.as_mut().map(|pipe| pipe as _));
        let stderr = read(request.child.stderr.as_mut().map(|pipe| pipe as _));
        let origin = host(&request.endpoint).unwrap_or_default();
        match status {
            Ok(status) if status.success() && code.starts_with('2') => {}
            Ok(status) if status.success() && (code == "404" || code == "410") => {
                db.execute(
                    "DELETE FROM push_subscriptions WHERE endpoint = ?1",
                    [&request.endpoint],
                )?;
                eprintln!("priory daemon: {origin} says a subscription is gone, so it is removed");
            }
            Ok(status) if status.success() => {
                eprintln!("priory daemon: {origin} refused a notification with HTTP {code}");
            }
            Ok(status) => {
                eprintln!(
                    "priory daemon: sending a notification to {origin} failed ({status}): {stderr}"
                );
            }
            Err(error) => eprintln!("priory daemon: sending a notification to {origin}: {error}"),
        }
        for file in &request.files {
            let _ = fs::remove_file(file);
        }
    }
    sending.requests = still;
    if !sending.requests.is_empty() {
        return Ok(());
    }
    if let Some(row) = sending.row.take() {
        db.execute("DELETE FROM push_queue WHERE id = ?1", [row])?;
    }
    let Some((row, notice)) = next(db)? else {
        return Ok(());
    };
    sending.row = Some(row);
    let Some(notice) = notice else {
        return Ok(());
    };
    let key = match private_key(home) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("priory daemon: cannot send notifications: {error}");
            return Ok(());
        }
    };
    let mut statement = db.prepare("SELECT endpoint, p256dh, auth FROM push_subscriptions")?;
    let subscriptions = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for (n, subscription) in subscriptions.enumerate() {
        let (endpoint, p256dh, auth) = subscription?;
        let files = [
            folder(home).join(format!("{row}-{n}.headers")),
            folder(home).join(format!("{row}-{n}.body")),
        ];
        match start(&key, &endpoint, &p256dh, &auth, &notice, &files) {
            Ok(child) => sending.requests.push(Request {
                endpoint,
                child,
                files,
            }),
            Err(error) => {
                let origin = host(&endpoint).unwrap_or_default();
                eprintln!("priory daemon: cannot send a notification to {origin}: {error}");
                for file in &files {
                    let _ = fs::remove_file(file);
                }
            }
        }
    }
    Ok(())
}

/// The oldest queued row and its notification as JSON, `{"title", "url"}` and nothing else. The
/// notification is `None` once the ask closed or was asked again, the task left the state that
/// queued it, or a person snoozed the item: those send nothing.
fn next(db: &Connection) -> rusqlite::Result<Option<(i64, Option<String>)>> {
    use rusqlite::OptionalExtension;
    let snoozed = |kind: &str, item: &str| {
        format!(
            "NOT EXISTS (SELECT 1 FROM snoozes WHERE kind = '{kind}' AND item = {item}
             AND back IS NULL AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))"
        )
    };
    let queued = db
        .query_row(
            "SELECT id, kind, item, revision, state FROM push_queue ORDER BY id LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((row, kind, item, revision, state)) = queued else {
        return Ok(None);
    };
    let notice = if kind == "ask" {
        db.query_row(
            &format!(
                // A question can hold a result's summary, so an untitled ask names its task and step.
                "SELECT coalesce(asks.title, 'task ' || tasks.key || ' asks at ' || asks.step,
                   'ask ' || asks.name) FROM asks
                 LEFT JOIN tasks ON tasks.id = asks.task
                 WHERE asks.id = ?1 AND asks.revision IS ?2
                 AND asks.answer IS NULL AND asks.withdrawn IS NULL
                 AND (asks.task IS NULL OR tasks.state NOT IN ('done', 'failed', 'dropped'))
                 AND {}",
                snoozed("ask", "asks.id")
            ),
            params![item, revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|title| (title, format!("/asks/{item}")))
    } else if let Some(kind @ ("report" | "prompt" | "command")) = state.as_deref() {
        crate::listing::alarm(db, kind, item)?.map(|(title, task)| {
            let url = task.map_or_else(|| "/".to_owned(), |task| format!("/tasks/{task}"));
            (title, url)
        })
    } else {
        db.query_row(
            &format!(
                "SELECT 'task ' || key || ' ' || state FROM tasks
                 WHERE id = ?1 AND state IS ?2 AND {}",
                snoozed("task", "tasks.id")
            ),
            params![item, state],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|title| (title, format!("/tasks/{item}")))
    };
    let notice = notice.map(|(title, url)| {
        let title: String = title.chars().take(TITLE).collect();
        serde_json::json!({ "title": title, "url": url }).to_string()
    });
    Ok(Some((row, notice)))
}

/// Writes the request's header and body files, readable by their owner only, and starts `curl`.
fn start(
    key: &SecretKey,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    notice: &str,
    files: &[PathBuf; 2],
) -> Result<Child, String> {
    let origin = host(endpoint)?;
    let to = Base64UrlUnpadded::decode_vec(p256dh)
        .ok()
        .and_then(|point| PublicKey::from_sec1_bytes(&point).ok())
        .ok_or("its p256dh is not a P-256 key")?;
    let auth = Base64UrlUnpadded::decode_vec(auth).map_err(|error| error.to_string())?;
    let salt: [u8; 16] = Array::<u8, U16>::generate().into();
    let body = encrypt(&to, &auth, &SecretKey::generate(), &salt, notice.as_bytes())?;
    let headers = format!(
        "Authorization: vapid t={}, k={}\nContent-Encoding: aes128gcm\n\
         Content-Type: application/octet-stream\nTTL: {TTL}\nUrgency: high\n",
        token(key, &origin)?,
        public_text(&key.public_key()),
    );
    for (file, bytes) in files.iter().zip([headers.as_bytes(), &body]) {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(file)
            .and_then(|mut out| out.write_all(bytes))
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    runtime::post_push(endpoint, &files[0], &files[1]).map_err(|error| error.to_string())
}

/// The VAPID JWT (RFC 8292) for the push service at `origin`, signed with ES256, for 12 hours.
fn token(key: &SecretKey, origin: &str) -> Result<String, String> {
    let exp = jiff::Timestamp::now().as_second() + 12 * 3600;
    let encode =
        |json: serde_json::Value| Base64UrlUnpadded::encode_string(json.to_string().as_bytes());
    let input = format!(
        "{}.{}",
        encode(serde_json::json!({ "typ": "JWT", "alg": "ES256" })),
        encode(serde_json::json!({ "aud": origin, "exp": exp, "sub": SUBJECT })),
    );
    let signature: Signature = SigningKey::from(key).sign(input.as_bytes());
    Ok(format!(
        "{input}.{}",
        Base64UrlUnpadded::encode_string(&signature.to_bytes())
    ))
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
    Ok([
        salt.as_slice(),
        &size.to_be_bytes(),
        &[length],
        &from,
        &sealed,
    ]
    .concat())
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

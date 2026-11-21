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
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

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

/// The requests in flight, one `curl` each, and the retries that wait for their time.
#[derive(Default)]
pub(crate) struct Sending {
    requests: Vec<Request>,
    retries: Vec<Retry>,
    /// Names each request's files, so two tries of one row never share them.
    files: u64,
}

struct Request {
    row: i64,
    endpoint: String,
    tries: u32,
    child: Child,
    files: [PathBuf; 2],
}

struct Retry {
    row: i64,
    endpoint: String,
    tries: u32,
    due: Instant,
}

/// The tries a notification gets at one subscription: the first send and four retries.
const TRIES: u32 = 5;

/// The wait after the `tries`th failed try: 15 s, then four times longer each time, so the last
/// retry goes out about 21 minutes after the first send.
fn backoff(tries: u32) -> Duration {
    Duration::from_secs(15 * 4u64.pow(tries.saturating_sub(1)))
}

/// What one try at one subscription came to.
enum Outcome {
    Sent,
    /// The push service dropped the subscription (404 or 410).
    Gone,
    /// It may pass later: `curl` failed, or the service answered 429 or 5xx.
    Again(String),
    /// It never will, such as a 400 or a 413.
    Refused(String),
}

/// How long a push service keeps a notification for a phone that is off: a day.
const TTL: u32 = 86_400;
/// The VAPID `sub` claim: who runs the sender. Apple refuses some `mailto:` forms, so a URL.
const SUBJECT: &str = "https://github.com/maxence-lefebvre/priory";
/// The longest title a notification carries, in characters.
const TITLE: usize = 120;

/// Starts one `curl` that posts a request's files to a push service.
type Post<'a> = dyn Fn(&str, &Path, &Path) -> std::io::Result<Child> + 'a;

/// Sends the queued notifications, one row at a time, as the OTLP exporter sends spans. A try that
/// may pass later is retried with backoff, up to `TRIES` tries, while later rows go out. A
/// subscription the push service calls gone (404 or 410) is removed. A row leaves the queue once
/// every subscription got it or gave up, so a daemon stopped before then sends it again on its next
/// start, and the service worker drops the copy by its `id`.
pub(crate) fn tick(db: &Connection, home: &Path, sending: &mut Sending) -> rusqlite::Result<()> {
    send(db, home, sending, Instant::now(), &runtime::post_push)
}

fn send(
    db: &Connection,
    home: &Path,
    sending: &mut Sending,
    now: Instant,
    post: &Post<'_>,
) -> rusqlite::Result<()> {
    let mut ended = Vec::new();
    let mut still = Vec::new();
    for mut request in sending.requests.drain(..) {
        let outcome = match request.child.try_wait() {
            Ok(None) => {
                still.push(request);
                continue;
            }
            Ok(Some(status)) => {
                let read = |pipe: Option<&mut dyn Read>| {
                    let mut text = String::new();
                    pipe.map(|pipe| pipe.read_to_string(&mut text));
                    text.trim().to_owned()
                };
                let code = read(request.child.stdout.as_mut().map(|pipe| pipe as _));
                let stderr = read(request.child.stderr.as_mut().map(|pipe| pipe as _));
                outcome(status, &code, &stderr)
            }
            Err(error) => Outcome::Again(error.to_string()),
        };
        for file in &request.files {
            let _ = fs::remove_file(file);
        }
        let origin = host(&request.endpoint).unwrap_or_default();
        match outcome {
            Outcome::Sent => {}
            Outcome::Gone => {
                db.execute(
                    "DELETE FROM push_subscriptions WHERE endpoint = ?1",
                    [&request.endpoint],
                )?;
                eprintln!("priory daemon: {origin} says a subscription is gone, so it is removed");
            }
            Outcome::Refused(why) => {
                eprintln!("priory daemon: {origin} refused a notification: {why}");
            }
            Outcome::Again(why) if request.tries < TRIES => {
                let wait = backoff(request.tries);
                eprintln!(
                    "priory daemon: sending a notification to {origin} failed ({why}); \
                     retrying in {}s",
                    wait.as_secs()
                );
                sending.retries.push(Retry {
                    row: request.row,
                    endpoint: request.endpoint,
                    tries: request.tries,
                    due: now + wait,
                });
            }
            Outcome::Again(why) => {
                eprintln!(
                    "priory daemon: sending a notification to {origin} failed ({why}) \
                     {TRIES} times, so it is dropped"
                );
            }
        }
        ended.push(request.row);
    }
    sending.requests = still;

    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut sending.retries)
        .into_iter()
        .partition(|retry| retry.due <= now);
    sending.retries = waiting;
    for retry in due {
        ended.push(retry.row);
        // The item may have closed, or the subscription gone, while the retry waited.
        let Some(notice) = notice(db, retry.row)? else {
            continue;
        };
        let keys = db
            .query_row(
                "SELECT p256dh, auth FROM push_subscriptions WHERE endpoint = ?1",
                [&retry.endpoint],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((p256dh, auth)) = keys {
            let to = (retry.endpoint, p256dh, auth);
            begin(home, sending, retry.row, retry.tries + 1, &notice, to, post);
        }
    }

    finish(db, sending, ended)?;

    if sending.requests.is_empty()
        && let Some(row) = next(db, sending)?
    {
        if let Some(notice) = notice(db, row)? {
            let mut statement =
                db.prepare("SELECT endpoint, p256dh, auth FROM push_subscriptions")?;
            let subscriptions = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<Vec<(String, String, String)>>>()?;
            for to in subscriptions {
                begin(home, sending, row, 1, &notice, to, post);
            }
        }
        finish(db, sending, vec![row])?;
    }
    Ok(())
}

/// Removes from the queue each of `rows` that no request or retry still sends.
fn finish(db: &Connection, sending: &Sending, mut rows: Vec<i64>) -> rusqlite::Result<()> {
    rows.sort_unstable();
    rows.dedup();
    for row in rows {
        let busy = sending.requests.iter().any(|request| request.row == row)
            || sending.retries.iter().any(|retry| retry.row == row);
        if !busy {
            db.execute("DELETE FROM push_queue WHERE id = ?1", [row])?;
        }
    }
    Ok(())
}

/// What a finished `curl` came to, from its exit status and the HTTP code it wrote.
fn outcome(status: std::process::ExitStatus, code: &str, stderr: &str) -> Outcome {
    if !status.success() {
        return Outcome::Again(format!("{status}: {stderr}"));
    }
    match code {
        _ if code.starts_with('2') => Outcome::Sent,
        "404" | "410" => Outcome::Gone,
        _ if code == "429" || code.starts_with('5') => Outcome::Again(format!("HTTP {code}")),
        _ => Outcome::Refused(format!("HTTP {code}")),
    }
}

/// Starts one try of row `row` at the subscription `to`; a failure to start is logged, not retried.
fn begin(
    home: &Path,
    sending: &mut Sending,
    row: i64,
    tries: u32,
    notice: &str,
    (endpoint, p256dh, auth): (String, String, String),
    post: &Post<'_>,
) {
    let origin = host(&endpoint).unwrap_or_default();
    let key = match private_key(home) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("priory daemon: cannot send notifications: {error}");
            return;
        }
    };
    sending.files += 1;
    let files = [
        folder(home).join(format!("{row}-{}.headers", sending.files)),
        folder(home).join(format!("{row}-{}.body", sending.files)),
    ];
    match start(&key, &endpoint, &p256dh, &auth, notice, &files, post) {
        Ok(child) => sending.requests.push(Request {
            row,
            endpoint,
            tries,
            child,
            files,
        }),
        Err(error) => {
            eprintln!("priory daemon: cannot send a notification to {origin}: {error}");
            for file in &files {
                let _ = fs::remove_file(file);
            }
        }
    }
}

/// The oldest queued row with no retry waiting on it.
fn next(db: &Connection, sending: &Sending) -> rusqlite::Result<Option<i64>> {
    let mut statement = db.prepare("SELECT id FROM push_queue ORDER BY id")?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    for row in rows {
        let row = row?;
        if !sending.retries.iter().any(|retry| retry.row == row) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// Row `row`'s notification as JSON, `{"id", "title", "url"}` and nothing else. `id` names the
/// row, with the time it was queued, so a service worker drops a copy even after a new state file
/// starts its rows at 1 again. The notification is `None` once the row left the queue, the ask
/// closed or was asked again, the task left the state that queued it, or a person snoozed the
/// item: those send nothing.
fn notice(db: &Connection, row: i64) -> rusqlite::Result<Option<String>> {
    let snoozed = |kind: &str, item: &str| {
        format!(
            "NOT EXISTS (SELECT 1 FROM snoozes WHERE kind = '{kind}' AND item = {item}
             AND back IS NULL AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))"
        )
    };
    let queued = db
        .query_row(
            "SELECT kind, item, revision, state, created FROM push_queue WHERE id = ?1",
            [row],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, item, revision, state, created)) = queued else {
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
    Ok(notice.map(|(title, url)| {
        let title: String = title.chars().take(TITLE).collect();
        let id = format!("{created}/{row}");
        serde_json::json!({ "id": id, "title": title, "url": url }).to_string()
    }))
}

/// Writes the request's header and body files, readable by their owner only, and starts `curl`.
fn start(
    key: &SecretKey,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    notice: &str,
    files: &[PathBuf; 2],
    post: &Post<'_>,
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
    post(endpoint, &files[0], &files[1]).map_err(|error| error.to_string())
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

    use std::cell::RefCell;
    use std::process::{Command, Stdio};

    /// A home with a key pair, a state with one subscription and one blocked task, and `rows`
    /// notifications queued for that task.
    fn setup(name: &str, rows: usize) -> (PathBuf, Connection) {
        let home = std::env::temp_dir().join(format!("priory-push-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        keys(&home).unwrap();
        let db = crate::state::memory();
        let to = SecretKey::generate().public_key();
        db.execute(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth) VALUES (?1, ?2, ?3)",
            params![
                "https://fcm.googleapis.com/fcm/send/one",
                public_text(&to),
                Base64UrlUnpadded::encode_string(&[7; 16])
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO tasks
               (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
             VALUES ('cli:a', 'build', 'code', 'blocked', 'now', 'cli', 0, 'cli', 'a',
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )
        .unwrap();
        for _ in 0..rows {
            db.execute(
                "INSERT INTO push_queue (kind, item, state) VALUES ('alarm', 1, 'blocked')",
                [],
            )
            .unwrap();
        }
        (home, db)
    }

    /// A push service that answers each post with the next code, and counts the posts.
    struct Service {
        codes: RefCell<Vec<&'static str>>,
        posts: RefCell<usize>,
    }

    impl Service {
        fn new(codes: &[&'static str]) -> Self {
            Self {
                codes: RefCell::new(codes.iter().rev().copied().collect()),
                posts: RefCell::new(0),
            }
        }

        fn post(&self, _url: &str, _headers: &Path, _body: &Path) -> std::io::Result<Child> {
            *self.posts.borrow_mut() += 1;
            let code = self.codes.borrow_mut().pop().unwrap_or("201");
            Command::new("printf")
                .arg(code)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        }
    }

    /// Ticks at `now` until no request is in flight.
    fn settle(
        db: &Connection,
        home: &Path,
        sending: &mut Sending,
        now: Instant,
        service: &Service,
    ) {
        let post = |url: &str, headers: &Path, body: &Path| service.post(url, headers, body);
        send(db, home, sending, now, &post).unwrap();
        while !sending.requests.is_empty() {
            std::thread::sleep(Duration::from_millis(5));
            send(db, home, sending, now, &post).unwrap();
        }
    }

    fn queued(db: &Connection) -> Vec<i64> {
        let mut statement = db.prepare("SELECT id FROM push_queue ORDER BY id").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_failed_send_is_retried_after_its_backoff_while_later_rows_go_out() {
        let (home, db) = setup("retry", 2);
        let service = Service::new(&["503", "201", "201"]);
        let mut sending = Sending::default();
        let start = Instant::now();

        // Row 1 fails, and row 2 goes out while row 1 waits for its retry.
        settle(&db, &home, &mut sending, start, &service);
        assert_eq!((*service.posts.borrow(), queued(&db)), (2, vec![1]));
        settle(
            &db,
            &home,
            &mut sending,
            start + Duration::from_secs(14),
            &service,
        );
        assert_eq!(*service.posts.borrow(), 2);

        settle(
            &db,
            &home,
            &mut sending,
            start + Duration::from_secs(15),
            &service,
        );
        assert_eq!(*service.posts.borrow(), 3);
        settle(
            &db,
            &home,
            &mut sending,
            start + Duration::from_secs(16),
            &service,
        );
        assert!(queued(&db).is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn retries_stop_after_the_last_try_and_a_refusal_is_not_retried() {
        let (home, db) = setup("give-up", 1);
        let service = Service::new(&["500", "429", "500", "503", "502", "500"]);
        let mut sending = Sending::default();
        let mut now = Instant::now();
        for _ in 0..10 {
            settle(&db, &home, &mut sending, now, &service);
            now += Duration::from_secs(3600);
        }
        assert_eq!(*service.posts.borrow(), TRIES as usize);
        assert!(queued(&db).is_empty());

        db.execute(
            "INSERT INTO push_queue (kind, item, state) VALUES ('alarm', 1, 'blocked')",
            [],
        )
        .unwrap();
        let service = Service::new(&["400"]);
        for _ in 0..3 {
            settle(&db, &home, &mut sending, now, &service);
            now += Duration::from_secs(3600);
        }
        assert_eq!(*service.posts.borrow(), 1);
        assert!(queued(&db).is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_retry_sends_nothing_once_the_item_needs_no_one() {
        let (home, db) = setup("closed", 1);
        let service = Service::new(&["503"]);
        let mut sending = Sending::default();
        let start = Instant::now();
        settle(&db, &home, &mut sending, start, &service);
        db.execute("UPDATE tasks SET state = 'running' WHERE id = 1", [])
            .unwrap();

        settle(
            &db,
            &home,
            &mut sending,
            start + Duration::from_secs(60),
            &service,
        );
        assert_eq!(*service.posts.borrow(), 1);
        assert!(queued(&db).is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn each_notification_carries_an_id_a_service_worker_can_drop_a_copy_by() {
        let (home, db) = setup("id", 2);
        let read = |row| {
            let text = notice(&db, row).unwrap().unwrap();
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let (one, two) = (read(1), read(2));
        assert_eq!(one["title"], "task cli:a blocked");
        assert!(one["id"].as_str().unwrap().ends_with("/1"));
        assert_ne!(one["id"], two["id"]);
        let _ = fs::remove_dir_all(&home);
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

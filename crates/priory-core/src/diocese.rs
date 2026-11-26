//! The diocese's gateway: an HTTP MCP endpoint that a cloud run reaches with a run grant's token.
//! It serves `whoami` only, which proves the connection and acts on nothing.

use herdr_bus::Message;
use priory_factory::factory::Factory;
use priory_intake::intake::{self, OUTPUT};
use priory_intake::log::Log;
use priory_intake::log::sql::GATEWAY_CONSUMER;
use priory_runtime::http::{self, Lane, Reply, Request};
use priory_runtime::mcp;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A runner made a run grant: the token's hash and how long it lasts from when the log took it.
pub const GRANTED: &str = "priory.run.granted";
/// A run grant stops working.
pub const REVOKED: &str = "priory.run.revoked";

/// The one path the gateway serves.
const PATH: &str = "/mcp";
/// The largest request body the gateway reads; a longer one gets 413 before it is read.
const BODY: usize = 64 * 1024;
/// How old the last good read of the log may be before every call gets 503, so a revocation the
/// gateway cannot read never leaves a grant working.
const STALE: Duration = Duration::from_secs(10);
/// The longest tool name an audit line keeps.
const TOOL: usize = 64;
/// Calls a minute one grant may make.
const CALLS: u32 = 60;
/// Refused requests a minute, from anyone: behind the TLS proxy every peer is loopback anyway.
const REFUSALS: u32 = 10;
const MINUTE: Duration = Duration::from_secs(60);

/// Makes a grant that lasts `span`, such as `30m`, publishes its hash on the log, and returns the
/// grant's id and its token as JSON. The token reaches nothing else.
pub fn grant(factory: &Factory, span: &str) -> Result<String, String> {
    let seconds = span
        .parse::<jiff::SignedDuration>()
        .map_err(|error| format!("--for {span}: {error}"))?
        .as_secs();
    if seconds < 1 {
        return Err(format!("--for {span}: a grant lasts at least 1s"));
    }
    let token = crate::serve::new_secret()?;
    let id = uuid::Uuid::now_v7().to_string();
    let data = json!({ "grant": id, "hash": hash(&token), "seconds": seconds });
    crate::runners::publish(factory.log(), &Message::new(GRANTED, data).subject(&id))?;
    Ok(json!({ "grant": id, "token": token }).to_string())
}

/// Publishes that grant `id` stops working.
pub fn revoke(factory: &Factory, id: &str) -> Result<String, String> {
    let data = json!({ "grant": id });
    crate::runners::publish(factory.log(), &Message::new(REVOKED, data).subject(id))?;
    Ok(format!(
        "grant {id} revoked: the gateway refuses it once it reads the log"
    ))
}

/// Serves the gateway on `listen`, a loopback address, until the process ends. It reads the log as
/// `priory.gateway` once before it listens and then each second.
pub fn serve(factory: &Factory, listen: SocketAddr) -> Result<(), String> {
    if !listen.ip().is_loopback() {
        return Err(format!(
            "{listen} is beyond this machine: the gateway listens on loopback, behind a TLS proxy"
        ));
    }
    let home = priory_runtime::runtime::home();
    std::fs::create_dir_all(&home).map_err(|error| format!("{}: {error}", home.display()))?;
    let state = home.join("diocese-gateway.db");
    let db = open(&state)?;
    let log = priory_intake::log::open(factory.log()).map_err(|error| error.to_string())?;
    log.declare(&intake::output())
        .map_err(|error| error.to_string())?;
    let audit = home.join("diocese-gateway.audit.jsonl");
    let audit = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&audit)
        .map_err(|error| format!("{}: {error}", audit.display()))?;
    let gate = Arc::new(Gate {
        audit: Mutex::new(audit),
        calls: Mutex::new(Window::default()),
        refusals: Mutex::new(Window::default()),
        synced: Mutex::new(Instant::now()),
    });
    gate.sync(&*log, &db)?;
    let listener =
        TcpListener::bind(listen).map_err(|error| format!("listen on {listen}: {error}"))?;
    let bound = listener.local_addr().map_err(|error| error.to_string())?;
    // Every path but `PATH` needs this token, which nobody holds.
    let nobody = crate::serve::new_secret()?;
    let served = Arc::clone(&gate);
    let make = move || {
        let (gate, db) = (Arc::clone(&served), open(&state));
        move |request: &Request| match &db {
            Ok(db) => gate.handle(db, request),
            Err(error) => Reply::error(500, error),
        }
    };
    http::serve(
        listener,
        nobody,
        Vec::new(),
        &[PATH],
        BODY,
        // Any method on `PATH` reaches the handler, which answers 405 to all but POST.
        |path| path == PATH,
        |_| Lane::Write,
        |_| Reply::error(404, "no such path"),
        make,
    )
    .map_err(|error| format!("start the server threads: {error}"))?;
    eprintln!("priory: serving on http://{bound}");
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if let Err(error) = gate.sync(&*log, &db) {
            eprintln!("priory gateway: read the log: {error}");
        }
    }
}

fn open(path: &Path) -> Result<Connection, String> {
    let db = Connection::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    db.busy_timeout(Duration::from_secs(5))
        .and_then(|()| {
            db.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS grants (
                     id TEXT PRIMARY KEY,
                     hash TEXT UNIQUE,
                     deadline INTEGER,
                     revoked INTEGER NOT NULL DEFAULT 0
                 );",
            )
        })
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(db)
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Counts per key within the current minute.
#[derive(Default)]
struct Window {
    start: Option<Instant>,
    counts: HashMap<String, u32>,
}

impl Window {
    /// Counts one for `key`, and returns its count this minute.
    fn count(&mut self, key: &str) -> u32 {
        if self.start.is_none_or(|start| start.elapsed() >= MINUTE) {
            self.start = Some(Instant::now());
            self.counts.clear();
        }
        let count = self.counts.entry(key.to_owned()).or_default();
        *count += 1;
        *count
    }
}

struct Gate {
    audit: Mutex<std::fs::File>,
    calls: Mutex<Window>,
    refusals: Mutex<Window>,
    /// When the gateway last read the log to its end.
    synced: Mutex<Instant>,
}

impl Gate {
    /// Records each grant and revocation the log holds past the gateway's cursor, and moves it. A
    /// message it cannot record is skipped with an audit line, so it never blocks the ones after it.
    fn sync(&self, log: &dyn Log, db: &Connection) -> Result<(), String> {
        loop {
            let read = log
                .read(GATEWAY_CONSUMER, OUTPUT, 100)
                .map_err(|error| error.to_string())?;
            if read.is_empty() {
                if let Ok(mut synced) = self.synced.lock() {
                    *synced = Instant::now();
                }
                return Ok(());
            }
            for delivery in &read {
                if let Err(why) = record(db, delivery) {
                    let grant = delivery.message.data["grant"].as_str();
                    let kind = &delivery.message.kind;
                    self.audit(grant, None, None, &format!("skipped {kind}: {why}"));
                }
                log.ack(GATEWAY_CONSUMER, delivery)
                    .map_err(|error| error.to_string())?;
            }
        }
    }

    fn handle(&self, db: &Connection, request: &Request) -> Reply {
        if self
            .synced
            .lock()
            .is_ok_and(|synced| synced.elapsed() > STALE)
        {
            return Reply::error(503, "the gateway cannot read the log: retry in a minute");
        }
        if request.method != "POST" {
            return Reply::error(405, "POST a JSON-RPC message to /mcp");
        }
        let grant = match self.grant(db, request) {
            Ok(grant) => grant,
            Err(why) => {
                let count = self.refusals.lock().map_or(u32::MAX, |mut w| w.count(""));
                // Past the limit, one line says so and the rest go unwritten.
                match count {
                    ..=REFUSALS => self.audit(None, None, Some(401), &why),
                    count if count == REFUSALS + 1 => {
                        let over = format!("over {REFUSALS} refusals a minute: {why}");
                        self.audit(None, None, Some(429), &over);
                    }
                    _ => {}
                }
                let status = if count <= REFUSALS { 401 } else { 429 };
                return limited(status, &why);
            }
        };
        let id = grant.id.as_str();
        if self.calls.lock().map_or(u32::MAX, |mut w| w.count(id)) > CALLS {
            let why = format!("over {CALLS} calls a minute");
            self.audit(Some(id), None, Some(429), &why);
            return limited(429, &why);
        }
        let line = String::from_utf8_lossy(request.body);
        let message: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let tool = match message["method"].as_str() {
            Some("tools/call") => message["params"]["name"].as_str(),
            method => method,
        };
        let deadline = jiff::Timestamp::from_second(grant.deadline)
            .map_or_else(|_| grant.deadline.to_string(), |at| at.to_string());
        let call = |params: &Value| match params["name"].as_str() {
            Some("whoami") => Ok(json!({ "content": [{ "type": "text", "text":
                format!("run grant {id}, valid until {deadline}: the gateway reached Priory") }] })),
            _ => Err("the gateway serves whoami only".to_owned()),
        };
        let (status, body, outcome) =
            match mcp::respond(&line, &json!([whoami()]), INSTRUCTIONS, &call) {
                None => (202, String::new(), "notification".to_owned()),
                Some(reply) => {
                    let outcome = reply["error"]["message"]
                        .as_str()
                        .map_or_else(|| "served".to_owned(), |error| format!("error: {error}"));
                    (200, reply.to_string(), outcome)
                }
            };
        self.audit(Some(id), tool, Some(status), &outcome);
        Reply::json(status, body)
    }

    /// The grant the request's bearer token names, while it is live, or why it is refused.
    fn grant(&self, db: &Connection, request: &Request) -> Result<Grant, String> {
        let token = request
            .header("authorization")
            .and_then(|value| value.get(..7).zip(value.get(7..)))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(b"bearer "))
            .and_then(|(_, token)| std::str::from_utf8(token).ok())
            .ok_or("send the run grant's token as `Authorization: Bearer <token>`")?;
        let found = db
            .query_row(
                "SELECT id, deadline, revoked FROM grants WHERE hash = ?1",
                [hash(token)],
                |row| {
                    Ok((
                        Grant {
                            id: row.get(0)?,
                            deadline: row.get(1)?,
                        },
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((grant, revoked)) = found else {
            return Err("no run grant has this token".to_owned());
        };
        if revoked {
            return Err(format!("run grant {} is revoked", grant.id));
        }
        if intake::now()?.as_second() >= grant.deadline {
            return Err(format!("run grant {} has expired", grant.id));
        }
        Ok(grant)
    }

    /// Appends one line to the audit log: never the token or a call's arguments. `status` is the
    /// HTTP status of a request, and none for a message of the log it skipped.
    fn audit(&self, grant: Option<&str>, tool: Option<&str>, status: Option<u16>, outcome: &str) {
        let time = intake::now().map(|now| now.to_string()).unwrap_or_default();
        let tool = tool.map(|tool| tool.chars().take(TOOL).collect::<String>());
        let line = json!({ "time": time, "grant": grant, "tool": tool, "status": status,
            "outcome": outcome });
        if let Ok(mut file) = self.audit.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

struct Grant {
    id: String,
    deadline: i64,
}

/// Records one grant or revocation in the gateway's state file, or says why it cannot.
fn record(db: &Connection, delivery: &priory_intake::log::Delivery) -> Result<(), String> {
    let kind = delivery.message.kind.as_str();
    if kind != GRANTED && kind != REVOKED {
        return Ok(());
    }
    let data = &delivery.message.data;
    let id = data["grant"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("it names no grant")?;
    let recorded = match kind {
        GRANTED => {
            let hash = data["hash"]
                .as_str()
                .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("its hash is no SHA-256 in hex")?;
            let seconds = data["seconds"]
                .as_i64()
                .filter(|seconds| *seconds > 0)
                .ok_or("its seconds are no positive integer")?;
            // The log's clock when it has one, so a runner's clock decides no deadline.
            let from = delivery.logged.unwrap_or(delivery.message.time);
            db.execute(
                "INSERT INTO grants (id, hash, deadline) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET hash = ?2, deadline = ?3",
                params![id, hash, from.as_second().saturating_add(seconds)],
            )
        }
        _ => db.execute(
            "INSERT INTO grants (id, revoked) VALUES (?1, 1)
             ON CONFLICT (id) DO UPDATE SET revoked = 1",
            [id],
        ),
    };
    recorded.map(|_| ()).map_err(|error| error.to_string())
}

fn limited(status: u16, why: &str) -> Reply {
    let mut reply = Reply::error(status, why);
    if status == 429 {
        reply.headers = "Retry-After: 60\r\n".into();
    }
    reply
}

const INSTRUCTIONS: &str = "Priory's diocese gateway. whoami says which run grant signed in.";

fn whoami() -> Value {
    json!({
        "name": "whoami",
        "description": "Says which run grant this session signed in with, and until when. Acts on nothing.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_bus::{Output, Seq, Stored};
    use priory_intake::log::{Delivery, Result as LogResult};

    /// A log whose every read fails, as a Postgres server that went away.
    struct Down;

    impl Log for Down {
        fn declare(&self, _: &Output) -> LogResult<()> {
            Ok(())
        }
        fn publish(&self, _: &str, _: &Message) -> LogResult<Seq> {
            Err("down".into())
        }
        fn message(&self, _: Seq) -> LogResult<Stored> {
            Err("down".into())
        }
        fn messages(&self, _: &str, _: Option<Seq>, _: usize) -> LogResult<Vec<Stored>> {
            Err("down".into())
        }
        fn read(&self, _: &str, _: &str, _: usize) -> LogResult<Vec<Delivery>> {
            Err("down".into())
        }
        fn ack(&self, _: &str, _: &Delivery) -> LogResult<()> {
            Err("down".into())
        }
        fn backlog(&self, _: &str) -> LogResult<Vec<(String, u64)>> {
            Err("down".into())
        }
    }

    #[test]
    fn a_gateway_that_cannot_read_the_log_for_over_10s_answers_503() {
        let db = Connection::open_in_memory().unwrap();
        let gate = Gate {
            audit: Mutex::new(std::fs::File::create("/dev/null").unwrap()),
            calls: Mutex::new(Window::default()),
            refusals: Mutex::new(Window::default()),
            synced: Mutex::new(Instant::now()),
        };
        let request = Request::get(PATH, Instant::now());
        assert!(gate.sync(&Down, &db).is_err());
        assert_eq!(gate.handle(&db, &request).status, 405);

        *gate.synced.lock().unwrap() = Instant::now() - STALE - Duration::from_secs(1);
        assert!(gate.sync(&Down, &db).is_err());
        assert_eq!(gate.handle(&db, &request).status, 503);
    }
}

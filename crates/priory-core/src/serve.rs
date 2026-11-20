//! The HTTP server over the state: `/api` as JSON, with `web` the web view's files, and with a
//! GitHub webhook its signed deliveries.

use crate::{listing, state};
use herdr_bus::Bus;
use priory_factory::factory::Factory;
use priory_intake::intake::Github;
use priory_runtime::http::{self, Reply, Request};
use priory_runtime::runtime;
use rusqlite::Connection;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const GITHUB: &str = "/hooks/github";
const INDEX: &[u8] = include_bytes!("../web/index.html");
/// How long `/api/events` holds a request without a change, at most and by default.
const WAIT: u64 = 25;
/// Requests `/api/events` holds at once: half the workers, so listings always find a free one.
const WAITERS: usize = http::WORKERS / 2;

/// Listens on `listen` and serves the state from a pool of threads, each with its own read-only
/// connection. Refuses to start without the token or a state file it can read.
pub fn start(factory: &Factory, listen: SocketAddr) -> Result<Vec<JoinHandle<()>>, String> {
    let config = factory
        .serve()
        .ok_or("factory.toml has no [serve] table, so nothing listens")?;
    config.allows(listen)?;
    let token = env(&config.token_env, "the token clients send")?;
    let github = factory
        .github_webhook()
        .map(|hook| env(&hook.secret_env, "the GitHub webhook's secret"))
        .transpose()?
        .map(|secret| Github::new(secret, factory))
        .transpose()?
        .map(Arc::new);
    drop(state::read_only()?);
    let listener =
        TcpListener::bind(listen).map_err(|error| format!("listen on {listen}: {error}"))?;
    let bound = listener
        .local_addr()
        .map_err(|error| format!("listen on {listen}: {error}"))?;
    let web = config.web;
    let hooks: &[&str] = if github.is_some() { &[GITHUB] } else { &[] };
    let waiters = Arc::new(AtomicUsize::new(0));
    let workers = http::serve(listener, token, hooks, move || {
        let db = state::read_only();
        let waiters = Arc::clone(&waiters);
        let home = runtime::home();
        let github = github.clone().map(|github| {
            (
                github,
                Bus::open_default().map_err(|error| error.to_string()),
            )
        });
        move |request: &Request| match &github {
            Some((github, bus))
                if request.method == "POST" && request.path.split('?').next() == Some(GITHUB) =>
            {
                github.receive(bus, request)
            }
            _ => route(&db, &home, web, &waiters, request.method, request.path),
        }
    })
    .map_err(|error| format!("start the server threads: {error}"))?;
    eprintln!("priory: serving on http://{bound}");
    Ok(workers)
}

/// The value of the environment variable `name`, refused when unset or empty.
fn env(name: &str, what: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is unset or empty: set it to {what}"))
}

fn route(
    db: &Result<Connection, String>,
    home: &Path,
    web: bool,
    waiters: &AtomicUsize,
    method: &str,
    path: &str,
) -> Reply {
    if method != "GET" {
        return Reply::error(405, "only GET is served");
    }
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    if web && matches!(path, "/" | "/index.html") {
        return Reply {
            status: 200,
            kind: "text/html; charset=utf-8",
            body: INDEX.to_vec(),
        };
    }
    let db = match db {
        Ok(db) => db,
        Err(error) => return Reply::error(500, error),
    };
    let read = match path {
        "/api/events" => return events(db, query, waiters),
        "/api/desk" => listing::revised(db, "desk", |db| listing::desk(db, home, false).map(Some)),
        "/api/inbox" => listing::revised(db, "inbox", |db| listing::inbox(db, 0).map(Some)),
        "/api/asks" => listing::revised(db, "asks", |db| listing::asks(db).map(Some)),
        "/api/tasks" => {
            listing::revised(db, "tasks", |db| listing::tasks(db, None, None).map(Some))
        }
        "/api/queue" => listing::revised(db, "queue", |db| listing::queue(db).map(Some)),
        _ => match path.strip_prefix("/api/tasks/").map(str::parse) {
            Some(Ok(id)) => listing::revised(db, "task", |db| listing::task(db, id)),
            _ => return Reply::error(404, "no such path"),
        },
    };
    match read {
        Ok(Some(body)) => Reply::json(200, body.to_string()),
        Ok(None) => Reply::error(404, "no such task"),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

/// `/api/events?since=REV[&timeout=SECONDS]`: the revision once it differs from `since`, or the
/// same one after the timeout. Refuses a request past `WAITERS` held at once with `503`.
fn events(db: &Connection, query: &str, waiters: &AtomicUsize) -> Reply {
    let (mut since, mut timeout) = (None, Some(WAIT));
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        match pair.split_once('=') {
            Some(("since", value)) => since = value.parse::<i64>().ok(),
            Some(("timeout", value)) => {
                timeout = value.parse().ok().filter(|seconds| *seconds <= WAIT)
            }
            _ => return Reply::error(400, "events takes since=REV and timeout=SECONDS"),
        }
    }
    let (Some(since), Some(timeout)) = (since, timeout) else {
        return Reply::error(
            400,
            "events needs since=REV, and timeout=SECONDS of at most 25",
        );
    };
    let held = waiters.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |held| {
        (held < WAITERS).then_some(held + 1)
    });
    if held.is_err() {
        return Reply::error(503, "too many requests wait on events: retry in a second");
    }
    let waited = state::wait(db, since, Duration::from_secs(timeout));
    waiters.fetch_sub(1, Ordering::SeqCst);
    match waited {
        Ok(revision) => Reply::json(200, serde_json::json!({ "revision": revision }).to_string()),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

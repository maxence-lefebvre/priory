//! The HTTP server over the state: `/api` as JSON and, with `web`, the web view's files.

use crate::{listing, state};
use priory_factory::factory::Serve;
use priory_runtime::http::{self, Reply};
use priory_runtime::runtime;
use rusqlite::Connection;
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::thread::JoinHandle;

const INDEX: &[u8] = include_bytes!("../web/index.html");

/// Listens on `listen` and serves the state from a pool of threads, each with its own read-only
/// connection. Refuses to start without the token.
pub fn start(config: &Serve, listen: SocketAddr) -> Result<Vec<JoinHandle<()>>, String> {
    config.allows(listen)?;
    let name = &config.token_env;
    let token = std::env::var(name)
        .ok()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("{name} is unset or empty: set it to the token clients send"))?;
    drop(state::open()?);
    let listener =
        TcpListener::bind(listen).map_err(|error| format!("listen on {listen}: {error}"))?;
    let bound = listener
        .local_addr()
        .map_err(|error| format!("listen on {listen}: {error}"))?;
    let web = config.web;
    let workers = http::serve(listener, token, move || {
        let db = state::read_only();
        let home = runtime::home();
        move |method: &str, path: &str| route(&db, &home, web, method, path)
    })
    .map_err(|error| format!("start the server threads: {error}"))?;
    eprintln!("priory: serving on http://{bound}");
    Ok(workers)
}

fn route(
    db: &Result<Connection, String>,
    home: &Path,
    web: bool,
    method: &str,
    path: &str,
) -> Reply {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if web && matches!(path, "/" | "/index.html") {
        if method != "GET" {
            return Reply::error(405, "only GET is served");
        }
        return Reply {
            status: 200,
            kind: "text/html; charset=utf-8",
            body: INDEX.to_vec(),
        };
    }
    let Some(read) = endpoint(path) else {
        return Reply::error(404, "no such path");
    };
    if method != "GET" {
        return Reply::error(405, "only GET is served");
    }
    let db = match db {
        Ok(db) => db,
        Err(error) => return Reply::error(500, error),
    };
    match read(db, home) {
        Ok(Some(body)) => Reply::json(200, body.to_string()),
        Ok(None) => Reply::error(404, "no such task"),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

type Read = Box<dyn Fn(&Connection, &Path) -> rusqlite::Result<Option<Value>>>;

fn endpoint(path: &str) -> Option<Read> {
    let found: Read = match path {
        "/api/desk" => Box::new(|db, home| listing::desk(db, home).map(Some)),
        "/api/inbox" => Box::new(|db, _| listing::inbox(db).map(Some)),
        "/api/asks" => Box::new(|db, _| listing::asks(db).map(Some)),
        "/api/tasks" => Box::new(|db, _| listing::tasks(db, None).map(Some)),
        "/api/queue" => Box::new(|db, _| listing::queue(db).map(Some)),
        _ => {
            let id: i64 = path.strip_prefix("/api/tasks/")?.parse().ok()?;
            Box::new(move |db, _| listing::task(db, id))
        }
    };
    Some(found)
}

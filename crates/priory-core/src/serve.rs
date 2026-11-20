//! The HTTP server over the state: `/api` as JSON and, with `web`, the web view's files.

use crate::{listing, state};
use priory_factory::factory::Serve;
use priory_runtime::http::{self, Reply};
use priory_runtime::runtime;
use rusqlite::Connection;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::thread::JoinHandle;

const INDEX: &[u8] = include_bytes!("../web/index.html");

/// Listens on `listen` and serves the state from a pool of threads, each with its own read-only
/// connection. Refuses to start without the token or a state file it can read.
pub fn start(config: &Serve, listen: SocketAddr) -> Result<Vec<JoinHandle<()>>, String> {
    config.allows(listen)?;
    let name = &config.token_env;
    let token = std::env::var(name)
        .ok()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("{name} is unset or empty: set it to the token clients send"))?;
    drop(state::read_only()?);
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
    if method != "GET" {
        return Reply::error(405, "only GET is served");
    }
    let path = path.split_once('?').map_or(path, |(path, _)| path);
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
        "/api/desk" => listing::desk(db, home).map(Some),
        "/api/inbox" => listing::inbox(db).map(Some),
        "/api/asks" => listing::asks(db).map(Some),
        "/api/tasks" => listing::tasks(db, None, None).map(Some),
        "/api/queue" => listing::queue(db).map(Some),
        _ => match path.strip_prefix("/api/tasks/").map(str::parse) {
            Some(Ok(id)) => listing::task(db, id),
            _ => return Reply::error(404, "no such path"),
        },
    };
    match read {
        Ok(Some(body)) => Reply::json(200, body.to_string()),
        Ok(None) => Reply::error(404, "no such task"),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

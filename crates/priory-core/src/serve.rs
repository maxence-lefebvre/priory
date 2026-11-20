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
use std::thread::JoinHandle;

const GITHUB: &str = "/hooks/github";
const INDEX: &[u8] = include_bytes!("../web/index.html");

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
    let workers = http::serve(listener, token, hooks, move || {
        let db = state::read_only();
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
            _ => route(&db, &home, web, request.method, request.path),
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
        "/api/desk" => listing::desk(db, home, false).map(Some),
        "/api/inbox" => listing::inbox(db, 0).map(Some),
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

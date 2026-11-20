//! The HTTP server over the state: `/api` as JSON and, with `web`, the web view's files.

use crate::{listing, state};
use priory_factory::factory::{Factory, Serve};
use priory_runtime::http::{self, Reply};
use priory_runtime::runtime;
use rusqlite::Connection;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::thread::JoinHandle;

const INDEX: &[u8] = include_bytes!("../web/index.html");

macro_rules! file {
    ($path:literal, $kind:expr, $source:literal) => {
        ($path, $kind, include_bytes!(concat!("../../../", $source)).as_slice())
    };
}

const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const WOFF2: &str = "font/woff2";

/// The web view's files: the path each is served at, its type and its bytes.
const WEB: &[(&str, &str, &[u8])] = &[
    file!("/boot.js", JS, "crates/priory-core/web/boot.js"),
    file!("/app.js", JS, "crates/priory-core/web/app.js"),
    file!("/app.css", CSS, "crates/priory-core/web/app.css"),
    file!("/tokens/structure.css", CSS, "design/tokens/structure.css"),
    file!("/tokens/themes.css", CSS, "design/tokens/themes.css"),
    file!("/favicon.ico", "image/x-icon", "design/logo/favicon.ico"),
    file!("/favicon.svg", "image/svg+xml", "design/logo/favicon.svg"),
    file!("/apple-touch-icon.png", "image/png", "design/logo/apple-touch-icon.png"),
    file!("/fonts/mona-sans-200-900.woff2", WOFF2, "design/fonts/mona-sans-200-900.woff2"),
    file!("/fonts/jetbrains-mono-400-500.woff2", WOFF2, "design/fonts/jetbrains-mono-400-500.woff2"),
    file!("/fonts/eb-garamond-400-800.woff2", WOFF2, "design/fonts/eb-garamond-400-800.woff2"),
    file!("/fonts/ibm-plex-sans-400-700.woff2", WOFF2, "design/fonts/ibm-plex-sans-400-700.woff2"),
    file!("/fonts/ibm-plex-mono-400.woff2", WOFF2, "design/fonts/ibm-plex-mono-400.woff2"),
    file!("/fonts/ibm-plex-mono-500.woff2", WOFF2, "design/fonts/ibm-plex-mono-500.woff2"),
    file!("/fonts/alegreya-400-900.woff2", WOFF2, "design/fonts/alegreya-400-900.woff2"),
    file!("/fonts/alegreya-sans-400.woff2", WOFF2, "design/fonts/alegreya-sans-400.woff2"),
    file!("/fonts/alegreya-sans-500.woff2", WOFF2, "design/fonts/alegreya-sans-500.woff2"),
    file!("/fonts/alegreya-sans-700.woff2", WOFF2, "design/fonts/alegreya-sans-700.woff2"),
    file!("/fonts/alegreya-sans-800.woff2", WOFF2, "design/fonts/alegreya-sans-800.woff2"),
];

/// Agent text reaches the page, so the page runs and loads only its own files.
const WEB_HEADERS: &str = "Content-Security-Policy: default-src 'none'; script-src 'self'; \
    style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\n\
    Referrer-Policy: no-referrer\r\n";

/// Listens on `listen` and serves the state from a pool of threads, each with its own read-only
/// connection. Refuses to start without the token or a state file it can read.
pub fn start(
    factory: &Factory,
    config: &Serve,
    listen: SocketAddr,
) -> Result<Vec<JoinHandle<()>>, String> {
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
    let views = serde_json::json!({ "factory": factory.name, "views": factory.views() }).to_string();
    let public = |path: &str| !path.starts_with("/api");
    let workers = http::serve(listener, token, public, move || {
        let db = state::read_only();
        let home = runtime::home();
        let views = views.clone();
        move |method: &str, path: &str| route(&db, &home, &views, web, method, path)
    })
    .map_err(|error| format!("start the server threads: {error}"))?;
    eprintln!("priory: serving on http://{bound}");
    Ok(workers)
}

fn route(
    db: &Result<Connection, String>,
    home: &Path,
    views: &str,
    web: bool,
    method: &str,
    path: &str,
) -> Reply {
    if method != "GET" {
        return Reply::error(405, "only GET is served");
    }
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if web && !path.starts_with("/api") {
        return web_file(path);
    }
    if path == "/api/views" {
        return Reply::json(200, views.to_owned());
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

/// A web view file, or the page itself for any path without an extension, such as `/asks/42`.
fn web_file(path: &str) -> Reply {
    let found = WEB.iter().find(|(served, _, _)| *served == path);
    let (kind, body) = match found {
        Some((_, kind, body)) => (*kind, *body),
        None if path.rsplit('/').next().is_some_and(|name| name.contains('.'))
            && path != "/index.html" =>
        {
            return Reply::error(404, "no such file");
        }
        None => ("text/html; charset=utf-8", INDEX),
    };
    Reply {
        status: 200,
        kind,
        headers: WEB_HEADERS,
        body: body.to_vec(),
    }
}

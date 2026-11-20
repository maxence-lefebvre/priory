//! The HTTP server over the state: `/api` as JSON, with `web` the web view's files, and with a
//! GitHub webhook its signed deliveries.

use crate::{listing, sections, snooze, state, tools};
use herdr_bus::Bus;
use priory_factory::factory::{Factory, Trigger};
use priory_intake::intake::{self, Github};
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

macro_rules! file {
    ($path:literal, $kind:expr, $source:literal) => {
        (
            $path,
            $kind,
            include_bytes!(concat!("../../../", $source)).as_slice(),
        )
    };
}

const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const WOFF2: &str = "font/woff2";

/// The web view's files: the path each is served at, its type and its bytes.
const WEB: &[(&str, &str, &[u8])] = &[
    file!("/boot.js", JS, "crates/priory-core/web/boot.js"),
    file!("/app.js", JS, "crates/priory-core/web/app.js"),
    file!("/asks.js", JS, "crates/priory-core/web/asks.js"),
    file!("/app.css", CSS, "crates/priory-core/web/app.css"),
    file!("/tokens/structure.css", CSS, "design/tokens/structure.css"),
    file!("/tokens/themes.css", CSS, "design/tokens/themes.css"),
    file!("/favicon.ico", "image/x-icon", "design/logo/favicon.ico"),
    file!("/favicon.svg", "image/svg+xml", "design/logo/favicon.svg"),
    file!(
        "/apple-touch-icon.png",
        "image/png",
        "design/logo/apple-touch-icon.png"
    ),
    file!(
        "/fonts/mona-sans-200-900.woff2",
        WOFF2,
        "design/fonts/mona-sans-200-900.woff2"
    ),
    file!(
        "/fonts/jetbrains-mono-400-500.woff2",
        WOFF2,
        "design/fonts/jetbrains-mono-400-500.woff2"
    ),
    file!(
        "/fonts/eb-garamond-400-800.woff2",
        WOFF2,
        "design/fonts/eb-garamond-400-800.woff2"
    ),
    file!(
        "/fonts/ibm-plex-sans-400-700.woff2",
        WOFF2,
        "design/fonts/ibm-plex-sans-400-700.woff2"
    ),
    file!(
        "/fonts/ibm-plex-mono-400.woff2",
        WOFF2,
        "design/fonts/ibm-plex-mono-400.woff2"
    ),
    file!(
        "/fonts/ibm-plex-mono-500.woff2",
        WOFF2,
        "design/fonts/ibm-plex-mono-500.woff2"
    ),
    file!(
        "/fonts/alegreya-400-900.woff2",
        WOFF2,
        "design/fonts/alegreya-400-900.woff2"
    ),
    file!(
        "/fonts/alegreya-sans-400.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-400.woff2"
    ),
    file!(
        "/fonts/alegreya-sans-500.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-500.woff2"
    ),
    file!(
        "/fonts/alegreya-sans-700.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-700.woff2"
    ),
    file!(
        "/fonts/alegreya-sans-800.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-800.woff2"
    ),
];

/// Agent text reaches the page, so the page runs and loads only its own files.
const WEB_HEADERS: &str = "Content-Security-Policy: default-src 'none'; script-src 'self'; \
    style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\n\
    Referrer-Policy: no-referrer\r\n";

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
    let served = Arc::new(Served {
        views: serde_json::json!({ "factory": factory.name, "views": factory.views() }).to_string(),
        schedules: listing::schedule_triggers(factory),
    });
    let hooks: &[&str] = if github.is_some() { &[GITHUB] } else { &[] };
    let waiters = Arc::new(AtomicUsize::new(0));
    let public = |path: &str| !path.starts_with("/api") && !path.starts_with("/hooks");
    let workers = http::serve(listener, token, hooks, public, move || {
        let db = state::read_only();
        let waiters = Arc::clone(&waiters);
        let home = runtime::home();
        let served = served.clone();
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
            _ if request.method == "POST" => write(&db, &home, request),
            _ => route(
                &db,
                &home,
                &served,
                web,
                &waiters,
                request.method,
                request.path,
            ),
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

/// What every server thread reads from the factory, once, when the server starts.
struct Served {
    views: String,
    schedules: Vec<(String, Trigger)>,
}

fn route(
    db: &Result<Connection, String>,
    home: &Path,
    served: &Served,
    web: bool,
    waiters: &AtomicUsize,
    method: &str,
    path: &str,
) -> Reply {
    if method != "GET" {
        return Reply::error(405, "only GET is served");
    }
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    if Write::of(path).is_some() {
        return Reply {
            headers: "Allow: POST\r\n",
            ..Reply::error(405, "only POST is served")
        };
    }
    if web && !path.starts_with("/api") && !path.starts_with("/hooks") {
        return web_file(path);
    }
    if path == "/api/views" {
        return Reply::json(200, served.views.clone());
    }
    let db = match db {
        Ok(db) => db,
        Err(error) => return Reply::error(500, error),
    };
    let read = match path {
        "/api/events" => return events(db, query, waiters),
        "/api/desk" => {
            let person = match query.split_once('=') {
                None if query.is_empty() => listing::ME.to_owned(),
                Some(("person", name)) if !name.contains('&') => match decode(name) {
                    Some(name) => name,
                    None => return Reply::error(400, "person is not percent-encoded UTF-8"),
                },
                _ => return Reply::error(400, "desk takes one parameter: person=NAME"),
            };
            listing::revised(db, "desk", |db| {
                listing::desk(db, home, false, &person).map(Some)
            })
        }
        "/api/inbox" => listing::revised(db, "inbox", |db| listing::inbox(db, 0).map(Some)),
        "/api/asks" => {
            let (mut bands, mut all) = (Vec::new(), false);
            for pair in query.split('&').filter(|pair| !pair.is_empty()) {
                match pair.split_once('=') {
                    Some(("all", "1")) => all = true,
                    Some(("band", value)) => match decode(value) {
                        Some(value) => bands.extend(value.split(',').map(str::to_owned)),
                        None => return Reply::error(400, "band is not percent-encoded UTF-8"),
                    },
                    _ => {
                        return Reply::error(400, "asks takes band=BAND[,BAND...] and all=1");
                    }
                }
            }
            for band in &bands {
                if let Err(refusal) = state::check_band(db, band) {
                    return Reply::error(400, &refusal);
                }
            }
            let since = match week_ago() {
                _ if all => None,
                Ok(since) => since,
                Err(reply) => return reply,
            };
            let bands: Vec<&str> = bands.iter().map(String::as_str).collect();
            listing::revised(db, "asks", |db| {
                listing::recent_asks(db, &bands, since.as_deref()).map(Some)
            })
        }
        "/api/tasks" => match since(query) {
            Ok(since) => listing::revised(db, "tasks", |db| {
                listing::recent_tasks(db, since.as_deref()).map(Some)
            }),
            Err(reply) => return reply,
        },
        "/api/queue" => match intake::now() {
            Ok(now) => listing::revised(db, "queue", |db| {
                listing::queue(db, &served.schedules, now).map(Some)
            }),
            Err(error) => return Reply::error(500, &error),
        },
        "/api/reports/latest" => listing::revised(db, "report", |db| sections::show(db, None)),
        _ => match (
            path.strip_prefix("/api/tasks/").map(str::parse),
            path.strip_prefix("/api/reports/").map(str::parse),
        ) {
            (Some(Ok(id)), _) => listing::revised(db, "task", |db| listing::task(db, id)),
            (_, Some(Ok(task))) => {
                listing::revised(db, "report", |db| sections::show(db, Some(task)))
            }
            _ => return Reply::error(404, "no such path"),
        },
    };
    match read {
        Ok(Some(body)) => Reply::json(200, body.to_string()),
        Ok(None) if path.starts_with("/api/reports/") => Reply::error(404, "no such report"),
        Ok(None) => Reply::error(404, "no such task"),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

/// A path that writes, each the twin of a CLI verb. An ask is named by its number or, for an
/// outside ask, its name.
enum Write<'a> {
    Answer(&'a str),
    Dismiss(&'a str),
    Snooze,
}

impl<'a> Write<'a> {
    fn of(path: &'a str) -> Option<Self> {
        if path == "/api/snooze" {
            return Some(Write::Snooze);
        }
        match path.strip_prefix("/api/asks/")?.split_once('/')? {
            (ask, "answer") => Some(Write::Answer(ask)),
            (ask, "dismiss") => Some(Write::Dismiss(ask)),
            _ => None,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBody {
    option: Option<String>,
    text: Option<String>,
    revision: i64,
    #[serde(default)]
    run: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DismissBody {
    revision: i64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SnoozeBody {
    kind: String,
    id: i64,
    #[serde(rename = "for")]
    span: Option<String>,
    until: Option<String>,
    #[serde(default)]
    clear: bool,
}

/// Sends a write to the daemon on its socket, as the CLI verb does; this thread never writes the
/// state. A refusal re-reads the ask to tell a stale or closed ask (`409`) from a refused one.
fn write(db: &Result<Connection, String>, home: &Path, request: &Request) -> Reply {
    let path = request.path.split('?').next().unwrap_or_default();
    let Some(write) = Write::of(path) else {
        return Reply::error(405, "only GET is served");
    };
    let json = request
        .header("content-type")
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return Reply::error(415, "send the body as Content-Type: application/json");
    }
    let db = match db {
        Ok(db) => db,
        Err(error) => return Reply::error(500, error),
    };
    let (line, ask) = match write_line(db, &write, request.body) {
        Ok(line) => line,
        Err(reply) => return reply,
    };
    let reply = match runtime::call(home, &line) {
        Ok(reply) => reply,
        // Only a refused connection proves the line never reached the daemon.
        Err(error) if error.starts_with(runtime::NO_DAEMON) => {
            return Reply::error(
                503,
                &format!("{error}. Start `priory daemon` to write; reads still work"),
            );
        }
        Err(error) => {
            return Reply::error(
                504,
                &format!(
                    "{error}. The daemon may have applied it: read the state before you retry"
                ),
            );
        }
    };
    match serde_json::from_str::<Result<String, String>>(&reply) {
        Ok(Ok(done)) => Reply::json(200, serde_json::json!({ "ok": done }).to_string()),
        Ok(Err(refused)) => refusal(db, ask, &refused),
        Err(error) => Reply::error(500, &format!("unreadable reply {reply:?}: {error}")),
    }
}

/// The socket line of `write` and, for an ask, its number and the revision the body sent.
fn write_line(
    db: &Connection,
    write: &Write,
    body: &[u8],
) -> Result<(String, Option<(i64, i64)>), Reply> {
    let bad = |error: serde_json::Error| Reply::error(400, &error.to_string());
    match write {
        Write::Answer(ask) => {
            let body: AnswerBody = serde_json::from_slice(body).map_err(bad)?;
            let text = match (body.option, body.text) {
                (Some(text), None) | (None, Some(text)) => text,
                _ => return Err(Reply::error(400, "send one of option and text")),
            };
            let ask = ask_id(db, ask)?;
            let revision = body.revision;
            let answer = tools::Answer {
                ask,
                revision,
                text,
                run: body.run,
            };
            Ok((line(&answer), Some((ask, revision))))
        }
        Write::Dismiss(ask) => {
            let body: DismissBody = serde_json::from_slice(body).map_err(bad)?;
            let ask = ask_id(db, ask)?;
            let dismiss = tools::Dismiss {
                dismiss: ask,
                revision: Some(body.revision),
            };
            Ok((line(&dismiss), Some((ask, body.revision))))
        }
        Write::Snooze => {
            let body: SnoozeBody = serde_json::from_slice(body).map_err(bad)?;
            let until = match (&body.span, &body.until, body.clear) {
                (Some(span), None, false) => snooze::until(Some(span), None),
                (None, Some(at), false) => snooze::until(None, Some(at)),
                (None, None, true) => Ok(None),
                _ => return Err(Reply::error(400, "send one of for, until and clear: true")),
            }
            .map_err(|error| Reply::error(400, &error))?;
            let snooze = snooze::Snooze {
                snooze: snooze::Item {
                    kind: body.kind,
                    id: body.id,
                },
                until,
            };
            Ok((line(&snooze), None))
        }
    }
}

fn line(message: &impl serde::Serialize) -> String {
    serde_json::to_string(message).expect("a socket line serializes")
}

/// The ask's number, from its number or the name of an outside ask, as `priory answer` reads it.
fn ask_id(db: &Connection, ask: &str) -> Result<i64, Reply> {
    if !ask.is_empty() && ask.bytes().all(|byte| byte.is_ascii_digit()) {
        return ask
            .parse()
            .map_err(|_| Reply::error(404, &format!("there is no ask {ask}")));
    }
    tools::ask_named(db, ask).map_err(|error| Reply::error(404, &error))
}

/// The reply to a write the daemon refused. For an ask: `404` once it is gone, `409` with the ask
/// when its revision moved or it closed, `422` otherwise, such as an option a step does not take.
fn refusal(db: &Connection, ask: Option<(i64, i64)>, refused: &str) -> Reply {
    let Some((id, sent)) = ask else {
        return Reply::error(422, refused);
    };
    match listing::ask(db, id) {
        Ok(None) => Reply::error(404, refused),
        Ok(Some(ask)) if ask["state"] != "open" || ask["revision"] != sent => Reply::json(
            409,
            serde_json::json!({ "error": refused, "ask": ask }).to_string(),
        ),
        Ok(Some(_)) => Reply::error(422, refused),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

/// The time from which `/api/tasks` keeps ended tasks: a week ago, or none for `all=1`.
fn since(query: &str) -> Result<Option<String>, Reply> {
    match query {
        "all=1" => Ok(None),
        "" => week_ago(),
        _ => Err(Reply::error(400, "tasks takes one parameter: all=1")),
    }
}

/// The time a week ago, from which a listing keeps closed rows.
fn week_ago() -> Result<Option<String>, Reply> {
    let now = intake::now().map_err(|error| Reply::error(500, &error))?;
    let week = now
        .checked_sub(jiff::SignedDuration::from_hours(7 * 24))
        .map_err(|error| Reply::error(500, &error.to_string()))?;
    Ok(Some(week.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()))
}

/// A query value with its `%XX` escapes and `+` decoded; none when an escape or the UTF-8 is broken.
fn decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let hex = std::str::from_utf8(rest.get(..2)?)
                    .ok()
                    .filter(|hex| hex.bytes().all(|digit| digit.is_ascii_hexdigit()))?;
                rest = &rest[2..];
                u8::from_str_radix(hex, 16).ok()?
            }
            byte => byte,
        });
    }
    String::from_utf8(bytes).ok()
}

/// A web view file, or the page itself for any other path, such as `/asks/42`.
fn web_file(path: &str) -> Reply {
    let found = WEB.iter().find(|(served, _, _)| *served == path);
    let (kind, body) = match found {
        Some((_, kind, body)) => (*kind, *body),
        None if path.starts_with("/fonts/")
            || path.starts_with("/tokens/")
            || (path
                .strip_prefix('/')
                .is_some_and(|name| !name.contains('/') && name.contains('.'))
                && path != "/index.html") =>
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

/// `/api/events?since=REV[&timeout=SECONDS]`: the revision once it differs from `since`, or the
/// same one after the timeout. Refuses a request that would wait past `WAITERS` held at once with
/// `503`.
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
    let answer = |revision: rusqlite::Result<i64>| match revision {
        Ok(revision) => Reply::json(200, serde_json::json!({ "revision": revision }).to_string()),
        Err(error) => Reply::error(500, &error.to_string()),
    };
    match state::revision(db) {
        Ok(revision) if revision == since && timeout > 0 => {}
        ready => return answer(ready),
    }
    let held = waiters.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |held| {
        (held < WAITERS).then_some(held + 1)
    });
    if held.is_err() {
        return Reply::error(503, "too many requests wait on events: retry in a second");
    }
    let waited = state::wait(db, since, Duration::from_secs(timeout));
    waiters.fetch_sub(1, Ordering::SeqCst);
    answer(waited)
}

//! The HTTP server over the state: `/api` as JSON, with `web` the web view's files, and with a
//! GitHub webhook its signed deliveries.

use crate::{grades, listing, push, report, screen, sections, snooze, state, todo, tools};
use priory_factory::factory::{Factory, Price, Terminal, Trigger};
use priory_intake::intake::{self, Github, LinearWebhook};
use priory_intake::log;
use priory_runtime::http::{self, Reply, Request};
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const GITHUB: &str = "/hooks/github";
const LINEAR: &str = "/hooks/linear";
const INDEX: &[u8] = include_bytes!("../web/index.html");
/// How long `/api/events` holds a request without a change, at most and by default.
const WAIT: u64 = 25;
/// Requests `/api/events` holds at once: half the workers, so listings always find a free one.
const WAITERS: usize = http::WORKERS / 2;
/// Screens read at once, so a hung tmux server holds at most this many workers.
const SNAPSHOTS: usize = 2;

macro_rules! asset {
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
    asset!("/boot.js", JS, "crates/priory-core/web/boot.js"),
    asset!("/app.js", JS, "crates/priory-core/web/app.js"),
    asset!("/asks.js", JS, "crates/priory-core/web/asks.js"),
    asset!("/add.js", JS, "crates/priory-core/web/add.js"),
    asset!("/links.js", JS, "crates/priory-core/web/links.js"),
    asset!("/reports.js", JS, "crates/priory-core/web/reports.js"),
    asset!("/stack.js", JS, "crates/priory-core/web/stack.js"),
    asset!("/flows.js", JS, "crates/priory-core/web/flows.js"),
    asset!("/desk.js", JS, "crates/priory-core/web/desk.js"),
    asset!("/costs.js", JS, "crates/priory-core/web/costs.js"),
    asset!("/inbox.js", JS, "crates/priory-core/web/inbox.js"),
    asset!("/tasks.js", JS, "crates/priory-core/web/tasks.js"),
    asset!("/runs.js", JS, "crates/priory-core/web/runs.js"),
    asset!("/queue.js", JS, "crates/priory-core/web/queue.js"),
    asset!("/batches.js", JS, "crates/priory-core/web/batches.js"),
    asset!("/terminal.js", JS, "crates/priory-core/web/terminal.js"),
    asset!("/sw.js", JS, "crates/priory-core/web/sw.js"),
    asset!(
        "/manifest.webmanifest",
        "application/manifest+json",
        "crates/priory-core/web/manifest.webmanifest"
    ),
    asset!("/icon-192.png", "image/png", "design/logo/icon-192.png"),
    asset!("/icon-512.png", "image/png", "design/logo/icon-512.png"),
    asset!(
        "/icon-maskable-192.png",
        "image/png",
        "design/logo/icon-maskable-192.png"
    ),
    asset!(
        "/icon-maskable-512.png",
        "image/png",
        "design/logo/icon-maskable-512.png"
    ),
    asset!("/app.css", CSS, "crates/priory-core/web/app.css"),
    asset!("/tokens/structure.css", CSS, "design/tokens/structure.css"),
    asset!("/tokens/themes.css", CSS, "design/tokens/themes.css"),
    asset!("/favicon.ico", "image/x-icon", "design/logo/favicon.ico"),
    asset!("/favicon.svg", "image/svg+xml", "design/logo/favicon.svg"),
    asset!(
        "/apple-touch-icon.png",
        "image/png",
        "design/logo/apple-touch-icon.png"
    ),
    asset!(
        "/fonts/mona-sans-200-900.woff2",
        WOFF2,
        "design/fonts/mona-sans-200-900.woff2"
    ),
    asset!(
        "/fonts/jetbrains-mono-400-500.woff2",
        WOFF2,
        "design/fonts/jetbrains-mono-400-500.woff2"
    ),
    asset!(
        "/fonts/eb-garamond-400-800.woff2",
        WOFF2,
        "design/fonts/eb-garamond-400-800.woff2"
    ),
    asset!(
        "/fonts/ibm-plex-sans-400-700.woff2",
        WOFF2,
        "design/fonts/ibm-plex-sans-400-700.woff2"
    ),
    asset!(
        "/fonts/ibm-plex-mono-400.woff2",
        WOFF2,
        "design/fonts/ibm-plex-mono-400.woff2"
    ),
    asset!(
        "/fonts/ibm-plex-mono-500.woff2",
        WOFF2,
        "design/fonts/ibm-plex-mono-500.woff2"
    ),
    asset!(
        "/fonts/alegreya-400-900.woff2",
        WOFF2,
        "design/fonts/alegreya-400-900.woff2"
    ),
    asset!(
        "/fonts/alegreya-sans-400.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-400.woff2"
    ),
    asset!(
        "/fonts/alegreya-sans-500.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-500.woff2"
    ),
    asset!(
        "/fonts/alegreya-sans-700.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-700.woff2"
    ),
    asset!(
        "/fonts/alegreya-sans-800.woff2",
        WOFF2,
        "design/fonts/alegreya-sans-800.woff2"
    ),
];

/// Agent text reaches the page, so the page runs and loads only its own files.
const WEB_HEADERS: &str = "Content-Security-Policy: default-src 'none'; script-src 'self'; \
    style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; \
    worker-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\n\
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
    let linear = factory
        .linear_webhook()
        .map(|hook| env(&hook.secret_env, "the Linear webhook's signing secret"))
        .transpose()?
        .map(|secret| LinearWebhook::new(secret, factory))
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
        views: listing::views(factory).to_string(),
        links: listing::links(factory).to_string(),
        flows: listing::flows(factory).to_string(),
        schedules: listing::schedule_triggers(factory),
        workflows: factory.workflows.keys().cloned().collect(),
        request: serde_json::json!({
            "workflow": factory.request().map(|defaults| &defaults.workflow),
            "band": factory.request().and_then(|defaults| defaults.band.as_ref()),
            "workflows": factory.workflows.keys().collect::<Vec<_>>(),
            "bands": factory.bands(),
        })
        .to_string(),
        prices: factory.prices.clone(),
        terminal: config.terminal,
        secrets: screen::secrets(&factory.secret_envs()),
        snapshots: AtomicUsize::new(0),
    });
    let hooks: &[&str] = match (github.is_some(), linear.is_some()) {
        (true, true) => &[GITHUB, LINEAR],
        (true, false) => &[GITHUB],
        (false, true) => &[LINEAR],
        (false, false) => &[],
    };
    let waiters = Arc::new(AtomicUsize::new(0));
    let public = |path: &str| !path.starts_with("/api") && !path.starts_with("/hooks");
    let event_log = factory.log().clone();
    let origins = config.extension_origins.clone();
    let workers = http::serve(listener, token, origins, hooks, public, move || {
        let db = state::read_only();
        let waiters = Arc::clone(&waiters);
        let home = runtime::home();
        let served = served.clone();
        let mut github = github.clone().map(|github| {
            (
                github,
                log::open(&event_log).map_err(|error| error.to_string()),
            )
        });
        let mut linear = linear.clone().map(|linear| {
            (
                linear,
                log::open(&event_log).map_err(|error| error.to_string()),
            )
        });
        move |request: &Request| match (&mut github, &mut linear) {
            (Some((github, bus)), _)
                if request.method == "POST" && request.path.split('?').next() == Some(GITHUB) =>
            {
                github.receive(bus, request)
            }
            (_, Some((linear, bus)))
                if request.method == "POST" && request.path.split('?').next() == Some(LINEAR) =>
            {
                linear.receive(bus, request)
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
pub(crate) fn env(name: &str, what: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is unset or empty: set it to {what}"))
}

/// What every server thread reads from the factory, once, when the server starts.
struct Served {
    views: String,
    links: String,
    flows: String,
    schedules: Vec<(String, Trigger)>,
    workflows: Vec<String>,
    /// `GET /api/request`: what the Add control prefills and offers.
    request: String,
    prices: BTreeMap<String, Price>,
    terminal: Terminal,
    /// The secret values the terminal hides, read when the server starts.
    secrets: Vec<String>,
    /// Screens being read now.
    snapshots: AtomicUsize,
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
    if path == "/api/request" {
        return Reply::json(200, served.request.clone());
    }
    if path == "/api/push/key" {
        return match push::public_key(home) {
            Some(key) => Reply::json(200, serde_json::json!({ "key": key }).to_string()),
            None => Reply::error(503, "no key yet: start `priory daemon` once to make it"),
        };
    }
    if Write::of(path).is_some() {
        return Reply {
            headers: "Allow: POST\r\n",
            ..Reply::error(405, "only POST is served")
        };
    }
    if web && !path.starts_with("/api") && !path.starts_with("/hooks") {
        return web_file(path);
    }
    match path {
        "/api/views" => return Reply::json(200, served.views.clone()),
        "/api/links" => return Reply::json(200, served.links.clone()),
        "/api/flows" => return Reply::json(200, served.flows.clone()),
        _ => {}
    }
    let db = match db {
        Ok(db) => db,
        Err(error) => return Reply::error(500, error),
    };
    if let Some(run) = path
        .strip_prefix("/api/runs/")
        .and_then(|rest| rest.strip_suffix("/screen"))
    {
        return terminal(db, served, run, query);
    }
    let read = match path {
        "/api/events" => return events(db, query, waiters),
        "/api/batches" => return batches(db, None),
        "/api/desk" => {
            let person = match query.split_once('=') {
                None if query.is_empty() => listing::ME.to_owned(),
                Some(("person", name)) if !name.contains('&') => match decode(name) {
                    Some(name) => name,
                    None => return Reply::error(400, "person is not percent-encoded UTF-8"),
                },
                _ => return Reply::error(400, "desk takes one parameter: person=NAME"),
            };
            let now = match intake::now() {
                Ok(now) => now,
                Err(error) => return Reply::error(500, &error),
            };
            listing::revised(db, "desk", |db| {
                listing::desk(db, home, false, &person, now).map(Some)
            })
        }
        "/api/inbox" => match query {
            "" => listing::revised(db, "inbox", |db| listing::inbox(db, 0).map(Some)),
            "unrouted=1" => {
                listing::revised(db, "unrouted", |db| listing::unrouted(db, 0).map(Some))
            }
            _ => return Reply::error(400, "inbox takes one parameter: unrouted=1"),
        },
        "/api/asks" => {
            let (mut bands, mut risks, mut all) = (Vec::new(), Vec::new(), false);
            for pair in query.split('&').filter(|pair| !pair.is_empty()) {
                let (list, name, value) = match pair.split_once('=') {
                    Some(("all", "1")) => {
                        all = true;
                        continue;
                    }
                    Some(("band", value)) => (&mut bands, "band", value),
                    Some(("risk", value)) => (&mut risks, "risk", value),
                    _ => {
                        return Reply::error(
                            400,
                            "asks takes band=BAND[,BAND...], risk=RISK[,RISK...] and all=1",
                        );
                    }
                };
                let Some(value) = decode(value) else {
                    return Reply::error(400, &format!("{name} is not percent-encoded UTF-8"));
                };
                list.extend(value.split(',').map(str::to_owned));
            }
            for band in &bands {
                if let Err(refusal) = state::check_band(db, band) {
                    return Reply::error(400, &refusal);
                }
            }
            for risk in &risks {
                if let Err(refusal) = listing::check_risk(risk) {
                    return Reply::error(400, &refusal);
                }
            }
            let since = match week_ago() {
                _ if all => None,
                Ok(since) => since,
                Err(reply) => return reply,
            };
            let bands: Vec<&str> = bands.iter().map(String::as_str).collect();
            let risks: Vec<&str> = risks.iter().map(String::as_str).collect();
            listing::revised(db, "asks", |db| {
                listing::recent_asks(db, &bands, &risks, since.as_deref()).map(Some)
            })
        }
        "/api/tasks" if query.starts_with("pr=") => {
            let Some((repo, number)) = decode(&query[3..]).as_deref().and_then(pull_request) else {
                return Reply::error(400, "pr is a pull request such as github:owner/repo#7");
            };
            listing::revised(db, "tasks", |db| {
                listing::holding(db, &repo, number).map(Some)
            })
        }
        "/api/tasks" => match since(query) {
            Ok(since) => listing::revised(db, "tasks", |db| {
                listing::recent_tasks(db, since.as_deref()).map(Some)
            }),
            Err(reply) => return reply,
        },
        "/api/runs" => match since(query) {
            Ok(since) => listing::revised(db, "runs", |db| {
                listing::recent_runs(db, since.as_deref()).map(Some)
            }),
            Err(reply) => return reply,
        },
        "/api/queue" => match intake::now() {
            Ok(now) => listing::revised(db, "queue", |db| {
                listing::queue(db, &served.schedules, &served.workflows, now).map(Some)
            }),
            Err(error) => return Reply::error(500, &error),
        },
        "/api/reports/latest" => listing::revised(db, "report", |db| sections::show(db, None)),
        "/api/costs" => return costs(db, &served.prices, query),
        "/api/grades" => return grades(db, query),
        "/api/attention" => return attention(db, query),
        _ if path.starts_with("/api/batches/") => {
            return batches(db, path.strip_prefix("/api/batches/"));
        }
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

/// Every batch, or the one `which` names, as `priory batch show --json` prints it.
/// `GET /api/runs/{id}/screen`: the run's screen, read by at most `SNAPSHOTS` requests at once.
fn terminal(db: &Connection, served: &Served, run: &str, query: &str) -> Reply {
    let Ok(run) = run.parse() else {
        return Reply::error(404, "no such path");
    };
    if served.terminal == Terminal::Off {
        return Reply::error(404, "[serve] terminal = \"off\" serves no screen");
    }
    let taken = served.snapshots.fetch_add(1, Ordering::SeqCst);
    let _slot = Slot(&served.snapshots);
    if taken >= SNAPSHOTS {
        return Reply::error(503, "too many screens are being read; retry in a second");
    }
    let have = query.split('&').find_map(|pair| pair.strip_prefix("have="));
    match screen::read(db, &state::Attach::Run(run), &served.secrets) {
        Ok(shown) => Reply::json(200, screen::reply(shown, have).to_string()),
        Err(error) => Reply::error(404, &error),
    }
}

/// A screen being read, given back when its request ends, even by a panic.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn batches(db: &Connection, which: Option<&str>) -> Reply {
    let _snapshot = db.unchecked_transaction();
    match crate::batch::show(db, which) {
        Ok(shown) => Reply::json(200, shown.to_string()),
        Err(error) if error.starts_with("there is no batch") => Reply::error(404, &error),
        Err(error) => Reply::error(500, &error),
    }
}

/// A path that writes, each the twin of a CLI verb. An ask is named by its number or, for an
/// outside ask, its name. `Act` is a desk action on a task: `resume`, `revive`, `cancel`, `release`
/// or `say`. `Read` marks a report read.
enum Write<'a> {
    Answer(&'a str),
    Dismiss(&'a str),
    Snooze,
    Add,
    Comment(sections::Remark),
    Pick,
    Act(&'a str, &'a str),
    Subscribe,
    Unsubscribe,
    Follow,
    TodoAdd,
    TodoDone(i64),
    Seen,
    Read(i64),
    BatchEdit(&'a str),
    Move(&'a str),
}

impl<'a> Write<'a> {
    fn of(path: &'a str) -> Option<Self> {
        match path {
            "/api/snooze" => return Some(Write::Snooze),
            "/api/request" => return Some(Write::Add),
            "/api/reports/pick" => return Some(Write::Pick),
            "/api/reports/comment" => return Some(Write::Comment(sections::Remark::Comment)),
            "/api/reports/reply" => return Some(Write::Comment(sections::Remark::Reply)),
            "/api/reports/resolve" => return Some(Write::Comment(sections::Remark::Resolve)),
            "/api/reports/reopen" => return Some(Write::Comment(sections::Remark::Reopen)),
            "/api/push/subscribe" => return Some(Write::Subscribe),
            "/api/push/unsubscribe" => return Some(Write::Unsubscribe),
            "/api/subscribe" => return Some(Write::Follow),
            "/api/todos" => return Some(Write::TodoAdd),
            "/api/seen" => return Some(Write::Seen),
            _ => {}
        }
        if let Some(id) = path
            .strip_prefix("/api/todos/")
            .and_then(|rest| rest.strip_suffix("/done"))
        {
            return id.parse().ok().map(Write::TodoDone);
        }
        if let Some(seq) = path
            .strip_prefix("/api/inbox/")
            .and_then(|rest| rest.strip_suffix("/read"))
        {
            return seq.parse().ok().map(Write::Read);
        }
        if let Some(batch) = path
            .strip_prefix("/api/batches/")
            .and_then(|rest| rest.strip_suffix("/edit"))
        {
            return Some(Write::BatchEdit(batch));
        }
        if let Some(task) = path
            .strip_prefix("/api/tasks/")
            .and_then(|rest| rest.strip_suffix("/move"))
        {
            return Some(Write::Move(task));
        }
        if let Some((task, act)) = path
            .strip_prefix("/api/tasks/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(_, act)| ["resume", "revive", "cancel", "release", "say"].contains(act))
        {
            return Some(Write::Act(task, act));
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

/// What a desk action takes: `say` its text, `revive` an optional step and `cancel` the optional
/// revision it was made on, as the CLI's `--step` and `--expected`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActBody {
    text: Option<String>,
    step: Option<String>,
    revision: Option<i64>,
}

/// What a move takes: the revision the page drew, a band, and at most one place.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveBody {
    revision: i64,
    band: Option<String>,
    #[serde(default)]
    top: bool,
    before: Option<i64>,
    after: Option<i64>,
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

/// The pick endpoint's body: `revision` is the report's revision the client showed.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PickBody {
    pick: String,
    option: String,
    revision: i64,
}

/// A comment endpoint's body: `row` for a new thread, else `comment`; `revision` as `--expected`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentBody {
    row: Option<String>,
    comment: Option<i64>,
    text: Option<String>,
    revision: Option<i64>,
}

/// The body of `POST /api/todos/{id}/done`: nothing.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// `POST /api/seen`'s body: when the page read the desk it drew, as `priory seen --at`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SeenBody {
    at: jiff::Timestamp,
}

/// `POST /api/subscribe`'s body: `revision` as `--expected`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowBody {
    task: i64,
    resource: String,
    revision: Option<i64>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unsubscribe {
    endpoint: String,
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
    let (line, sent) = match write_line(db, &write, request.body) {
        Ok(line) => line,
        Err(reply) => return reply,
    };
    let reply = match runtime::call(home, &line, runtime::REPLY) {
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
        // A repeated text names the task it reached, with the reason nothing was added.
        Ok(Ok(added)) if matches!(write, Write::Add) => {
            let repeated = serde_json::from_str::<serde_json::Value>(&added)
                .is_ok_and(|reply| reply.get("error").is_some());
            Reply::json(if repeated { 409 } else { 200 }, added)
        }
        Ok(Ok(done)) => Reply::json(200, serde_json::json!({ "ok": done }).to_string()),
        Ok(Err(refused)) => refusal(db, sent, &refused),
        Err(error) => Reply::error(500, &format!("unreadable reply {reply:?}: {error}")),
    }
}

/// The revision a write was made on, which tells a stale write from a refused one.
enum Sent {
    /// The ask and the ask's revision the body sent.
    Ask(i64, i64),
    /// The task of a desk action, and the state's revision it was made on, if any.
    Task(i64, Option<i64>),
    /// The pick, the option and the report's revision the body sent.
    Pick(String, String, i64),
    /// The batch and the plan revision the body sent.
    Batch(String, i64),
    Nothing,
}

/// The socket line of `write`, and the revision it was made on.
fn write_line(db: &Connection, write: &Write, body: &[u8]) -> Result<(String, Sent), Reply> {
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
                by: "serve".to_owned(),
            };
            Ok((line(&answer), Sent::Ask(ask, revision)))
        }
        Write::Dismiss(ask) => {
            let body: DismissBody = serde_json::from_slice(body).map_err(bad)?;
            let ask = ask_id(db, ask)?;
            let dismiss = tools::Dismiss {
                dismiss: ask,
                revision: Some(body.revision),
            };
            Ok((line(&dismiss), Sent::Ask(ask, body.revision)))
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
            Ok((line(&snooze), Sent::Nothing))
        }
        Write::Comment(remark) => {
            let body: CommentBody = serde_json::from_slice(body).map_err(bad)?;
            let new = *remark == sections::Remark::Comment;
            let on = match (new, body.row, body.comment) {
                (true, Some(row), None) => row,
                (false, None, Some(comment)) => comment.to_string(),
                (true, ..) => return Err(Reply::error(400, "send the row, not a comment")),
                (false, ..) => return Err(Reply::error(400, "send the comment, not a row")),
            };
            if new != body.revision.is_some() {
                return Err(Reply::error(400, "send revision with a new comment only"));
            }
            let writes = matches!(remark, sections::Remark::Comment | sections::Remark::Reply);
            if writes != body.text.is_some() {
                return Err(Reply::error(
                    400,
                    "send text with a comment or a reply only",
                ));
            }
            let comment = sections::Comment {
                comment: *remark,
                on,
                text: body.text,
                expected: body.revision,
                session: None,
            };
            Ok((line(&comment), Sent::Nothing))
        }
        Write::Pick => {
            let body: PickBody = serde_json::from_slice(body).map_err(bad)?;
            let choice = sections::Choice {
                pick: body.pick,
                option: body.option,
                expected: body.revision,
            };
            let sent = Sent::Pick(choice.pick.clone(), choice.option.clone(), choice.expected);
            Ok((line(&choice), sent))
        }
        Write::Add => {
            let add: priory_intake::request::Add = serde_json::from_slice(body).map_err(bad)?;
            Ok((line(&serde_json::json!({ "add": add })), Sent::Nothing))
        }
        Write::Act(task, act) => {
            let body: ActBody = serde_json::from_slice(body).map_err(bad)?;
            let takes = match *act {
                "say" => body.text.is_some() && body.step.is_none() && body.revision.is_none(),
                "revive" => body.text.is_none() && body.revision.is_none(),
                "cancel" | "release" => body.text.is_none() && body.step.is_none(),
                _ => body.text.is_none() && body.step.is_none() && body.revision.is_none(),
            };
            if !takes {
                let fields = match *act {
                    "say" => "text",
                    "revive" => "an optional step",
                    "cancel" | "release" => "an optional revision",
                    _ => "no field",
                };
                return Err(Reply::error(400, &format!("{act} takes {fields}")));
            }
            let task = task
                .parse()
                .map_err(|_| Reply::error(404, &format!("there is no task {task}")))?;
            let act = tools::Act {
                act: (*act).to_owned(),
                task,
                step: body.step,
                hold: None,
                text: body.text,
                expected: body.revision,
            };
            Ok((line(&act), Sent::Task(act.task, act.expected)))
        }
        Write::Move(task) => {
            let body: MoveBody = serde_json::from_slice(body).map_err(bad)?;
            let place = match (body.top, body.before, body.after) {
                (false, None, None) => None,
                (true, None, None) => Some(state::Place::Top),
                (false, Some(other), None) => Some(state::Place::Before(other)),
                (false, None, Some(other)) => Some(state::Place::After(other)),
                _ => return Err(Reply::error(400, "move takes one of top, before and after")),
            };
            if place.is_none() && body.band.is_none() {
                return Err(Reply::error(400, "move takes a band, a place or both"));
            }
            let task = task
                .parse()
                .map_err(|_| Reply::error(404, &format!("there is no task {task}")))?;
            let moved = state::Move {
                task,
                band: body.band,
                place,
                expected: body.revision,
            };
            let sent = Sent::Task(task, Some(body.revision));
            Ok((line(&state::MoveLine { r#move: moved }), sent))
        }
        Write::Subscribe => {
            let subscription: push::Subscription = serde_json::from_slice(body).map_err(bad)?;
            let change = push::Change::Subscribe(subscription);
            Ok((line(&push::Line { push: change }), Sent::Nothing))
        }
        Write::Unsubscribe => {
            let gone: Unsubscribe = serde_json::from_slice(body).map_err(bad)?;
            let change = push::Change::Unsubscribe {
                endpoint: gone.endpoint,
            };
            Ok((line(&push::Line { push: change }), Sent::Nothing))
        }
        Write::Follow => {
            let body: FollowBody = serde_json::from_slice(body).map_err(bad)?;
            let Some(resource) = crate::subscribe::resource(&body.resource) else {
                return Err(Reply::error(
                    400,
                    &format!(
                        "`{}` is not a resource: name github:owner/name#7, linear:ENG-7 or slack:C0123:1763500000.000100",
                        body.resource
                    ),
                ));
            };
            let request = crate::subscribe::Request {
                task: body.task,
                resource,
                expected: body.revision,
            };
            let sent = Sent::Task(request.task, request.expected);
            Ok((line(&crate::subscribe::Line { subscribe: request }), sent))
        }
        Write::TodoAdd => {
            let add: todo::Add = serde_json::from_slice(body).map_err(bad)?;
            let change = todo::Change::Add(add);
            Ok((line(&todo::Line { todo: change }), Sent::Nothing))
        }
        Write::TodoDone(id) => {
            let Empty {} = serde_json::from_slice(body).map_err(bad)?;
            let change = todo::Change::Done(*id);
            Ok((line(&todo::Line { todo: change }), Sent::Nothing))
        }
        Write::Read(seq) => {
            let Empty {} = serde_json::from_slice(body).map_err(bad)?;
            Ok((line(&crate::post::ReadLine { read: *seq }), Sent::Nothing))
        }
        Write::BatchEdit(batch) => {
            let body: serde_json::Value = serde_json::from_slice(body).map_err(bad)?;
            let (revision, _) =
                crate::batch::edit_of(&body).map_err(|error| Reply::error(400, &error))?;
            let edit = crate::batch::EditLine {
                batch_edit: crate::batch::EditRequest {
                    batch: (*batch).to_owned(),
                    body,
                },
            };
            Ok((line(&edit), Sent::Batch((*batch).to_owned(), revision)))
        }
        Write::Seen => {
            let body: SeenBody = serde_json::from_slice(body).map_err(bad)?;
            let seen = state::Visit {
                person: listing::ME.to_owned(),
                at: body.at,
            };
            Ok((line(&state::SeenLine { seen }), Sent::Nothing))
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
/// when its revision moved or it closed. For a desk action: `404` without the task, `409` once the
/// state moved past the revision it was made on. `422` otherwise, such as an option a step does not
/// take.
fn refusal(db: &Connection, sent: Sent, refused: &str) -> Reply {
    let (id, sent) = match sent {
        Sent::Ask(id, sent) => (id, sent),
        Sent::Task(task, expected) => {
            let found = db
                .query_row("SELECT 1 FROM tasks WHERE id = ?1", [task], |_| Ok(()))
                .optional()
                .and_then(|found| Ok((found, state::revision(db)?)));
            return match found {
                Ok((None, _)) => Reply::error(404, refused),
                Ok((_, at)) if expected.is_some_and(|expected| expected != at) => {
                    Reply::error(409, refused)
                }
                Ok(_) => Reply::error(422, refused),
                Err(error) => Reply::error(500, &error.to_string()),
            };
        }
        Sent::Pick(pick, option, sent) => return pick_refusal(db, &pick, &option, sent, refused),
        Sent::Batch(batch, sent) => {
            let at = db
                .query_row(
                    "SELECT plan_revision FROM batches WHERE CAST(id AS TEXT) = ?1 OR key = ?1",
                    [&batch],
                    |row| row.get::<_, i64>(0),
                )
                .optional();
            return match at {
                Ok(None) => Reply::error(404, refused),
                Ok(Some(at)) if at != sent => Reply::error(409, refused),
                Ok(Some(_)) => Reply::error(422, refused),
                Err(error) => Reply::error(500, &error.to_string()),
            };
        }
        Sent::Nothing => return Reply::error(422, refused),
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

/// The reply to a pick the daemon refused: `404` once no report holds it, `409` once its report
/// moved past the revision sent or another choice took it. A pick this write recorded, whose request
/// then failed to publish, gets `422`, as does any other refusal.
fn pick_refusal(db: &Connection, pick: &str, option: &str, sent: i64, refused: &str) -> Reply {
    // `sections::pick` words a recorded pick so, whatever followed it.
    if refused.starts_with(&format!("pick {pick} is {option} at revision ")) {
        return Reply::error(422, refused);
    }
    let found = db
        .query_row(
            "SELECT reports.revision, json_extract(row.value, '$.pick.chosen')
             FROM reports, json_each(reports.report, '$.sections') AS section,
               json_each(section.value, '$.rows') AS row
             WHERE json_extract(row.value, '$.pick.id') = ?1 LIMIT 1",
            [pick],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional();
    match found {
        Ok(None) => Reply::error(404, refused),
        Ok(Some((at, chosen))) if at != sent || chosen.is_some() => Reply::error(409, refused),
        Ok(Some(_)) => Reply::error(422, refused),
        Err(error) => Reply::error(500, &error.to_string()),
    }
}

/// A report's `since=WINDOW&by=SPLIT` query: its window, and its split, one of `splits`, the
/// first by default.
fn window_by(
    db: &Connection,
    report: &str,
    query: &str,
    splits: &[&str],
) -> Result<(report::Window, String), Reply> {
    let (mut since, mut by) = (None, splits[0].to_owned());
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let Some((name @ ("since" | "by"), value)) = pair.split_once('=') else {
            return Err(Reply::error(
                400,
                &format!("{report} takes since=WINDOW and by=SPLIT"),
            ));
        };
        let Some(value) = decode(value) else {
            return Err(Reply::error(
                400,
                &format!("{name} is not percent-encoded UTF-8"),
            ));
        };
        match name {
            "since" => since = Some(value),
            _ => by = value,
        }
    }
    if !splits.contains(&by.as_str()) {
        return Err(Reply::error(
            400,
            &format!("by is none of {}", splits.join(", ")),
        ));
    }
    let now = intake::now().map_err(|error| Reply::error(500, &error))?;
    let window = report::window(db, since.as_deref(), &now.to_string())
        .map_err(|refusal| Reply::error(400, &refusal))?;
    Ok((window, by))
}

/// `GET /api/costs?since=WINDOW&by=SPLIT`: what `priory costs --json` prints.
fn costs(db: &Connection, prices: &BTreeMap<String, Price>, query: &str) -> Reply {
    let (window, by) = match window_by(db, "costs", query, &report::BY) {
        Ok(window_by) => window_by,
        Err(reply) => return reply,
    };
    match report::costs(db, prices, &window, &by) {
        Ok(costs) => Reply::json(200, costs.json().to_string()),
        Err(error) => Reply::error(500, &error),
    }
}

/// `GET /api/grades?since=WINDOW`: what `priory grades --json` prints.
fn grades(db: &Connection, query: &str) -> Reply {
    let mut since = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let Some(("since", value)) = pair.split_once('=') else {
            return Reply::error(400, "grades takes since=WINDOW");
        };
        let Some(value) = decode(value) else {
            return Reply::error(400, "since is not percent-encoded UTF-8");
        };
        since = Some(value);
    }
    let now = match intake::now() {
        Ok(now) => now.to_string(),
        Err(error) => return Reply::error(500, &error),
    };
    let since = match since
        .map(|since| report::start(db, &since, &now))
        .transpose()
    {
        Ok(since) => since,
        Err(refusal) => return Reply::error(400, &refusal),
    };
    match grades::grades(db, since) {
        Ok(grades) => Reply::json(200, grades.json().to_string()),
        Err(error) => Reply::error(500, &error),
    }
}

/// `GET /api/attention?since=WINDOW&by=SPLIT`: what `priory report attention --json` prints.
fn attention(db: &Connection, query: &str) -> Reply {
    let (window, by) = match window_by(db, "attention", query, &report::attention::BY) {
        Ok(window_by) => window_by,
        Err(reply) => return reply,
    };
    match report::attention::attention(db, &window, &by) {
        Ok(attention) => Reply::json(200, attention.json().to_string()),
        Err(error) => Reply::error(500, &error),
    }
}

/// The time from which `/api/tasks` keeps ended tasks: a week ago, or none for `all=1`.
fn since(query: &str) -> Result<Option<String>, Reply> {
    match query {
        "all=1" => Ok(None),
        "" => week_ago(),
        _ => Err(Reply::error(400, "this listing takes one parameter: all=1")),
    }
}

/// The repository and number of `github:OWNER/REPO#N`.
fn pull_request(resource: &str) -> Option<(String, u64)> {
    let (repo, number) = resource.strip_prefix("github:")?.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    let part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    (part(owner) && part(name))
        .then(|| Some((repo.to_owned(), number.parse().ok()?)))
        .flatten()
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
pub(crate) fn decode(value: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::{Write, pull_request, write_line};
    use proptest::prelude::*;

    thread_local! {
        static DB: rusqlite::Connection = crate::state::memory();
    }

    proptest! {
        #[test]
        fn a_move_body_becomes_one_move_line_or_is_refused(
            body in r#"\{("revision": ?[0-9]{1,3}|"band": ?"[a-z]{0,6}"|"top": ?(true|false)|"before": ?[0-9]|"after": ?[0-9]|"x": ?1)(, ?("band": ?"[a-z]{0,6}"|"top": ?(true|false)|"before": ?[0-9]|"after": ?[0-9])){0,3}\}|.{0,40}"#
        ) {
            // A move reads nothing, so one migrated state serves every case.
            let written = DB.with(|db| write_line(db, &Write::Move("2"), body.as_bytes()));
            if let Ok((line, _)) = written {
                let line: crate::state::MoveLine = serde_json::from_str(&line).unwrap();
                prop_assert!(line.r#move.band.is_some() || line.r#move.place.is_some());
                prop_assert_eq!(line.r#move.task, 2);
            }
        }

        #[test]
        fn a_pr_parameter_parses_to_the_resource_it_names_or_is_refused(
            text in "(github:)?[a-z./#0-9+-]{0,30}|.{0,40}"
        ) {
            if let Some((repo, number)) = pull_request(&text) {
                let resource = format!("github:{repo}#{number}");
                prop_assert_eq!(pull_request(&resource), Some((repo, number)));
            }
        }
    }

    #[test]
    fn a_pr_parameter_names_a_github_pull_request() {
        assert_eq!(
            pull_request("github:o-1/r.s#7"),
            Some(("o-1/r.s".to_owned(), 7))
        );
        for bad in [
            "github:o/r",
            "github:o#7",
            "github:/r#7",
            "github:o/r/x#7",
            "linear:E-7",
        ] {
            assert_eq!(pull_request(bad), None, "{bad}");
        }
    }
}

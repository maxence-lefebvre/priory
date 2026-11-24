use priory_core::memory::Gate;
use priory_core::{
    daemon, deliver, embed, evidence, grades, label, listing, post, progress, proof, relate,
    reload, report, risk, runners, search, sections, serve, snooze, state, subscribe, todo, tools,
};
use priory_factory::{explain, factory, plural, readiness, rules};
use priory_intake::{intake, request};
use priory_runtime::{eval, mcp, runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if std::env::var_os("PRIORY_SESSION").is_some() && priory_factory::guard::person_verb(&words) {
        eprintln!(
            "priory {}: only a person runs it, and this shell is a run's (PRIORY_SESSION is set)",
            words[0]
        );
        return ExitCode::from(2);
    }
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("--version" | "-V") => {
            println!("priory {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("check") => check(args.collect()),
        Some("draw") => {
            let args: Vec<_> = args.collect();
            match &args[..] {
                [workflow] => draw(&factory::folder(None), workflow),
                [dir, workflow] => draw(Path::new(dir), workflow),
                _ => usage(),
            }
        }
        Some("daemon") if std::env::args_os().any(|arg| arg == "--view-only") => {
            let dirs: Vec<_> = args.filter(|arg| *arg != "--view-only").collect();
            let dir = match dirs.as_slice() {
                [] => factory::folder(None),
                [dir] if *dir != "--once" => PathBuf::from(dir),
                _ => return usage(),
            };
            serve("daemon", &dir, |factory, _| daemon::view_only(factory), false)
        }
        Some("intake") if std::env::args_os().nth(2).is_some_and(|arg| arg == "pre") => {
            intake_pre(args.skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect())
        }
        Some(verb @ ("intake" | "daemon")) => {
            let (once, dirs): (Vec<_>, Vec<_>) = args.partition(|arg| arg == "--once");
            let dir = match dirs.as_slice() {
                [] => factory::folder(None),
                [dir] => PathBuf::from(dir),
                _ => return usage(),
            };
            let run = if verb == "intake" {
                |factory: &factory::Factory, once| {
                    intake::run(factory, once, watched, merged, health, ended, pre_ran)
                }
            } else {
                daemon::run
            };
            serve(verb, &dir, run, !once.is_empty())
        }
        Some("batch") => batch(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("add") => add(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("request") => request(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("adopt") => adopt(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("post") => match (args.next(), args.next(), args.next()) {
            (Some(source), Some(line), None) => {
                post(&source.to_string_lossy(), &line.to_string_lossy())
            }
            _ => usage(),
        },
        Some(verb @ ("inbox" | "tasks" | "asks" | "desk" | "queue")) => {
            listed(verb, args.map(|arg| arg.to_string_lossy().into_owned()).collect())
        }
        Some("wait") => wait(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("schedules") => {
            schedules(args.map(|arg| arg.to_string_lossy().into_owned()).collect())
        }
        Some("links") => links(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("flows") => flows(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("views") => views(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("todo") => todo(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("deliver") => deliver(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("snooze") => snooze(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("serve") => serve_http(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("runs") => match (args.next(), args.next()) {
            (None, _) => list(
                "id\ttask\tstep\ttmux\tsession\tstarted\tended\tend\tkit\tcommit\tmemories\tsandbox",
                "SELECT id, task, step, tmux, coalesce(thread, session), started, ended, ended_as, kit_hash,
                   kit_commit, memories_commit, sandbox
                 FROM runs ORDER BY id",
            ),
            (Some(flag), None) if flag == "--json" => runs_json(),
            _ => usage(),
        },
        Some("outbox") => list(
            "id\ttask\trun\tstate\ttext",
            "SELECT id, task, run, state, replace(text, char(10), '\\n') FROM outbox ORDER BY id",
        ),
        Some("waits") => list(
            "id\ttask\trun\tkind\ton\tcreated",
            "SELECT id, task, run, kind, coalesce(resource, until, 'task ' || target,
               printf('%.1f GB free', bytes / 1073741824.0)), created
             FROM waits WHERE fired IS NULL AND withdrawn IS NULL ORDER BY id",
        ),
        Some("attach") => attach(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("screen") => screen(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("trace") => trace(),
        Some("recap") => recap(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("report") => report(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("costs") => costs(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("grades") => grades(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("calibrate") => calibrate(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("score") => score(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("bench") => bench(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("move") => move_task(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some(verb @ ("resume" | "cancel" | "revive" | "release" | "say")) => act(
            verb,
            args.map(|arg| arg.to_string_lossy().into_owned()).collect(),
            None,
        ),
        Some("pause") => match (args.next(), args.next()) {
            (reason, None) => {
                let reason = reason.map(|reason| reason.to_string_lossy().into_owned());
                let paused = state::open().and_then(|db| {
                    state::pause(&db, reason.as_deref()).map_err(|error| error.to_string())
                });
                exit("pause", paused.map(|text| println!("{text}")))
            }
            _ => usage(),
        },
        Some("seen") => seen(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("unpause") => match args.next() {
            None => {
                let unpaused = state::open()
                    .and_then(|db| state::unpause(&db).map_err(|error| error.to_string()));
                exit("unpause", unpaused.map(|text| println!("{text}")))
            }
            Some(_) => usage(),
        },
        Some("hold") => hold(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("actions") => list(
            "id\ttask\tstep\taction\tstatus\tpull_request\tcommit\terror\tstdout\tstderr",
            "SELECT id, task, step, action, status, pull_request, commit_sha,
             replace(error, char(10), '\\n'), replace(stdout, char(10), '\\n'),
             replace(stderr, char(10), '\\n') FROM actions ORDER BY id",
        ),
        Some("task") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(flag), None) if flag == "--json" => task_json(&task.to_string_lossy()),
            (Some(task), None, None) => task_text(&task.to_string_lossy()),
            _ => usage(),
        },
        Some(verb @ ("subscribe" | "unsubscribe")) => {
            match expected(args.map(|arg| arg.to_string_lossy().into_owned()).collect()) {
                Some((revision, args)) if revision.is_none() || verb == "subscribe" => {
                    match &args[..] {
                        [task, resource] => subscription(verb, task, resource, revision),
                        _ => usage(),
                    }
                }
                _ => usage(),
            }
        }
        Some("label") => label(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("patterns") => patterns(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("relate") => relate(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("tldr") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(text), None) => tldr(&task.to_string_lossy(), &text.to_string_lossy()),
            _ => usage(),
        },
        Some("progress") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(fields), None) => {
                progress(&task.to_string_lossy(), &fields.to_string_lossy())
            }
            _ => usage(),
        },
        Some("search") => search(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("memory") => memory(args.collect()),
        Some("memories") => memories(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("gate") if args.next().is_none() => project_gate(),
        Some("evidence") => evidence(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("answer") => answer(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("give") => {
            let mut args: Vec<String> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
            let before = args.len();
            args.retain(|arg| arg != "--force");
            let force = args.len() < before;
            let (dir, key, runner) = match &args[..] {
                [key, runner] => (None, key, runner),
                [dir, key, runner] => (Some(dir.as_str()), key, runner),
                _ => return usage(),
            };
            let Some(factory) = load(&factory::folder(dir)) else {
                return ExitCode::FAILURE;
            };
            reply(
                "give",
                state::read_only().and_then(|db| runners::give(&factory, &db, key, runner, force)),
            )
        }
        Some("dismiss") => dismiss(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("ask") => match (args.next(), args.next(), args.next()) {
            (Some(name), Some(fields), None) => ask(&name.to_string_lossy(), &fields.to_string_lossy()),
            _ => usage(),
        },
        Some("mcp") => match (args.next(), args.next(), args.next()) {
            (Some(flag), None, None) if flag == "--outside" => outside_mcp(),
            (Some(flag), Some(session), first) if flag == "--session" => {
                let mut debate = false;
                let mut set = mcp::Tools::Worker;
                for flag in first.into_iter().chain(args.by_ref()) {
                    let flag = flag.to_string_lossy();
                    match mcp::Tools::of_flag(&flag) {
                        _ if flag == "--debate" && !debate => debate = true,
                        Some(picked) if set == mcp::Tools::Worker => set = picked,
                        _ => return usage(),
                    }
                }
                exit("mcp", mcp::serve(&session.to_string_lossy(), set, debate))
            }
            (Some(flag), Some(log), describe) if flag == "--eval" => {
                let rest: Vec<String> = describe
                    .into_iter()
                    .chain(args)
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                let describe = match rest.as_slice() {
                    [] => None,
                    [flag, tool, text] if flag == "--describe" => Some((tool.as_str(), text.as_str())),
                    _ => return usage(),
                };
                exit("mcp", eval::serve(Path::new(&log), describe))
            }
            _ => usage(),
        },
        Some("eval") => prompt_eval(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("hook") => match (args.next(), args.next(), args.next()) {
            (Some(event), None, None) => hook(&event.to_string_lossy()),
            (Some(event), Some(flag), None) if event == "PreToolUse" && flag == "--outside" => {
                outside_hook()
            }
            _ => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: priory --version\n       priory check [DIR] [--explain [--json]]\n       priory check [DIR] --repo PATH [--json]\n       priory draw [DIR] WORKFLOW\n       priory intake [DIR] [--once]\n       priory intake pre [DIR] TRIGGER < EVENT.json    runs the trigger's pre script on one event, publishes nothing\n       priory daemon [DIR] [--once]    --once routes pending events, starts no task and exits\n       priory daemon --view-only [DIR]    mirrors every runner's asks and tasks and serves them, runs nothing\n       priory batch add [DIR] ISSUE... [--repo <owner/name>] [--entries <workflow>] [--band <band>] [--lane <lane>]\n       priory batch show [BATCH] [--json]\n       priory batch edit BATCH --expected REVISION (--drop ENTRY [--with-successors] | --move ENTRY (--top | --before OTHER | --after OTHER) | --moment ENTRY OTHER MOMENT)\n       priory add [DIR] [--workflow <name>] [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--auto-answer high|medium] [--relates KIND:TASK]... <text>\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--auto-answer high|medium] [--relates KIND:TASK]... [--hold <reason> [CONDITION]...] <text>\n       priory adopt [DIR] TRIGGER [PR...]\n       priory serve [DIR] [--listen ADDR]\n       priory inbox [--unrouted] [--since SEQ] [--json]\n       priory inbox read SEQ\n       priory post SOURCE LINE\n       priory tasks [--all] [--lane <lane>] [--grade red|yellow|green] [--json]\n       priory tasks --graph [TASK] [--mermaid]\n       priory queue [DIR] [--json]\n       priory schedules [DIR] [--json]\n       priory links [DIR] [--json]\n       priory flows [DIR] --json\n       priory views [DIR] --json\n       priory runs [--json]\n       priory outbox\n       priory asks [--all] [--band <band>[,<band>...]]... [--risk <risk>[,<risk>...]]... [--json]\n       priory waits\n       priory actions\n       priory attach (TASK | --run RUN) [--read-only] [--print]\n       priory screen [DIR] (TASK | --run RUN) [--history N] [--json]\n       priory desk [--all] [--json] [--person NAME]\n       priory seen [--person NAME] [--at TIME]\n       priory wait --since REV [--timeout D]\n       priory snooze (ask | blocked | failed | prompt) ID (--for DURATION | --until TIME | --clear)\n       priory todo add TEXT [--band B] [--due TIME]\n       priory todo done ID\n       priory todo list [--json]\n       priory trace\n       priory recap [--since <instant-or-duration>] [--max-bytes N]\n       priory report put TASK FILE|-\n       priory report show [TASK] [--json]\n       priory report list [--json]\n       priory report pick PICK OPTION --expected REVISION\n       priory report comment ROW --expected REVISION TEXT\n       priory report (reply COMMENT TEXT | resolve COMMENT | reopen COMMENT)\n       priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]\n       priory report risk [DIR] [--workflow <name>] [--since <instant-or-duration>] [--by lane] [--json]\n       priory report attention [--since <instant-or-duration>] [--by workflow|lane|kind] [--json]\n       priory costs [DIR] [--since <instant-or-duration>] [--by workflow|agent|model|machine|repo|lane] [--json]\n       priory grades [--since <instant-or-duration>] [--by lane] [--json]\n       priory score [DIR] TASK [--scorer NAME] [--budget USD] [--json]\n       priory score [DIR] --calibrate SCORER --budget USD [--json]\n       priory calibrate triage [DIR] [--json]\n       priory bench capture [DIR] TASK --into FOLDER\n       priory bench replay [DIR] CASE --budget USD [--turn-timeout DURATION]\n       priory bench FILE --base REF --head REF --budget USD [--runs N] [--case NAME] [--turn-timeout DURATION] [--post OWNER/NAME#N]\n       priory move TASK [--band <band>] [--top | --before TASK | --after TASK] [--expected REVISION]\n       priory resume TASK\n       priory say TASK TEXT\n       priory pause [REASON]\n       priory unpause\n       priory cancel TASK [--expected REVISION]\n       priory revive TASK [--step STEP]\n       priory hold TASK --reason TEXT [--until TIME] [--after TASK-OR-ISSUE]... [--merge PR]... [--if-closed release|drop|ask] [--blockers] [--expected REVISION]\n       priory release TASK [--expected REVISION]\n       priory (resume | cancel | release | revive [--step STEP]) [--generation N] [DIR] RUNNER:KEY\n       priory say [--generation N] [DIR] RUNNER:KEY TEXT\n       priory task TASK [--json]\n       priory label [DIR] TASK wrong (--evidence URL [--happened DATE] | --clear)\n       priory patterns [--all] [--json]\n       priory patterns find [DIR] [--since <instant-or-duration>]\n       priory patterns show ID [--json]\n       priory patterns (close | dismiss) [DIR] ID\n       priory patterns capture [DIR] ID --into FOLDER\n       priory relate TASK builds-on|fixes|takes-up|relates OTHER [--remove]\n       priory subscribe TASK RESOURCE [--expected REVISION]\n       priory unsubscribe TASK RESOURCE\n       priory tldr TASK TEXT\n       priory progress TASK JSON\n       priory search QUERY [--state STATE[,STATE...]] [--limit N]\n       priory gate\n       priory evidence add FILE --caption TEXT\n       priory evidence TASK [--json]\n       priory memory [(acquire | lease) [--gb N] [--timeout D] -- COMMAND...]\n       priory memories [DIR] [--all] [--json]\n       priory memories (accept | drop) [DIR] CANDIDATE\n       priory answer [--revision N | --expected N] [--run] [--] [DIR] [RUNNER:]ASK OPTION-OR-TEXT\n       priory give [DIR] KEY RUNNER [--force]\n       priory ask ID JSON\n       priory dismiss [--revision N | --expected N] [DIR] [RUNNER:]ASK\n       priory deliver ASK (--retry | --clear)\n       priory mcp --session ID [--debate] [--consult | --investigator]\n       priory mcp --outside\n       priory mcp --eval LOG [--describe TOOL TEXT]\n       priory eval FILE --budget USD [--case NAME] [--runs N] [--model MODEL]\n       priory hook EVENT\n       priory hook PreToolUse --outside"
    );
    ExitCode::from(2)
}

/// The factory, or `None` after printing its problems.
fn load(dir: &Path) -> Option<factory::Factory> {
    rules::validated(dir)
        .inspect_err(|problems| {
            for problem in problems {
                eprintln!("{problem}");
            }
        })
        .ok()
}

/// Runs `run` on the factory in `dir`, once it loads with no problems.
/// The open waits the intake polls for, read from `PRIORY_DB`; none when the intake runs without
/// a state file.
fn watched() -> Result<Vec<intake::Watch>, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(Vec::new());
    }
    let db = state::open()?;
    let watched = state::watched(&db).map_err(|error| format!("open waits: {error}"))?;
    watched
        .into_iter()
        .map(|(resource, since, rank)| {
            let since = since
                .parse()
                .map_err(|error| format!("wait on {resource} since `{since}`: {error}"))?;
            Ok(intake::Watch {
                resource,
                since,
                rank,
            })
        })
        .collect()
}

/// The task pull requests merged since `since` whose merge commits the intake reads check runs at;
/// none when the intake runs without a state file.
fn merged(since: &str) -> Result<Vec<intake::Merge>, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(Vec::new());
    }
    let db = state::open()?;
    state::merges(&db, since).map_err(|error| format!("merged pull requests: {error}"))
}

/// How many tasks with `key` ended, from `PRIORY_DB`; none when the intake runs without a state
/// file.
fn ended(key: &str) -> Result<u64, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(0);
    }
    let db = state::open()?;
    state::ended(&db, key).map_err(|error| format!("tasks with key {key}: {error}"))
}

/// Records each intake source's poll in the state, when the intake has one. A source failing for
/// longer than `alarm_after` reports.
fn health(
    outcomes: &intake::Outcomes,
    at: jiff::Timestamp,
    alarm_after: std::time::Duration,
) -> Result<(), String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(());
    }
    let mut db = state::open()?;
    let at = at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    for (source, error) in outcomes {
        post::polled(&mut db, source, error.as_deref(), &at, alarm_after)
            .map_err(|error| format!("record the {source} poll: {error}"))?;
    }
    Ok(())
}

/// Records a `pre` script's run on the desk, when the intake has a state file.
fn pre_ran(trigger: &str, failure: Option<&str>) -> Result<(), String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(());
    }
    let mut db = state::open()?;
    post::pre_ran(&mut db, trigger, failure)
        .map_err(|error| format!("report the pre script of trigger {trigger}: {error}"))
}

/// `priory intake pre [DIR] TRIGGER < event.json`: runs the trigger's `pre` script on one event and
/// prints the message the intake would publish. It publishes and records nothing.
fn intake_pre(args: Vec<String>) -> ExitCode {
    let (dir, name) = match args.as_slice() {
        [name] => (factory::folder(None), name),
        [dir, name] => (PathBuf::from(dir), name),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let printed = (|| -> Result<serde_json::Value, String> {
        let trigger = factory
            .triggers
            .get(name)
            .map(|trigger| &trigger.value)
            .ok_or_else(|| format!("no trigger {name}"))?;
        let pre = trigger
            .pre
            .as_ref()
            .ok_or_else(|| format!("trigger {name} has no pre script"))?;
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
            .map_err(|error| format!("read the event on stdin: {error}"))?;
        let source = priory_intake::pre::event(&input).map_err(|error| {
            format!(
                "the event on stdin is not a message such as `priory inbox --json` lists: {error}"
            )
        })?;
        let request = intake::request_for(name, trigger, &source);
        let bands: Vec<String> = factory.bands().into_iter().map(str::to_owned).collect();
        let decided = priory_intake::pre::decide(&factory.dir, &bands, name, pre, &source, request);
        let message = serde_json::to_value(&decided.message).expect("a message serializes");
        Ok(serde_json::json!({
            "outcome": if decided.dropped() { "dropped" } else { "published" },
            "failed": decided.failure,
            "message": message,
        }))
    })();
    match printed {
        Ok(printed) => {
            println!("{printed:#}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory intake pre: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve(
    verb: &str,
    dir: &Path,
    run: fn(&factory::Factory, bool) -> Result<(), String>,
    once: bool,
) -> ExitCode {
    let Some(factory) = load(dir) else {
        return ExitCode::FAILURE;
    };
    if let Err(error) = metrics(&factory, verb, once) {
        eprintln!("priory {verb}: {error}");
        return ExitCode::FAILURE;
    }
    match run(&factory, once) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Starts sending this process's metrics as `service`, unless `once`. Refuses a `PRIORY_METRICS`
/// that names an unknown group, even with `once`.
fn metrics(factory: &factory::Factory, service: &str, once: bool) -> Result<(), String> {
    let groups = priory_core::metrics::groups(factory)?;
    if once {
        return Ok(());
    }
    priory_core::metrics::start(factory, service, groups)
}

/// Attaches this terminal to the tmux session of a task's live run, or of one run, or prints the
/// command that would. tmux keeps owning the session.
fn attach(args: Vec<String>) -> ExitCode {
    let (mut target, mut print, mut read_only) = (None, false, false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--print" => print = true,
            "--read-only" => read_only = true,
            "--run" if target.is_none() => match args.next().map(|run| run.parse()) {
                Some(Ok(run)) => target = Some(state::Attach::Run(run)),
                _ => return usage(),
            },
            _ if target.is_none() => match arg.parse() {
                Ok(task) => target = Some(state::Attach::Task(task)),
                Err(_) => return usage(),
            },
            _ => return usage(),
        }
    }
    let Some(target) = target else {
        return usage();
    };
    let (run, command) = match state::open()
        .and_then(|db| state::live_run(&db, &target))
        .and_then(|run| {
            runtime::attach(&run.tmux, &run.session, read_only)
                .map(|command| (run.id, command))
                .ok_or_else(|| state::no_pane(&run))
        }) {
        Ok(found) => found,
        Err(error) => return exit("attach", Err(error)),
    };
    if !read_only {
        let issued = priory_core::keyboard::AttachLine {
            attach_issued: priory_core::keyboard::Attach {
                run,
                printed: print,
            },
        };
        let line = serde_json::to_string(&issued).expect("a socket line serializes");
        // tmux owns the session, so a person attaches even when nothing can record it.
        let recorded = runtime::call(&runtime::home(), &line, runtime::REPLY).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|_| format!("unreadable reply {reply:?}"))?
                .map(drop)
        });
        match recorded {
            Err(error) if error.starts_with(runtime::NO_DAEMON) => {
                eprintln!("priory attach: not recorded: no priory daemon listens; attaching anyway")
            }
            Err(error) => eprintln!("priory attach: not recorded: {error}"),
            Ok(()) => {}
        }
    }
    if print {
        let quoted: Vec<_> = command.iter().map(|word| shell_word(word)).collect();
        println!("{}", quoted.join(" "));
        return ExitCode::SUCCESS;
    }
    use std::os::unix::process::CommandExt;
    let error = Command::new(&command[0]).args(&command[1..]).exec();
    exit("attach", Err(format!("{}: {error}", command[0])))
}

/// Prints the screen of a task's live run, or of one run, after up to `--history` rows of its
/// scrollback, as `GET /api/runs/{id}/screen` shows it. It hides the secret shapes and the values of
/// the secret variables of the factory in `DIR`, or the default ones without it.
fn screen(args: Vec<String>) -> ExitCode {
    let (mut run, mut json, mut history, mut positional) = (None, false, 0, Vec::new());
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--history" => match args.next().map(|rows| rows.parse::<usize>()) {
                Some(Ok(rows)) => history = rows.min(priory_core::screen::HISTORY),
                _ => return usage(),
            },
            "--run" if run.is_none() => match args.next().map(|run| run.parse()) {
                Some(Ok(id)) => run = Some(id),
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => positional.push(arg),
        }
    }
    let task = |task: &String| task.parse().ok().map(state::Attach::Task);
    let (dir, target) = match (run, &positional[..]) {
        (Some(run), []) => (None, Some(state::Attach::Run(run))),
        (Some(run), [dir]) => (Some(dir), Some(state::Attach::Run(run))),
        (None, [one]) => (None, task(one)),
        (None, [dir, one]) => (Some(dir), task(one)),
        _ => return usage(),
    };
    let Some(target) = target else {
        return usage();
    };
    let factory = match factory::named_folder(dir.map(String::as_str)) {
        Some(dir) => match load(&dir) {
            Some(factory) => factory,
            None => return ExitCode::FAILURE,
        },
        None => factory::Factory::default(),
    };
    let secrets = priory_core::screen::secrets(&factory.secret_envs());
    let read = state::open().and_then(|db| {
        priory_core::screen::read(&db, &target, &secrets, history).map_err(String::from)
    });
    let colours = std::io::IsTerminal::is_terminal(&std::io::stdout());
    exit(
        "screen",
        read.map(|screen| match json {
            true => println!("{screen}"),
            false => screen["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|line| line.as_str().unwrap_or_default())
                .for_each(|line| match colours {
                    true => println!("{line}"),
                    false => println!("{}", priory_core::screen::plain(line)),
                }),
        }),
    )
}

/// `word` as one shell word, in single quotes unless every character is plainly safe.
fn shell_word(word: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./:@%+,".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Every trace record, oldest first, one CloudEvents JSON object per line.
fn trace() -> ExitCode {
    let printed = state::open().and_then(|db| {
        let events = state::events(&db).map_err(|error| error.to_string())?;
        events.iter().for_each(|event| state::emit(event));
        Ok(())
    });
    exit("trace", printed)
}

/// Prints `priory recap [--since <window>] [--max-bytes N]`: the window's results, answers, says,
/// step ends, failed or blocked tasks and candidate names, redacted and bounded, for a dream to read.
fn recap(args: Vec<String>) -> ExitCode {
    let (mut since, mut max_bytes) = (
        priory_core::recap::SINCE.to_owned(),
        priory_core::recap::MAX_BYTES,
    );
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--since", Some(window)) => since = window,
            ("--max-bytes", Some(bytes)) => match bytes.parse() {
                Ok(bytes) => max_bytes = bytes,
                Err(_) => return usage(),
            },
            _ => return usage(),
        }
    }
    let printed = intake::now().and_then(|now| {
        let now = now.to_string();
        let db = state::open()?;
        let start = report::start(&db, &since, &now)?;
        let recap = priory_core::recap::recap(&db, &start, &now, max_bytes)
            .map_err(|error| error.to_string())?;
        print!("{recap}");
        Ok(())
    });
    exit("recap", printed)
}

/// Prints `priory costs [DIR] [--since <window>] [--by <split>] [--json]`, priced from the factory's
/// `prices.toml`.
/// `priory score [DIR] TASK [--scorer NAME] [--budget USD] [--json]`: grades the latest finished
/// run of each step of the task with the factory's scorers, one row per run and scorer.
/// `priory score [DIR] --calibrate SCORER --budget USD [--json]` measures a judge on its labeled
/// cases instead.
fn score(args: Vec<String>) -> ExitCode {
    let (mut only, mut json, mut positional) = (None, false, Vec::new());
    let (mut budget, mut calibrate) = (None, None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--scorer" => match args.next() {
                Some(name) => only = Some(name),
                None => return usage(),
            },
            "--calibrate" => match args.next() {
                Some(name) => calibrate = Some(name),
                None => return usage(),
            },
            "--budget" => match args.next().and_then(|value| value.parse::<f64>().ok()) {
                Some(value) if value > 0.0 && value.is_finite() => budget = Some(value),
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => positional.push(arg),
        }
    }
    if let Some(name) = calibrate {
        let dir = match &positional[..] {
            [] => None,
            [dir] => Some(dir.as_str()),
            _ => return usage(),
        };
        if only.is_some() {
            return usage();
        }
        return calibrate_judge(&factory::folder(dir), &name, budget, json);
    }
    let (dir, task) = match &positional[..] {
        [task] => (None, task),
        [dir, task] => (Some(dir.as_str()), task),
        _ => return usage(),
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    let Some(factory) = load(&factory::folder(dir)) else {
        return ExitCode::FAILURE;
    };
    let scored = state::open().and_then(|mut db| {
        let scores = priory_core::score::score(
            &mut db,
            &factory,
            &runtime::home(),
            task,
            only.as_deref(),
            budget,
        )?;
        let rows = serde_json::Value::Array(scores.iter().map(|score| score.json()).collect());
        let mut columns = vec!["run", "step", "scorer", "verdict", "reason"];
        if scores.iter().any(|score| score.judged.is_some()) {
            columns.push("agreement");
        }
        if json {
            println!("{}", serde_json::json!({ "scores": rows }));
        } else {
            listing::print(&columns, &rows);
        }
        let spent: f64 = scores.iter().map(|score| score.cost).sum();
        if spent > 0.0 {
            eprintln!("the judges spent ${spent:.4}");
        }
        Ok(())
    });
    exit("score", scored)
}

/// `priory score [DIR] --calibrate SCORER --budget USD [--json]`.
fn calibrate_judge(dir: &Path, name: &str, budget: Option<f64>, json: bool) -> ExitCode {
    let Some(budget) = budget else {
        eprintln!("priory score: give --budget USD, the most the calibration may spend");
        return ExitCode::FAILURE;
    };
    let Some(factory) = load(dir) else {
        return ExitCode::FAILURE;
    };
    let calibrated = state::open().and_then(|mut db| {
        let calibrated =
            priory_core::judge::calibrate(&mut db, &factory, &runtime::home(), name, budget)?;
        let rows: Vec<serde_json::Value> = calibrated.rows.iter().map(|row| row.json()).collect();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "scorer": name,
                    "cases": rows,
                    "agreed": calibrated.agreed(),
                    "of": calibrated.cases,
                    "needs": calibrated.needs,
                    "spent": (calibrated.spent * 10_000.0).round() / 10_000.0,
                    "complete": calibrated.complete,
                })
            );
        } else {
            listing::print(
                &["case", "label", "verdict", "agreed", "reason"],
                &serde_json::Value::Array(rows),
            );
            println!("{}", calibrated.line());
        }
        match calibrated.complete {
            true => Ok(()),
            false => Err(
                "the budget ran out before every labeled case ran, so nothing was stored: give a larger --budget"
                    .to_owned(),
            ),
        }
    });
    exit("score", calibrated)
}

/// `priory calibrate triage [DIR] [--json]` measures the triage decision model on its labeled
/// cases.
fn calibrate(args: Vec<String>) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--json")
        .collect();
    let dir = match positional[..] {
        ["triage"] => None,
        ["triage", dir] if !dir.starts_with("--") => Some(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&factory::folder(dir)) else {
        return ExitCode::FAILURE;
    };
    let calibrated = state::open().and_then(|mut db| {
        let calibrated = priory_core::decision::calibrate(&mut db, &factory)?;
        let rows: Vec<serde_json::Value> = calibrated.rows.iter().map(|row| row.json()).collect();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "point": "triage",
                    "cases": rows,
                    "agreed": calibrated.agreed(),
                    "of": calibrated.rows.len(),
                    "needs": calibrated.needs,
                    "misses": calibrated.misses(),
                    "calibrated": calibrated.counts(),
                })
            );
        } else {
            listing::print(
                &["case", "label", "choice", "probability", "agreed", "error"],
                &serde_json::Value::Array(rows),
            );
            println!("{}", calibrated.line());
        }
        match &calibrated.refused {
            None => Ok(()),
            Some(refused) => Err(format!("{refused}, so nothing was stored")),
        }
    });
    exit("calibrate", calibrated)
}

/// `priory bench capture`, `priory bench replay`, and `priory bench FILE --base REF --head REF`.
fn bench(args: Vec<String>) -> ExitCode {
    let Some((verb, rest)) = args.split_first().map(|(verb, rest)| (verb.as_str(), rest)) else {
        return usage();
    };
    match verb {
        "capture" => capture(rest),
        "replay" => replay(rest),
        _ => compare(&args),
    }
}

/// `priory bench FILE --base REF --head REF --budget USD [--runs N] [--case NAME]
/// [--turn-timeout DURATION] [--post OWNER/NAME#N]`: compares two commits of the factory on the
/// benchmark `FILE`, prints the table, and with `--post` writes it on the pull request and sets
/// its head commit's status.
fn compare(args: &[String]) -> ExitCode {
    let (mut base, mut head, mut budget, mut positional) = (None, None, None, Vec::new());
    let mut options = priory_core::bench::Options {
        post: None,
        budget: 0.0,
        runs: 3,
        case: None,
        turn: priory_core::replay::TURN,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => match args.next() {
                Some(value) => base = Some(value.clone()),
                None => return usage(),
            },
            "--head" => match args.next() {
                Some(value) => head = Some(value.clone()),
                None => return usage(),
            },
            "--case" => match args.next() {
                Some(value) => options.case = Some(value.clone()),
                None => return usage(),
            },
            "--post" => match args.next().filter(|value| pull_request(value).is_some()) {
                Some(value) => options.post = Some(value.clone()),
                None => return usage(),
            },
            "--runs" => match args.next().and_then(|value| value.parse::<usize>().ok()) {
                Some(value) if value > 0 => options.runs = value,
                _ => return usage(),
            },
            "--budget" => match args.next().and_then(|value| value.parse::<f64>().ok()) {
                Some(value) if value > 0.0 && value.is_finite() => budget = Some(value),
                _ => return usage(),
            },
            "--turn-timeout" => match args
                .next()
                .and_then(|value| value.parse::<jiff::SignedDuration>().ok())
                .and_then(|value| Duration::try_from(value).ok())
                .filter(|value| !value.is_zero())
            {
                Some(value) => options.turn = value,
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => positional.push(arg.as_str()),
        }
    }
    let ([file], Some(base), Some(head)) = (&positional[..], base, head) else {
        return usage();
    };
    let Some(budget) = budget else {
        eprintln!("priory bench: give --budget USD, the most the comparison may spend");
        return ExitCode::FAILURE;
    };
    options.budget = budget;
    let compared = state::open().and_then(|mut db| {
        let compared = priory_core::bench::compare(
            &mut db,
            &runtime::home(),
            Path::new(file),
            &base,
            &head,
            &options,
            &mut |line| println!("{line}"),
        )?;
        println!();
        print!("{}", compared.table);
        let Some((repo, number)) = options.post.as_deref().and_then(pull_request) else {
            return Ok(());
        };
        let body = priory_core::bench::comment(&compared.table);
        let url = runtime::comment_on_pull_request(repo, number, &body).map_err(|error| {
            format!("the comparison ran, but its comment was not posted: {error}")
        })?;
        println!("posted: {url}");
        match compared.status {
            Some((state, description)) => {
                let context = priory_core::bench::CONTEXT;
                runtime::set_commit_status(repo, &compared.head, state, context, &description)
                    .map_err(|error| {
                        format!("the comparison ran, but its status was not set: {error}")
                    })?;
                println!("status: {context} {state} on {}", compared.head);
            }
            None => println!("status: left as it is, since this comparison is a repeat"),
        }
        Ok(())
    });
    exit("bench", compared)
}

/// The repository and number of a pull request written `owner/name#12`.
fn pull_request(text: &str) -> Option<(&str, u64)> {
    let (repo, number) = text.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    let word = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['-', '.'])
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    (word(owner) && word(name))
        .then_some(())
        .and(number.parse().ok().filter(|number| *number > 0))
        .map(|number| (repo, number))
}

/// `priory bench replay [DIR] CASE --budget USD [--turn-timeout DURATION]`: plays one case again
/// under the factory in `DIR`, and prints its estimate, its ending and what it spent.
fn replay(args: &[String]) -> ExitCode {
    let (mut budget, mut turn, mut positional) = (None, priory_core::replay::TURN, Vec::new());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--budget" => match args.next().and_then(|value| value.parse::<f64>().ok()) {
                Some(value) if value > 0.0 && value.is_finite() => budget = Some(value),
                _ => return usage(),
            },
            "--turn-timeout" => match args
                .next()
                .and_then(|value| value.parse::<jiff::SignedDuration>().ok())
                .and_then(|value| Duration::try_from(value).ok())
                .filter(|value| !value.is_zero())
            {
                Some(value) => turn = value,
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => positional.push(arg.as_str()),
        }
    }
    let (dir, case) = match positional[..] {
        [case] => (None, case),
        [dir, case] => (Some(dir), case),
        _ => return usage(),
    };
    let Some(budget) = budget else {
        eprintln!("priory bench replay: give --budget USD, the most the replay may spend");
        return ExitCode::FAILURE;
    };
    let dir = factory::folder(dir);
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let replayed = priory_factory::case::load(Path::new(case)).and_then(|loaded| {
        let estimate = priory_core::replay::estimate(&loaded.case, &factory, turn)?;
        println!(
            "estimate: ${:.4} of a ${budget:.4} budget, plus at most ${:.4} if it ends inside a turn of {}",
            estimate.dollars,
            estimate.overrun,
            jiff::SignedDuration::try_from(turn).map_or_else(|_| format!("{turn:?}"), |turn| format!("{turn:#}"))
        );
        priory_core::replay::fits(&estimate, budget)?;
        let options = priory_core::replay::Options { budget, turn };
        let replayed = priory_core::replay::replay(
            &dir,
            &factory,
            &loaded,
            &options,
            &mut |line| println!("{line}"),
            &mut |_, _, _| Vec::new(),
        )?;
        if replayed.ending.incomplete() {
            println!("Incomplete");
        }
        println!("ending: {}", replayed.ending);
        println!(
            "spent: ${:.4} of ${budget:.4} in {}s",
            replayed.spent,
            replayed.took.as_secs()
        );
        println!("gh calls not recorded: {}", replayed.unrecorded.len());
        for call in &replayed.unrecorded {
            println!("  gh {call}");
        }
        Ok(())
    });
    exit("bench replay", replayed)
}

/// `priory bench capture [DIR] TASK --into FOLDER`: captures a past task into a new benchmark case
/// folder.
fn capture(args: &[String]) -> ExitCode {
    let (mut into, mut positional) = (None, Vec::new());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--into" => match args.next() {
                Some(folder) => into = Some(PathBuf::from(folder)),
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => positional.push(arg.as_str()),
        }
    }
    let (dir, task) = match positional[..] {
        [task] => (None, task),
        [dir, task] => (Some(dir), task),
        _ => return usage(),
    };
    let (Some(into), Ok(task)) = (into, task.parse()) else {
        return usage();
    };
    let Some(factory) = load(&factory::folder(dir)) else {
        return ExitCode::FAILURE;
    };
    let captured = state::open().and_then(|db| {
        if into.exists() {
            return Err(format!(
                "{} already exists: capture into a new folder",
                into.display()
            ));
        }
        println!(
            "{}",
            priory_core::capture::capture_into(&db, &factory, task, &into)?
        );
        Ok(())
    });
    exit("bench capture", captured)
}

/// The trace of a person's command on the factory, on this machine.
fn person_trace(factory: &factory::Factory) -> priory_core::trace::Trace<'_> {
    priory_core::trace::Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        machine: priory_runtime::room::hostname(),
        runner: None,
        usage: priory_runtime::transcript::usage,
    }
}

/// `priory patterns`: lists failure patterns, finds new ones in a window, shows, closes, dismisses
/// or captures one.
fn patterns(args: Vec<String>) -> ExitCode {
    let (mut all, mut json, mut since, mut into) = (false, false, None, None);
    let mut words = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            "--into" => match args.next() {
                Some(folder) => into = Some(PathBuf::from(folder)),
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => words.push(arg),
        }
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let flags = |allowed: &[bool]| allowed.iter().all(|flag| !flag);
    match words[..] {
        [] if since.is_none() && into.is_none() => {
            let listed = state::open().and_then(|db| {
                let patterns = priory_core::patterns::patterns(&db)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .filter(|pattern| all || priory_core::patterns::shown(pattern))
                    .collect::<Vec<_>>();
                if json {
                    let listed: Vec<Value> = patterns
                        .iter()
                        .map(|pattern| {
                            let mut value = json!(pattern);
                            value.as_object_mut().map(|fields| fields.remove("matches"));
                            value
                        })
                        .collect();
                    println!("{}", Value::Array(listed));
                    return Ok(());
                }
                println!("id\tstate\tworkflow\tstep\ttasks\tlast\ttext");
                for pattern in &patterns {
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        pattern.id,
                        pattern.state,
                        pattern.workflow,
                        pattern.step,
                        pattern.tasks.len(),
                        pattern.last,
                        pattern.text
                    );
                }
                Ok(())
            });
            exit("patterns", listed)
        }
        ["find", ref rest @ ..] if rest.len() <= 1 && flags(&[all, json, into.is_some()]) => {
            let Some(factory) = load(Path::new(rest.first().copied().unwrap_or("."))) else {
                return ExitCode::FAILURE;
            };
            let since = since.unwrap_or_else(|| priory_core::patterns::SINCE.to_owned());
            let found = intake::now().and_then(|now| {
                let now = now.to_string();
                let mut db = state::open()?;
                let start = report::start(&db, &since, &now)?;
                let found =
                    priory_core::patterns::find(&mut db, &person_trace(&factory), &start, &now)?;
                let patterns =
                    priory_core::patterns::patterns(&db).map_err(|error| error.to_string())?;
                let new: usize = found.matched.values().sum();
                println!(
                    "{new} new {} from {start} to {now}",
                    if new == 1 { "match" } else { "matches" }
                );
                for pattern in patterns
                    .iter()
                    .filter(|pattern| found.matched.contains_key(&pattern.id))
                {
                    println!(
                        "{}\t{}\t{}/{}\t+{}, {} {}\t{}",
                        pattern.id,
                        pattern.state,
                        pattern.workflow,
                        pattern.step,
                        found.matched[&pattern.id],
                        pattern.tasks.len(),
                        if pattern.tasks.len() == 1 {
                            "task"
                        } else {
                            "tasks"
                        },
                        pattern.text
                    );
                }
                Ok(())
            });
            exit("patterns find", found)
        }
        ["show", id] if flags(&[all, since.is_some(), into.is_some()]) => {
            let shown = state::open().and_then(|db| {
                let pattern = priory_core::patterns::pattern(&db, id)?;
                if json {
                    println!("{}", json!(pattern));
                    return Ok(());
                }
                println!(
                    "pattern {}: {}, {}/{}, {} tasks, last {}\n{}\n",
                    pattern.id,
                    pattern.state,
                    pattern.workflow,
                    pattern.step,
                    pattern.tasks.len(),
                    pattern.last,
                    pattern.text
                );
                println!("task\tkey\tkind\thappened\ttext");
                for found in &pattern.matches {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        found.task, found.key, found.kind, found.happened, found.text
                    );
                }
                Ok(())
            });
            exit("patterns show", shown)
        }
        [verb @ ("close" | "dismiss"), ref rest @ ..]
            if !rest.is_empty()
                && rest.len() <= 2
                && flags(&[all, json, since.is_some(), into.is_some()]) =>
        {
            let (dir, id) = match rest {
                [id] => (".", *id),
                [dir, id] => (*dir, *id),
                _ => return usage(),
            };
            let Some(factory) = load(Path::new(dir)) else {
                return ExitCode::FAILURE;
            };
            let closed = state::open().and_then(|mut db| {
                let text = priory_core::patterns::close(
                    &mut db,
                    &person_trace(&factory),
                    id,
                    verb == "dismiss",
                )?;
                println!("{text}");
                Ok(())
            });
            exit(&format!("patterns {verb}"), closed)
        }
        ["capture", ref rest @ ..] if flags(&[all, json, since.is_some()]) => {
            let (dir, id) = match rest {
                [id] => (".", *id),
                [dir, id] => (*dir, *id),
                _ => return usage(),
            };
            let Some(into) = into else {
                return usage();
            };
            let Some(factory) = load(Path::new(dir)) else {
                return ExitCode::FAILURE;
            };
            let captured = state::open().and_then(|db| {
                let pattern = priory_core::patterns::pattern(&db, id)?;
                let mut kept = 0;
                for task in priory_core::patterns::tasks(&pattern) {
                    let folder = into.join(format!("task-{task}"));
                    if folder.exists() {
                        println!("skipped task {task}: {} already exists", folder.display());
                        continue;
                    }
                    match priory_core::capture::capture_into(&db, &factory, task, &folder) {
                        Ok(line) => {
                            kept += 1;
                            println!("{line}");
                        }
                        Err(error) => println!("skipped task {task}: {error}"),
                    }
                }
                if kept == 0 {
                    return Err(format!("no task of pattern {id} could be captured"));
                }
                Ok(())
            });
            exit("patterns capture", captured)
        }
        _ => usage(),
    }
}

fn costs(args: Vec<String>) -> ExitCode {
    let (mut since, mut by, mut json, mut dirs) = (None, "workflow".to_owned(), false, Vec::new());
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            "--by" => match args.next() {
                Some(split) if report::BY.contains(&split.as_str()) => by = split,
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    let dir = match &dirs[..] {
        [] => factory::folder(None),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let printed = intake::now().and_then(|now| {
        let now = now.to_string();
        let db = state::open()?;
        let window = report::window(&db, since.as_deref(), &now)?;
        let caps = priory_core::spend::report(&db, &priory_core::spend::Book::new(&factory), &now)
            .map_err(|error| error.to_string())?;
        let held = priory_core::spend::held(&db).map_err(|error| error.to_string())?;
        let costs = report::costs(&db, &factory.prices, &window, &by)?.with_caps(caps, held);
        match json {
            true => state::emit(&costs.json().to_string()),
            false => costs.print(),
        }
        Ok(())
    });
    exit("costs", printed)
}

/// Prints `priory grades [--since <window>] [--by lane] [--json]`: each grade against its tasks'
/// outcomes.
fn grades(args: Vec<String>) -> ExitCode {
    let (mut since, mut json, mut by_lane) = (None, false, false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--by" => match args.next().as_deref() {
                Some("lane") => by_lane = true,
                _ => return usage(),
            },
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            _ => return usage(),
        }
    }
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let since = since
            .map(|since| report::start(&db, &since, &now.to_string()))
            .transpose()?;
        let grades = grades::grades(&db, since, by_lane)?;
        match json {
            true => state::emit(&grades.json().to_string()),
            false => grades.print(),
        }
        Ok(())
    });
    exit("grades", printed)
}

/// Prints `priory report attention [--since <window>] [--by <split>] [--json]`.
fn report_attention(args: Vec<String>) -> ExitCode {
    let (mut since, mut by, mut json) = (None, "workflow".to_owned(), false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            "--by" => match args.next() {
                Some(split) if report::attention::BY.contains(&split.as_str()) => by = split,
                _ => return usage(),
            },
            _ => return usage(),
        }
    }
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let window = report::window(&db, since.as_deref(), &now.to_string())?;
        let attention = report::attention::attention(&db, &window, &by)?;
        match json {
            true => state::emit(&attention.json().to_string()),
            false => attention.print(),
        }
        Ok(())
    });
    exit("report", printed)
}

/// Prints `priory report risk [DIR] [--workflow <name>] [--since <window>] [--by lane] [--json]`.
/// It reads the factory for each irreversible step and the human steps that approve it.
fn report_risk(args: Vec<String>) -> ExitCode {
    let (mut workflow, mut since, mut json) = (None, risk::SINCE.to_owned(), false);
    let mut by_lane = false;
    let mut dirs = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--workflow" => match args.next() {
                Some(name) => workflow = Some(name),
                None => return usage(),
            },
            "--since" => match args.next() {
                Some(window) => since = window,
                None => return usage(),
            },
            "--by" => match args.next().as_deref() {
                Some("lane") => by_lane = true,
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    let dir = match &dirs[..] {
        [] => factory::folder(None),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let start = report::start(&db, &since, &now.to_string())?;
        let gates = risk::Gates::of(&factory);
        let risk = risk::risk(
            &db,
            &gates,
            workflow.as_deref(),
            &since,
            &start,
            now,
            by_lane,
        )?;
        match json {
            true => state::emit(&risk.json().to_string()),
            false => risk.print(),
        }
        Ok(())
    });
    exit("report", printed)
}

/// Prints `priory report [DIR] --workflow <name> [--since <window>] [--cost]`. Only `--cost` reads
/// the factory, for its prices.
fn report(args: Vec<String>) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("put") => return report_put(&args[1..]),
        Some("show") => return report_show(&args[1..]),
        Some("list") => return report_list(&args[1..]),
        Some("pick") => return report_pick(&args[1..]),
        Some("attention") => return report_attention(args[1..].to_vec()),
        Some("risk") => return report_risk(args[1..].to_vec()),
        Some(verb @ ("comment" | "reply" | "resolve" | "reopen")) => {
            return report_comment(verb, &args[1..]);
        }
        _ => {}
    }
    let mut workflow = None;
    let mut since = report::SINCE.to_owned();
    let mut cost = false;
    let mut by_lane = false;
    let mut dirs = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workflow" => workflow = args.next(),
            "--cost" => cost = true,
            "--by" if args.next().as_deref() == Some("lane") => by_lane = true,
            "--since" => match args.next() {
                Some(window) => since = window,
                None => return usage(),
            },
            "--by" => match args.next().as_deref() {
                Some("lane") => by_lane = true,
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    if cost && by_lane {
        return usage();
    }
    let (Some(workflow), [] | [_]) = (workflow, &dirs[..]) else {
        return usage();
    };
    if !cost {
        let printed =
            state::open().and_then(|db| report::outcomes(&db, &workflow, &since, by_lane));
        return exit("report", printed);
    }
    let dir = factory::folder(dirs.first().map(String::as_str));
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let printed =
        state::open().and_then(|db| report::cost(&db, &workflow, &since, &factory.prices));
    exit("report", printed)
}

/// Stores a task's report from a file, or from stdin with `-`: `priory report put TASK FILE|-`.
fn report_put(args: &[String]) -> ExitCode {
    let [task, file] = args else {
        return usage();
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    let text = if file == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(file)
    };
    let put = text
        .map_err(|error| format!("{file}: {error}"))
        .and_then(|text| sections::parse(&text))
        .and_then(|report| state::open().and_then(|mut db| sections::put(&mut db, task, report)))
        .map(|done| println!("{done}"));
    exit("report put", put)
}

/// Sends `priory report pick PICK OPTION --expected REVISION` to the daemon, which applies it.
fn report_pick(args: &[String]) -> ExitCode {
    let [pick, option, flag, expected] = args else {
        return usage();
    };
    let (true, Ok(expected)) = (flag == "--expected", expected.parse()) else {
        return usage();
    };
    let line = serde_json::to_string(&sections::Choice {
        pick: pick.clone(),
        option: option.clone(),
        expected,
    })
    .expect("a choice serializes");
    send("report pick", &line)
}

/// Sends `priory report comment ROW --expected REVISION TEXT`, `reply COMMENT TEXT`,
/// `resolve COMMENT` or `reopen COMMENT` to the daemon. A run's pane sets `PRIORY_SESSION`, which
/// tells the daemon an agent wrote it.
fn report_comment(verb: &str, args: &[String]) -> ExitCode {
    let (remark, on, text, expected) = match (verb, args) {
        ("comment", [row, flag, expected, text]) if flag == "--expected" => {
            let Ok(expected) = expected.parse() else {
                return usage();
            };
            (sections::Remark::Comment, row, Some(text), Some(expected))
        }
        ("reply", [comment, text]) => (sections::Remark::Reply, comment, Some(text), None),
        ("resolve", [comment]) => (sections::Remark::Resolve, comment, None, None),
        ("reopen", [comment]) => (sections::Remark::Reopen, comment, None, None),
        _ => return usage(),
    };
    let line = serde_json::to_string(&sections::Comment {
        comment: remark,
        on: on.clone(),
        text: text.cloned(),
        expected,
        session: std::env::var("PRIORY_SESSION")
            .ok()
            .filter(|id| !id.is_empty()),
    })
    .expect("a comment serializes");
    send(&format!("report {verb}"), &line)
}

/// Prints a task's report, or the one written last: `priory report show [TASK] [--json]`.
fn report_show(args: &[String]) -> ExitCode {
    let (json, rest): (Vec<_>, Vec<_>) = args.iter().partition(|arg| *arg == "--json");
    let task = match rest[..] {
        [] => None,
        [task] => match task.parse() {
            Ok(task) => Some(task),
            Err(_) => return usage(),
        },
        _ => return usage(),
    };
    let shown = state::open().and_then(|db| {
        let report = listing::revised(&db, "report", |db| sections::show(db, task))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| match task {
                Some(task) => format!("task {task} has no report"),
                None => "no task has a report".to_owned(),
            })?;
        if !json.is_empty() {
            state::emit(&report.to_string());
            return Ok(());
        }
        let report = &report["report"];
        state::emit(&format!(
            "task {} revision {} {}: {}",
            report["task"], report["revision"], report["date"], report["greeting"]
        ));
        for section in report["sections"].as_array().into_iter().flatten() {
            state::emit(&format!("{}", section["title"]));
            for row in section["rows"].as_array().into_iter().flatten() {
                let pick = &row["pick"];
                let choice = match &pick["chosen"] {
                    Value::Null if pick.is_object() => {
                        format!("  pick {}: {}", pick["id"], pick["options"])
                    }
                    Value::Null => String::new(),
                    chosen => format!("  pick {}: {chosen}", pick["id"]),
                };
                state::emit(&format!("  {}{choice}", row["text"]));
            }
        }
        Ok(())
    });
    exit("report show", shown)
}

/// Prints every task's report, newest first: `priory report list [--json]`.
fn report_list(args: &[String]) -> ExitCode {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return usage(),
    };
    let listed = state::open().and_then(|db| {
        let listed = listing::revised(&db, "reports", |db| sections::list(db).map(Some))
            .map_err(|error| error.to_string())?
            .expect("a list is always some");
        if json {
            state::emit(&listed.to_string());
            return Ok(());
        }
        for head in listed["reports"].as_array().into_iter().flatten() {
            let text = |key: &str| head[key].as_str().unwrap_or_default().to_owned();
            state::emit(&format!(
                "task {} {}: {}",
                head["task"],
                text("date"),
                text("greeting")
            ));
        }
        Ok(())
    });
    exit("report list", listed)
}

/// `priory seen [--person NAME] [--at TIME]` records a visit to the desk, now by default.
fn seen(args: Vec<String>) -> ExitCode {
    let (mut person, mut at) = (None, None);
    for pair in args.chunks(2) {
        match pair {
            [flag, value] if flag == "--person" && person.is_none() => person = Some(value),
            [flag, value] if flag == "--at" && at.is_none() => match value.parse() {
                Ok(time) => at = Some(time),
                Err(error) => return exit("seen", Err(format!("{value}: {error}"))),
            },
            _ => return usage(),
        }
    }
    let at = at.unwrap_or_else(jiff::Timestamp::now);
    let person = person.map_or(listing::ME, String::as_str);
    let done = state::open()
        .and_then(|db| state::seen(&db, person, at).map_err(|error| error.to_string()));
    exit("seen", done.map(|text| println!("{text}")))
}

/// `priory todo add TEXT [--band B] [--due TIME]` and `priory todo done ID` go to the daemon;
/// `priory todo list [--json]` reads the state.
fn todo(args: Vec<String>) -> ExitCode {
    let change = match &args[..] {
        [verb, rest @ ..] if verb == "add" => {
            let (mut text, mut band, mut due) = (None, None, None);
            let mut rest = rest.iter();
            while let Some(arg) = rest.next() {
                let slot = match arg.as_str() {
                    "--band" => &mut band,
                    "--due" => &mut due,
                    _ if text.is_none() => {
                        text = Some(arg.clone());
                        continue;
                    }
                    _ => return usage(),
                };
                match rest.next() {
                    Some(value) if slot.is_none() => *slot = Some(value.clone()),
                    _ => return usage(),
                }
            }
            let Some(text) = text else {
                return usage();
            };
            todo::Change::Add(todo::Add { text, band, due })
        }
        [verb, id] if verb == "done" => match id.parse() {
            Ok(id) => todo::Change::Done(id),
            Err(_) => return usage(),
        },
        [verb, flags @ ..] if verb == "list" => {
            let json = match flags {
                [] => false,
                [flag] if flag == "--json" => true,
                _ => return usage(),
            };
            let listed =
                state::open().and_then(|db| todo::list(&db).map_err(|error| error.to_string()));
            return exit(
                "todo",
                listed.map(|rows| match json {
                    true => println!("{rows}"),
                    false => {
                        listing::print(&["id", "text", "band", "due", "created", "done"], &rows)
                    }
                }),
            );
        }
        _ => return usage(),
    };
    let line = serde_json::to_string(&todo::Line { todo: change }).expect("a to-do serializes");
    send("todo", &line)
}

/// Sends `priory deliver ASK (--retry | --clear)` to the daemon.
fn deliver(args: Vec<String>) -> ExitCode {
    let (ask, act) = match &args[..] {
        [ask, flag] if flag == "--retry" => (ask, deliver::Act::Retry),
        [ask, flag] if flag == "--clear" => (ask, deliver::Act::Clear),
        _ => return usage(),
    };
    let deliver = match ask_id(ask) {
        Ok(deliver) => deliver,
        Err(error) => return exit("deliver", Err(error)),
    };
    let line =
        serde_json::to_string(&deliver::Line { deliver, act }).expect("a deliver serializes");
    send("deliver", &line)
}

/// Sends `priory snooze KIND ID (--for DURATION | --until TIME | --clear)` to the daemon, with the
/// time in UTC to the second.
fn snooze(args: Vec<String>) -> ExitCode {
    let [kind, id, rest @ ..] = &args[..] else {
        return usage();
    };
    let Ok(id) = id.parse() else {
        return usage();
    };
    let until = match rest {
        [flag] if flag == "--clear" => snooze::until(None, None),
        [flag, span] if flag == "--for" => snooze::until(Some(span), None),
        [flag, time] if flag == "--until" => snooze::until(None, Some(time)),
        _ => return usage(),
    };
    let until = match until {
        Ok(until) => until,
        Err(error) => return exit("snooze", Err(error)),
    };
    let line = serde_json::to_string(&snooze::Snooze {
        snooze: snooze::Item {
            kind: kind.clone(),
            id,
        },
        until,
    })
    .expect("a snooze serializes");
    send("snooze", &line)
}

/// Prints `priory inbox|tasks|asks|desk|queue`, as a table or, with `--json`, as the JSON that
/// `serve` returns. `priory inbox read SEQ` marks a report read instead.
fn listed(verb: &str, mut args: Vec<String>) -> ExitCode {
    if verb == "tasks" && args.first().is_some_and(|arg| arg == "--graph") {
        return graph(&args[1..]);
    }
    if verb == "inbox" && args.first().is_some_and(|arg| arg == "read") {
        let [_, seq] = &args[..] else {
            return usage();
        };
        let Ok(seq) = seq.parse() else {
            return usage();
        };
        return exit(
            "inbox read",
            state::open().and_then(|db| post::read(&db, seq)),
        );
    }
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let all = args
        .iter()
        .position(|arg| arg == "--all" && ["asks", "desk", "tasks"].contains(&verb))
        .map(|at| args.remove(at))
        .is_some();
    let unrouted = verb == "inbox"
        && args
            .iter()
            .position(|arg| arg == "--unrouted")
            .map(|at| args.remove(at))
            .is_some();
    // Without the factory folder, the queue knows no schedules or workflows, so up next lists none.
    let (mut schedules, mut workflows, mut now) = (Vec::new(), Vec::new(), jiff::Timestamp::now());
    if ["queue", "desk"].contains(&verb) || unrouted {
        now = match intake::now() {
            Ok(now) => now,
            Err(error) => return exit(verb, Err(error)),
        };
    }
    if verb == "queue" {
        let dir = match &args[..] {
            [] => None,
            [dir] if !dir.starts_with("--") => Some(dir.as_str()),
            _ => return usage(),
        };
        if let Some(dir) = factory::named_folder(dir) {
            let Some(factory) = load(&dir) else {
                return ExitCode::FAILURE;
            };
            schedules = listing::schedule_triggers(&factory);
            workflows = factory.workflows.into_keys().collect();
        }
        args.clear();
    }
    let (mut lane, mut grade, mut since, mut person) = (None, None, None, None);
    let (mut bands, mut risks) = (Vec::new(), Vec::new());
    for pair in args.chunks(2) {
        match (verb, pair) {
            ("asks", [flag, value]) if flag == "--band" => bands.extend(value.split(',')),
            ("asks", [flag, value]) if flag == "--risk" => risks.extend(value.split(',')),
            ("desk", [flag, value]) if flag == "--person" && person.is_none() => {
                person = Some(value.as_str())
            }
            ("inbox", [flag, value]) if flag == "--since" && since.is_none() => {
                let Ok(value) = value.parse() else {
                    return usage();
                };
                since = Some(value)
            }
            ("tasks", [flag, value]) if flag == "--lane" && lane.is_none() => {
                lane = Some(value.as_str())
            }
            ("tasks", [flag, value])
                if flag == "--grade"
                    && grade.is_none()
                    && tools::GRADES.contains(&value.as_str()) =>
            {
                grade = Some(value.as_str())
            }
            _ => return usage(),
        }
    }
    let home = runtime::home();
    let shown = state::open().and_then(|db| {
        for band in &bands {
            state::check_band(&db, band)?;
        }
        for risk in &risks {
            listing::check_risk(risk)?;
        }
        let name = if unrouted { "unrouted" } else { verb };
        let value = listing::revised(&db, name, |db| {
            match verb {
                "inbox" if unrouted => listing::unrouted(db, since.unwrap_or(0), now),
                "inbox" => listing::inbox(db, since.unwrap_or(0)),
                "tasks" => listing::tasks(db, lane, grade, all),
                "asks" => listing::asks(db, &bands, &risks),
                "desk" => listing::desk(db, &home, all, person.unwrap_or(listing::ME), now),
                _ => listing::queue(db, &schedules, &workflows, now),
            }
            .map(Some)
        })
        .map_err(|error| error.to_string())?
        .expect("a listing always reads");
        if json.is_some() {
            state::emit(&value.to_string());
            return Ok(());
        }
        if unrouted {
            print_unrouted(&value[name]);
            print_outsider(&value["outsider"]);
            return Ok(());
        }
        let rows = &value[verb];
        match verb {
            "inbox" => listing::print(&["seq", "type", "subject", "task", "outcome", "line"], rows),
            "tasks" => {
                let mut columns = vec![
                    "id",
                    "key",
                    "workflow",
                    "step",
                    "state",
                    "band",
                    "lane",
                    "reason",
                    "position",
                    "generation",
                    "retired",
                    "grade",
                    "grade_reason",
                ];
                // In a factory of several runners, each task names its runner, hold and when that
                // runner was last alive.
                if priory_core::runners::me(&db)
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    columns.extend(["runner", "hold", "last_seen"]);
                }
                listing::print(&columns, rows)
            }
            "asks" => {
                let open = rows
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|ask| ask["state"] == "open" && (all || ask["snoozed_until"].is_null()))
                    .map(|ask| {
                        let mut ask = ask.clone();
                        if ask["merge_risk"].is_object() {
                            let lines = tools::RISK_DIMENSIONS.map(|dimension| {
                                ask["merge_risk"][dimension].as_str().unwrap_or_default()
                            });
                            ask["merge_risk"] = lines.join("; ").into();
                        }
                        if let Some(estimate) = ask.get_mut("risk_estimate") {
                            *estimate = estimate["line"].clone();
                        }
                        ask
                    })
                    .collect();
                listing::print(
                    &[
                        "id",
                        "task",
                        "revision",
                        "question",
                        "options",
                        "snoozed_until",
                        "back_early",
                        "title",
                        "text",
                        "what_waits",
                        "next_action",
                        "command",
                        "coordinator",
                        "key",
                        "run_id",
                        "session",
                        "merge_risk",
                        "risk_estimate",
                    ],
                    &serde_json::Value::Array(open),
                );
            }
            "desk" => {
                if let Some(paused) = value["paused"].as_str() {
                    state::emit(paused);
                }
                for cap in value["caps"].as_array().into_iter().flatten() {
                    state::emit(cap.as_str().unwrap_or_default());
                }
                for review in value["reviews"].as_array().into_iter().flatten() {
                    state::emit(review.as_str().unwrap_or_default());
                }
                if let Some(banner) = value["banner"].as_str() {
                    state::emit(banner);
                }
                if let Some(intake) = value["intake"].as_str() {
                    state::emit(intake);
                }
                if let Some(unrouted) = value["counts"]["unrouted"].as_u64().filter(|n| *n > 0) {
                    let events = if unrouted == 1 { "event" } else { "events" };
                    state::emit(&format!(
                        "{unrouted} {events} reached no task in the last 24 hours. See them with `priory inbox --unrouted`."
                    ));
                }
                if let Some(dropped) = value["counts"]["dropped"].as_u64().filter(|n| *n > 0) {
                    let (requests, it) = if dropped == 1 {
                        ("request", "it")
                    } else {
                        ("requests", "them")
                    };
                    state::emit(&format!(
                        "{dropped} {requests} a pre script dropped in the last 24 hours. See {it} with `priory inbox --unrouted`."
                    ));
                }
                if let Some(unheard) = value["counts"]["unheard"].as_u64().filter(|n| *n > 0) {
                    let (events, it) = if unheard == 1 {
                        ("event", "it")
                    } else {
                        ("events", "them")
                    };
                    state::emit(&format!(
                        "{unheard} {events} kept by a sleeping task reached no agent. See {it} with `priory inbox --unrouted`."
                    ));
                }
                if let Some(outsider) = value["counts"]["outsider"].as_u64().filter(|n| *n > 0) {
                    let (comments, them) = if outsider == 1 {
                        ("comment", "it")
                    } else {
                        ("comments", "them")
                    };
                    state::emit(&format!(
                        "{outsider} {comments} from outside the repository came in the last 24 hours. See {them} with `priory inbox --unrouted`."
                    ));
                }
                listing::print(
                    &[
                        "since",
                        "kind",
                        "id",
                        "task",
                        "lane",
                        "what",
                        "generation",
                        "snoozed_until",
                        "back_early",
                    ],
                    &value["items"],
                );
                print_since_you_looked(&value["since_you_looked"]);
            }
            _ => {
                listing::print(
                    &[
                        "position",
                        "band_position",
                        "id",
                        "key",
                        "workflow",
                        "step",
                        "state",
                        "band",
                        "aged",
                        "lane",
                        "reason",
                    ],
                    &value["waiting"],
                );
                print_asleep(&value["asleep"]);
            }
        }
        Ok(())
    });
    exit(verb, shown)
}

/// After a blank line, what happened since the person last looked, newest first, when anything did.
fn print_since_you_looked(events: &Value) {
    let Some(events) = events.as_array().filter(|events| !events.is_empty()) else {
        return;
    };
    state::emit("");
    state::emit("since you looked\ttask\twhat");
    for event in events {
        let cell = |field: &str| event[field].as_str().unwrap_or("-");
        state::emit(&format!(
            "{}\t{}\t{}",
            cell("time"),
            cell("key"),
            cell("what")
        ));
    }
}

/// After a blank line, each asleep task with what its waits wait on and since when, when one is.
fn print_asleep(asleep: &Value) {
    let Some(tasks) = asleep.as_array().filter(|tasks| !tasks.is_empty()) else {
        return;
    };
    state::emit("");
    state::emit("asleep\tkey\tsince\twaits on");
    for task in tasks {
        let waits: Vec<String> = task["waits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|wait| {
                let text = |field: &str| wait[field].as_str().unwrap_or("-").to_owned();
                format!("{} {} since {}", text("kind"), text("on"), text("since"))
            })
            .collect();
        let cell = |field: &str| task[field].as_str().map_or("-".to_owned(), str::to_owned);
        state::emit(&format!(
            "{}\t{}\t{}\t{}",
            task["id"],
            cell("key"),
            cell("since"),
            if waits.is_empty() {
                "an ask".to_owned()
            } else {
                waits.join("; ")
            }
        ));
    }
}

/// Lists each schedule trigger with its next firing: `priory schedules [DIR] [--json]`.
fn schedules(mut args: Vec<String>) -> ExitCode {
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let dir = match &args[..] {
        [] => factory::folder(None),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let shown = intake::now().and_then(|now| {
        let db = state::open()?;
        let schedules = listing::schedule_triggers(&factory);
        let value = listing::revised(&db, "schedules", |db| {
            listing::schedules(db, &schedules, now).map(Some)
        })
        .map_err(|error| error.to_string())?
        .expect("a listing always reads");
        match json {
            Some(_) => state::emit(&value.to_string()),
            None => listing::print(
                &["trigger", "next", "key", "task", "workflow"],
                &value["schedules"],
            ),
        }
        Ok(())
    });
    exit("schedules", shown)
}

/// The factory's links, as `GET /api/links` serves them: `priory links [DIR] [--json]`.
fn links(mut args: Vec<String>) -> ExitCode {
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let dir = match &args[..] {
        [] => factory::folder(None),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let links = listing::links(&factory);
    match json {
        Some(_) => println!("{links}"),
        None => listing::print(&["pattern", "url"], &links["links"]),
    }
    ExitCode::SUCCESS
}

/// Prints what `GET /api/flows` serves: `priory flows [DIR] --json`. `priory check --explain`
/// prints the same facts as text.
fn flows(mut args: Vec<String>) -> ExitCode {
    let Some(at) = args.iter().position(|arg| arg == "--json") else {
        return usage();
    };
    args.remove(at);
    let dir = match &args[..] {
        [] => factory::folder(None),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let mut flows = listing::flows(&factory);
    listing::with_risk_estimates(&mut flows, history(&factory).as_ref());
    println!("{flows}");
    ExitCode::SUCCESS
}

/// Each irreversible step's trials, when `PRIORY_DB` names a state file this priory reads.
fn history(factory: &factory::Factory) -> Option<risk::Risk> {
    let db = state::read_only().ok()?;
    intake::now()
        .and_then(|now| risk::recent(&db, &risk::Gates::of(factory), now))
        .inspect_err(|error| eprintln!("warning: no history from the state: {error}"))
        .ok()
}

/// Prints the resolved views as `GET /api/views` serves them: `priory views [DIR] --json`.
fn views(mut args: Vec<String>) -> ExitCode {
    let Some(at) = args.iter().position(|arg| arg == "--json") else {
        return usage();
    };
    args.remove(at);
    let dir = match &args[..] {
        [] => factory::folder(None),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    println!("{}", listing::views(&factory));
    ExitCode::SUCCESS
}

/// How long `priory wait` waits without `--timeout`, as long as `serve`'s `/api/events`.
const WAIT_TIMEOUT: Duration = Duration::from_secs(25);

/// Prints the state's revision once it differs from `--since`, or the same one after the timeout:
/// `priory wait --since REV [--timeout D]`.
fn wait(args: Vec<String>) -> ExitCode {
    let (mut since, mut timeout) = (None, WAIT_TIMEOUT);
    for pair in args.chunks(2) {
        match pair {
            [flag, value] if flag == "--since" && since.is_none() => match value.parse::<i64>() {
                Ok(value) => since = Some(value),
                Err(_) => return usage(),
            },
            [flag, value] if flag == "--timeout" => {
                let Some(value) = value
                    .parse::<jiff::SignedDuration>()
                    .ok()
                    .and_then(|value| Duration::try_from(value).ok())
                else {
                    return usage();
                };
                timeout = value;
            }
            _ => return usage(),
        }
    }
    let Some(since) = since else {
        return usage();
    };
    let waited = state::open().and_then(|db| {
        state::wait(&db, since, timeout)
            .map(|revision| state::emit(&revision.to_string()))
            .map_err(|error| error.to_string())
    });
    exit("wait", waited)
}

/// Stores one report line on the inbox and prints its sequence number: `priory post SOURCE LINE`.
fn post(source: &str, line: &str) -> ExitCode {
    let posted = state::open()
        .and_then(|db| post::post(&db, source, line))
        .map(|seq| state::emit(&seq.to_string()));
    exit("post", posted)
}

/// Serves the state over HTTP until killed: `priory serve [DIR] [--listen ADDR]`.
fn serve_http(args: Vec<String>) -> ExitCode {
    let (mut dirs, mut listen) = (Vec::new(), None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => match args.next().map(|addr| addr.parse()) {
                Some(Ok(addr)) => listen = Some(addr),
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    let dir = match &dirs[..] {
        [] => factory::folder(None),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let Some(config) = factory.serve() else {
        return exit(
            "serve",
            Err("factory.toml has no [serve] table, so nothing listens".to_owned()),
        );
    };
    if let Err(error) = metrics(&factory, "serve", false) {
        return exit("serve", Err(error));
    }
    let mut watch = reload::Watch::new(&factory);
    let server = match serve::start(&factory, listen.unwrap_or(config.listen)) {
        Ok(server) => server,
        Err(error) => return exit("serve", Err(error)),
    };
    let mut live = factory;
    while !server.workers.iter().all(|worker| worker.is_finished()) {
        std::thread::sleep(reload::EVERY);
        match watch.poll() {
            reload::Polled::Same | reload::Polled::Current => {}
            reload::Polled::Refused(problems) => reload::report("serve", Err(&problems)),
            reload::Polled::Loaded(new, files) => {
                let restart = reload::restart_only(&live, &new);
                server.update(&new);
                watch.applied(&new, files);
                reload::report("serve", Ok(&restart));
                live = *new;
            }
        }
    }
    ExitCode::SUCCESS
}

fn list(header: &str, query: &str) -> ExitCode {
    let printed = state::open()
        .and_then(|db| state::print(&db, header, query, []).map_err(|error| error.to_string()));
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory: {error}");
            ExitCode::FAILURE
        }
    }
}

fn request(args: Vec<String>) -> ExitCode {
    let Some((dir, request)) = request::parse(args) else {
        return usage();
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    if let Err(error) = related(request.relates()) {
        eprintln!("priory request: {error}");
        return ExitCode::FAILURE;
    }
    match request::run(&factory, &request, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory request: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `priory adopt [DIR] TRIGGER [PR...]` parks a task on each open pull request of the comment or
/// review trigger `TRIGGER`'s author that no task in any state owns, or on the ones `PR...` names.
fn adopt(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter().peekable();
    let dir = match args.peek() {
        Some(dir) if Path::new(dir).is_dir() => PathBuf::from(args.next().unwrap_or_default()),
        _ => factory::folder(None),
    };
    let Some(name) = args.next().filter(|name| !name.starts_with("--")) else {
        return usage();
    };
    let named: Vec<String> = args.collect();
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let fail = |error: &str| {
        eprintln!("priory adopt: {error}");
        ExitCode::FAILURE
    };
    let Some(trigger) = factory.triggers.get(&name).map(|trigger| &trigger.value) else {
        return fail(&format!("the factory has no trigger {name}"));
    };
    let (Some(repo), Some(author)) = (
        trigger.repo.as_deref(),
        trigger
            .filter
            .as_ref()
            .and_then(|filter| filter.get("author")),
    ) else {
        return fail(&format!(
            "trigger {name} is not a pull request comment or review trigger"
        ));
    };
    if trigger.source != "github" || !rules::FEEDBACK.contains(&trigger.on.as_str()) {
        return fail(&format!(
            "trigger {name} is not a pull request comment or review trigger"
        ));
    }
    let mut numbers = vec![];
    for pr in &named {
        match pull_request_number(pr, repo) {
            Some(number) => numbers.push(number),
            None => return fail(&format!("{pr} is not a pull request of {repo}")),
        }
    }
    let open = match runtime::open_pull_requests(repo, Some(author)) {
        Ok(open) => open,
        Err(error) => return fail(&error),
    };
    let listed = |number: u64| {
        open.iter()
            .find(|pull| pull["number"].as_u64() == Some(number))
    };
    if let Some(number) = numbers.iter().find(|&&number| listed(number).is_none()) {
        return fail(&format!(
            "{repo}#{number} is not an open pull request by {author}"
        ));
    }
    let db = match state::open() {
        Ok(db) => db,
        Err(error) => return fail(&error),
    };
    for pull in &open {
        let Some(number) = pull["number"].as_u64() else {
            continue;
        };
        if !numbers.is_empty() && !numbers.contains(&number) {
            continue;
        }
        let key = format!("github:{repo}#{number}");
        match state::any_owner(&db, &key) {
            Ok(Some(task)) => {
                println!("{key}: task {task} owns it");
                continue;
            }
            Ok(None) => {}
            Err(error) => return fail(&error.to_string()),
        }
        let title = pull["title"].as_str().unwrap_or_default();
        let url = pull["url"].as_str().unwrap_or_default();
        match request::adopt(&factory, &name, trigger, number, title, url) {
            Ok(key) => println!("adopted {key}"),
            Err(error) => return fail(&error),
        }
    }
    ExitCode::SUCCESS
}

/// The number of the pull request `pr` names in `repo`: `42`, `owner/name#42` or its URL.
fn pull_request_number(pr: &str, repo: &str) -> Option<u64> {
    let url = format!("https://github.com/{repo}/pull/");
    let number = match (pr.strip_prefix(&url), pr.split_once('#')) {
        (Some(number), _) => number,
        (None, Some((named, number))) if named == repo => number,
        (None, None) => pr,
        _ => return None,
    };
    number.parse().ok()
}

fn add(args: Vec<String>) -> ExitCode {
    let Some((dir, add)) = request::parse_add(args) else {
        return usage();
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    if let Err(error) = related(&add.relates) {
        eprintln!("priory add: {error}");
        return ExitCode::FAILURE;
    }
    match request::add(&factory, &add) {
        Ok(key) => {
            println!("{key}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory add: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Refuses `--relates` to a task the state file does not hold, before anything is published.
fn related(relates: &[request::Relation]) -> Result<(), String> {
    if relates.is_empty() {
        return Ok(());
    }
    let db = state::open()?;
    match relate::missing(&db, relates).map_err(|error| error.to_string())? {
        Some(missing) => Err(missing),
        None => Ok(()),
    }
}

/// `priory batch add [DIR] ISSUE...` publishes a batch's planning request.
fn batch(args: Vec<String>) -> ExitCode {
    match args.split_first() {
        Some((verb, rest)) if verb == "add" => {
            let Some((dir, batch)) = request::parse_batch(rest.to_vec()) else {
                return usage();
            };
            let Some(factory) = load(&dir) else {
                return ExitCode::FAILURE;
            };
            let added = request::add_batch(&factory, &batch).map_err(|error| error.to_string());
            exit("batch add", added.map(|key| println!("{key}")))
        }
        Some((verb, rest)) if verb == "show" => {
            let (json, which): (Vec<&String>, Vec<&String>) =
                rest.iter().partition(|arg| *arg == "--json");
            let which = match which[..] {
                [] => None,
                [which] if !which.starts_with("--") => Some(which.as_str()),
                _ => return usage(),
            };
            let shown = state::open()
                .and_then(|db| priory_core::batch::show(&db, which))
                .map(|shown| match (json.is_empty(), which) {
                    (false, _) => state::emit(&shown.to_string()),
                    (true, None) => listing::print(
                        &["id", "key", "state", "title", "repo", "band", "went"],
                        &shown["batches"],
                    ),
                    (true, Some(_)) => {
                        listing::print(&["id", "key", "state", "title", "repo"], &json!([shown]));
                        listing::print(
                            &["entry", "position", "issue", "task", "state", "stage"],
                            &shown["entries"],
                        );
                        listing::print(&["issue", "why", "closed"], &shown["closes"]);
                    }
                });
            exit("batch show", shown)
        }
        Some((verb, rest)) if verb == "edit" => {
            let Some((Some(expected), args)) = expected(rest.to_vec()) else {
                return usage();
            };
            let Some((which, change)) = batch_edit(&args) else {
                return usage();
            };
            let edited = state::open()
                .and_then(|mut db| priory_core::batch::edit(&mut db, which, expected, &change));
            exit("batch edit", edited.map(|edited| println!("{edited}")))
        }
        _ => usage(),
    }
}

/// `BATCH (--drop ENTRY [--with-successors] | --move ENTRY (--top | --before OTHER | --after
/// OTHER) | --moment ENTRY OTHER MOMENT)`, the rest of `priory batch edit` after `--expected`.
fn batch_edit(args: &[String]) -> Option<(&str, priory_core::batch::Edit)> {
    use priory_core::batch::{Edit, Place};
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (which, rest) = words.split_first()?;
    let change = match *rest {
        ["--drop", entry] | ["--drop", entry, "--with-successors"] => Edit::Drop {
            entry: entry.to_owned(),
            with_successors: rest.len() == 3,
        },
        ["--move", entry, "--top"] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::Top,
        },
        ["--move", entry, "--before", other] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::Before(other.to_owned()),
        },
        ["--move", entry, "--after", other] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::After(other.to_owned()),
        },
        ["--moment", entry, other, moment] => Edit::Moment {
            entry: entry.to_owned(),
            other: other.to_owned(),
            moment: moment.to_owned(),
        },
        _ => return None,
    };
    Some((which, change))
}

fn exit(verb: &str, done: Result<(), String>) -> ExitCode {
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `priory eval FILE --budget USD`: runs a prompt evaluation against a real model, prints its table,
/// and rewrites the doc's table when every case ran within the budget.
fn prompt_eval(args: Vec<String>) -> ExitCode {
    let Some((file, flags)) = args.split_first() else {
        return usage();
    };
    let (mut case, mut runs, mut model, mut budget) = (None, 5, "haiku".to_owned(), None);
    for pair in flags.chunks(2) {
        match pair {
            [flag, value] if flag == "--case" => case = Some(value.clone()),
            [flag, value] if flag == "--model" => model = value.clone(),
            [flag, value] if flag == "--runs" => match value.parse() {
                Ok(value) if value > 0 => runs = value,
                _ => return usage(),
            },
            [flag, value] if flag == "--budget" => match value.parse::<f64>() {
                Ok(value) if value > 0.0 && value.is_finite() => budget = Some(value),
                _ => return usage(),
            },
            _ => return usage(),
        }
    }
    let Some(budget) = budget else {
        eprintln!("priory eval: give --budget USD, the most the evaluation may spend");
        return ExitCode::FAILURE;
    };
    let path = Path::new(file);
    let done = eval::load(path).and_then(|loaded| {
        let scratch = std::env::temp_dir().join(format!("priory-eval-{}", std::process::id()));
        let options = eval::Options {
            runs,
            budget,
            case: case.as_deref(),
        };
        let report = eval::run(
            &loaded,
            &scratch,
            &options,
            &mut eval::Claude { model: &model },
        );
        let _ = std::fs::remove_dir_all(&scratch);
        let report = report?;
        let table = eval::table(&report, &model, runs);
        print!("{table}");
        match &loaded.doc {
            Some(doc) if case.is_none() && report.complete => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                eval::rewrite(doc, &name, &table)
            }
            _ => Ok(()),
        }
    });
    exit("eval", done)
}

/// `priory memory` lists the memory grants, held and waiting.
fn memory(args: Vec<OsString>) -> ExitCode {
    match args.split_first() {
        None => list(
            "id\ttask\tstate\tgb\tcommand\tage\tpid",
            "SELECT id, task, state, printf('%.1f', bytes / 1073741824.0),
             replace(command, char(10), '\\n'),
             (strftime('%s', 'now') - strftime('%s', coalesce(granted, created))) || 's', pid
             FROM grants ORDER BY id",
        ),
        Some((verb, args)) if verb == "acquire" => acquire(args, false),
        Some((verb, args)) if verb == "lease" => acquire(args, true),
        Some(_) => usage(),
    }
}

/// The exit code of `priory memory acquire` when the gate itself fails, as `env` and `timeout` use.
const GATE_FAILED: u8 = 125;
/// How long a grant waits without `--timeout`: two runs' grants can wait on each other's promises.
const GATE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// The GB a grant asks for without `--gb`, as Fleet's gate did and agents' instructions assume.
const GATE_GB: f64 = 10.0;

/// Runs a command once the daemon grants it `--gb` GB of memory, and releases the grant when it
/// exits. Exits with the command's code. A `lease` keeps the grant and stops the command when it dies.
fn acquire(args: &[OsString], lease: bool) -> ExitCode {
    let Some(split) = args.iter().position(|arg| arg == "--") else {
        return usage();
    };
    let (mut gb, mut timeout) = (None, None);
    for pair in args[..split].chunks(2) {
        match pair {
            [flag, value] if flag == "--gb" => {
                let Some(value) = value
                    .to_str()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|gb| *gb > 0.0 && gb.is_finite())
                else {
                    return usage();
                };
                gb = Some(value);
            }
            [flag, value] if flag == "--timeout" => {
                let Some(value) = value
                    .to_str()
                    .and_then(|value| value.parse::<jiff::SignedDuration>().ok())
                    .and_then(|value| Duration::try_from(value).ok())
                else {
                    return usage();
                };
                timeout = Some(value);
            }
            _ => return usage(),
        }
    }
    let command = &args[split + 1..];
    let gb = gb.unwrap_or(GATE_GB);
    let Some((program, rest)) = command.split_first() else {
        return usage();
    };
    let failed = |error: String| {
        eprintln!("priory memory: {error}; the command did not run");
        ExitCode::from(GATE_FAILED)
    };
    let acquired = gate(&Gate::Acquire {
        bytes: (gb * f64::from(1u32 << 30)) as u64,
        pid: std::process::id(),
        session: std::env::var("PRIORY_SESSION").ok(),
        command: command.join(" ".as_ref()).to_string_lossy().into_owned(),
    });
    let mut reply = match acquired {
        Ok(reply) => reply,
        Err(error) => return failed(error),
    };
    let (id, _) = reply.split_once(' ').unwrap_or_default();
    let grant: i64 = id.parse().expect("the daemon replies with the grant id");
    let timeout = timeout.unwrap_or(GATE_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let mut said = String::new();
    while !reply.ends_with(" held") {
        if said != reply {
            eprintln!("priory memory: grant {reply} for {gb} GB");
            said = reply.clone();
        }
        if Instant::now() >= deadline {
            let _ = gate(&Gate::Release { grant });
            return failed(format!("grant {grant} still waits after {timeout:?}"));
        }
        std::thread::sleep(Duration::from_millis(250));
        reply = match gate(&Gate::Poll { grant }) {
            Ok(reply) => reply,
            Err(error) => return failed(error),
        };
    }
    let status = if lease {
        leased(program, rest)
    } else {
        match held(program, rest, grant) {
            Ok(status) => status,
            Err(error) => {
                let _ = gate(&Gate::Release { grant });
                return failed(error);
            }
        }
    };
    if let Err(error) = gate(&Gate::Release { grant }) {
        eprintln!(
            "priory memory: grant {grant} stays until the daemon sees this process end: {error}"
        );
    }
    match status {
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt;
            let code = status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal));
            ExitCode::from(code.unwrap_or(1) as u8)
        }
        Err(error) => {
            eprintln!("priory memory: {}: {error}", program.to_string_lossy());
            ExitCode::from(127)
        }
    }
}

/// Runs a command that holds `grant` from before it starts: a shell waits for a go-ahead on a pipe,
/// the daemon takes its pid, and only then it execs the command. A holder that dies first closes
/// the pipe, so the command never runs without its grant.
fn held(
    program: &OsString,
    rest: &[OsString],
    grant: i64,
) -> Result<std::io::Result<std::process::ExitStatus>, String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let (reader, mut writer) = std::io::pipe().map_err(|error| error.to_string())?;
    // The shell opens the pipe's read end through /proc by this pid, not $PPID, which a dead holder turns into init, and which keeps the command's stdin as it was.
    let wait = format!(
        "read -r _ <\"/proc/{}/fd/{}\" && exec \"$@\"",
        std::process::id(),
        reader.as_raw_fd()
    );
    let mut child = match Command::new("sh")
        .args(["-c", &wait, "sh"])
        .arg(program)
        .args(rest)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Ok(Err(error)),
    };
    let taken = gate(&Gate::Run {
        grant,
        pid: child.id(),
    });
    if let Err(error) =
        taken.and_then(|_| writer.write_all(b"\n").map_err(|error| error.to_string()))
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    drop(writer);
    Ok(child.wait())
}

/// Runs a leased command in the process group of a guard that stops the group once its pipe from
/// this process closes, however this process ends.
fn leased(program: &OsString, rest: &[OsString]) -> std::io::Result<std::process::ExitStatus> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let guard = Command::new("sh")
        .args([
            "-c",
            "trap '' TERM; read _; kill -TERM -$$; sleep 5; kill -KILL -$$",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| std::io::Error::other(format!("its guard did not start: {error}")))?;
    let status = Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .process_group(guard.id() as i32)
        .spawn()
        .and_then(|mut child| child.wait());
    // Dropping the guard closes its pipe, so it also stops what the command left behind.
    drop(guard);
    status
}

/// Sends `gate` to the daemon and returns its reply.
fn gate(gate: &Gate) -> Result<String, String> {
    let line = serde_json::to_string(gate).expect("a gate message serializes");
    runtime::call(&runtime::home(), &line, runtime::REPLY).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    })
}

/// `priory gate`, from a run's pane: runs the factory's gate in the run's task worktree at its head,
/// records the run with the daemon, and exits with the gate's code, or 1 when a signal killed it.
fn project_gate() -> ExitCode {
    let Some(session) = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
    else {
        eprintln!("priory gate: PRIORY_SESSION is unset; run it from a run's pane");
        return ExitCode::FAILURE;
    };
    let call = |line: &proof::Line| {
        let line = serde_json::to_string(line).expect("a gate line serializes");
        runtime::call(&runtime::home(), &line, runtime::REPLY).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
        })
    };
    let started = call(&proof::Line::Start {
        session: session.clone(),
    })
    .and_then(|reply| {
        serde_json::from_str::<proof::Started>(&reply)
            .map_err(|error| format!("unreadable start {reply:?}: {error}"))
    });
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            eprintln!("priory gate: {error}");
            return ExitCode::FAILURE;
        }
    };
    // The gate tests what is checked out, so a record for the branch head needs the two to match.
    let checked_out = || {
        priory_runtime::runtime::git(Path::new(&started.worktree))
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let away = |at: String| {
        eprintln!(
            "priory gate: the worktree is at {at}, not at the head of its branch, {}: check out the branch, then run it again",
            started.commit
        );
        ExitCode::FAILURE
    };
    let at = checked_out();
    if at != started.commit {
        return away(at);
    }
    let clock = Instant::now();
    let status = Command::new("sh")
        .args(["-c", &started.command])
        .current_dir(&started.worktree)
        .status();
    let exit = match status {
        // shortcut: a gate killed by a signal records -1, the way a shell would not; fine for a proof.
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            eprintln!("priory gate: cannot run sh: {error}");
            return ExitCode::FAILURE;
        }
    };
    let at = checked_out();
    if at != started.commit {
        return away(at);
    }
    let recorded = call(&proof::Line::Ran {
        session,
        commit: started.commit,
        exit,
        duration_ms: clock.elapsed().as_millis() as u64,
    });
    match recorded {
        Ok(text) => eprintln!("priory gate: {text}"),
        Err(error) => {
            eprintln!("priory gate: the gate ran, and the daemon did not record it: {error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::from(u8::try_from(exit).unwrap_or(1))
}

/// `priory evidence add FILE --caption TEXT`, from a run's pane, sends a cast to the daemon, which
/// checks and stores it. `priory evidence TASK [--json]` lists a task's items.
fn evidence(args: Vec<String>) -> ExitCode {
    match &args[..] {
        [add, file, flag, caption] | [add, flag, caption, file]
            if add == "add" && flag == "--caption" =>
        {
            evidence_add(file, caption)
        }
        [task] | [task, _] if args.len() == 1 || args[1] == "--json" => {
            let Ok(id) = task.parse::<i64>() else {
                return usage();
            };
            let listed = state::open().and_then(|db| {
                let found: bool = db
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                if !found {
                    return Err(format!("no task {id}"));
                }
                evidence::list(&db, id).map_err(|error| error.to_string())
            });
            let items = match listed {
                Ok(items) => items,
                Err(error) => return exit("evidence", Err(error)),
            };
            if args.len() == 2 {
                state::emit(&json!({ "evidence": items }).to_string());
                return ExitCode::SUCCESS;
            }
            state::emit("id\trun\tkind\tcaption\tcommit\tbytes\tsha256\tcreated\tapproved");
            for item in items.as_array().into_iter().flatten() {
                let cell = |name: &str| match &item[name] {
                    Value::String(text) => text.clone(),
                    Value::Bool(true) => "yes".to_owned(),
                    Value::Bool(false) | Value::Null => "-".to_owned(),
                    other => other.to_string(),
                };
                let row = [
                    "id",
                    "run",
                    "kind",
                    "caption",
                    "commit_sha",
                    "bytes",
                    "sha256",
                    "created",
                    "approved",
                ]
                .map(cell);
                state::emit(&row.join("\t"));
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

/// Sends the cast `file` with `caption` to the daemon for the run whose pane this is.
fn evidence_add(file: &str, caption: &str) -> ExitCode {
    let Some(session) = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
    else {
        eprintln!("priory evidence add: PRIORY_SESSION is unset; run it from a run's pane");
        return ExitCode::FAILURE;
    };
    let file = match std::path::absolute(file) {
        Ok(file) => file.to_string_lossy().into_owned(),
        Err(error) => return exit("evidence add", Err(format!("{file}: {error}"))),
    };
    let line = json!({ "evidence": evidence::Add { session, file, caption: caption.to_owned() } });
    let sent =
        runtime::call(&runtime::home(), &line.to_string(), runtime::REPLY).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
        });
    reply("evidence add", sent)
}

/// Sends `priory answer ASK TEXT --revision N [--run]` to the daemon, which applies it. `--run`
/// confirms an answer that runs the ask's command.
fn answer(mut args: Vec<String>) -> ExitCode {
    let run = args
        .iter()
        .take_while(|arg| *arg != "--")
        .position(|arg| arg == "--run");
    if let Some(at) = run {
        args.remove(at);
    }
    let Some((revision, rest)) = revised(args) else {
        return usage();
    };
    let (dir, ask, text) = match &rest[..] {
        [ask, text] => (None, ask, text),
        [dir, ask, text] => (Some(dir.as_str()), ask, text),
        _ => return usage(),
    };
    let Some(revision) = revision else {
        let run = if run.is_some() { "--run " } else { "" };
        let rest = [dir, Some(ask.as_str()), Some(text.as_str())];
        return unrevised("answer", ask, |revision| {
            format!("priory answer --revision {revision} {run}{}", words(&rest))
        });
    };
    // This machine's ask by its number or name, else another runner's, which goes through the log
    // and applies when that runner reads it.
    let local = ask_id(ask);
    let remote = local
        .is_err()
        .then(|| ask.split_once(':'))
        .flatten()
        .and_then(|(runner, id)| Some((runner, id.parse::<i64>().ok()?)));
    let ask = match (remote, local) {
        (Some((_, id)), _) | (None, Ok(id)) => id,
        (None, Err(error)) => return exit("answer", Err(error)),
    };
    let answer = tools::Answer {
        ask,
        revision,
        text: text.clone(),
        run: run.is_some(),
        by: "cli".to_owned(),
    };
    if let Some((runner, _)) = remote {
        let Some(factory) = load(&factory::folder(dir)) else {
            return ExitCode::FAILURE;
        };
        let label = format!("{runner}:{}", answer.ask);
        let mirrored = match remote_ask(runner, answer.ask, answer.revision) {
            Ok(mirrored) => mirrored,
            Err(refused) => return exit("answer", Err(refused)),
        };
        // The command runs on the ask's own runner, so the person confirms it knowing where.
        let runs = mirrored.as_ref().is_some_and(|ask| {
            ask["command"]["runs"]
                .as_array()
                .is_some_and(|runs| runs.iter().any(|option| *option == answer.text))
        });
        if runs && !answer.run {
            return exit(
                "answer",
                Err(format!(
                    "ask {label} runs its command when answered {:?}. Runs on {runner}: answer again with --run to run it",
                    answer.text
                )),
            );
        }
        let sent = runners::answer(&factory, runner, &answer).map(|submission| {
            let on = if runs { format!(" Runs on {runner}.") } else { String::new() };
            format!(
                "answer to ask {label} sent to {runner} as submission {submission}: it applies when {runner} reads the log.{on}"
            )
        });
        return reply("answer", sent);
    }
    let line = serde_json::to_string(&answer).expect("an answer serializes");
    send("answer", &line)
}

/// Another runner's ask as this machine's mirror holds it, refused when the mirror shows it closed
/// or at another revision. An ask the mirror lacks, or a machine without a state file, goes to its
/// runner unchecked, which checks it again anyway.
fn remote_ask(runner: &str, ask: i64, revision: i64) -> Result<Option<serde_json::Value>, String> {
    let Ok(db) = state::open() else {
        return Ok(None);
    };
    let mirrored = runners::mirrored(&db, runner, ask).map_err(|error| error.to_string())?;
    if let Some(stale) = mirrored
        .as_ref()
        .and_then(|ask| runners::stale(ask, revision))
    {
        return Err(stale);
    }
    Ok(mirrored)
}

/// The generation to send with an action on runner `runner`'s task of `key`: `generation`, checked
/// against this machine's claims, else the one they hold. A machine without a state file sends the
/// one the person named unchecked, which the runner checks again anyway.
fn remote_generation(runner: &str, key: &str, generation: Option<i64>) -> Result<i64, String> {
    let Ok(db) = state::open() else {
        return generation.ok_or_else(|| {
            format!("this machine knows no generation of task {runner}:{key}: pass --generation N")
        });
    };
    match runners::task_generation(&db, runner, key, generation) {
        Ok(Ok(generation)) => Ok(generation),
        Ok(Err(runners::Refused::Unknown(why) | runners::Refused::Stale(why))) => Err(why),
        Err(error) => Err(error.to_string()),
    }
}

/// Splits `<runner>:<ask>` when `ask` names no ask of this machine.
fn remote_id(ask: &str, local: &Result<i64, String>) -> Option<(String, i64)> {
    let (runner, id) = ask.split_once(':').filter(|_| local.is_err())?;
    Some((runner.to_owned(), id.parse().ok()?))
}

/// `priory memories [--all] [--json]` lists the memory candidates that wait for a person, or every
/// one; `priory memories (accept | drop) [DIR] CANDIDATE` publishes a person's verdict on the log.
fn memories(args: Vec<String>) -> ExitCode {
    let verdict = match args.first().map(String::as_str) {
        Some("accept") => Some(true),
        Some("drop") => Some(false),
        _ => None,
    };
    if let Some(accept) = verdict {
        let (dir, id) = match &args[1..] {
            [id] => (None, id),
            [dir, id] => (Some(dir.as_str()), id),
            _ => return usage(),
        };
        let Some(factory) = load(&factory::folder(dir)) else {
            return ExitCode::FAILURE;
        };
        let decided = state::read_only().and_then(|db| {
            priory_core::memories::decide(factory.log(), &db, accept, id).map_err(String::from)
        });
        return reply("memories", decided);
    }
    let (mut all, mut json, mut dir) = (false, false, None);
    for arg in &args {
        match arg.as_str() {
            "--all" => all = true,
            "--json" => json = true,
            _ if dir.is_none() && !arg.starts_with('-') => dir = Some(arg),
            _ => return usage(),
        }
    }
    // With the factory's folder, each candidate says what memory of its name runs get now.
    let factory = match factory::named_folder(dir.map(String::as_str)) {
        Some(dir) => match load(&dir) {
            Some(factory) => Some(factory),
            None => return ExitCode::FAILURE,
        },
        None => None,
    };
    let listed = state::read_only().and_then(|db| {
        let mut candidates =
            priory_core::memories::list(&db, all).map_err(|error| error.to_string())?;
        if let Some(factory) = &factory {
            priory_core::memories::Held::of(factory)
                .mark(&db, &runtime::home(), &mut candidates)
                .map_err(|error| error.to_string())?;
        }
        Ok(candidates)
    });
    let candidates = match listed {
        Ok(candidates) => candidates,
        Err(error) => return exit("memories", Err(error)),
    };
    if json {
        println!("{}", serde_json::json!({ "candidates": candidates }));
        return ExitCode::SUCCESS;
    }
    println!("id\tname\tkind\tstate\ttask\trunner\twritten");
    for candidate in candidates {
        let field = |name: &str| candidate[name].as_str().unwrap_or("-").to_owned();
        let kind = if candidate["forget"] == true {
            "forget"
        } else {
            "remember"
        };
        println!(
            "{}\t{}\t{kind}\t{}\t{}\t{}\t{}",
            field("id"),
            field("name"),
            field("state"),
            field("task"),
            field("runner"),
            field("written")
        );
    }
    ExitCode::SUCCESS
}

fn reply(verb: &str, done: Result<String, String>) -> ExitCode {
    match done {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => exit(verb, Err(error)),
    }
}

/// Sends `priory dismiss [--expected N] ASK` to the daemon, which closes the ask unanswered, or
/// publishes `priory dismiss --revision N [DIR] RUNNER:ASK` for that runner to apply.
fn dismiss(args: Vec<String>) -> ExitCode {
    let Some((revision, rest)) = revised(args) else {
        return usage();
    };
    let (dir, ask) = match &rest[..] {
        [ask] => (None, ask),
        [dir, ask] => (Some(dir.as_str()), ask),
        _ => return usage(),
    };
    let local = ask_id(ask);
    if let Some((runner, id)) = remote_id(ask, &local) {
        let Some(revision) = revision else {
            let rest = [dir, Some(ask.as_str())];
            return unrevised("dismiss", ask, |revision| {
                format!("priory dismiss --revision {revision} {}", words(&rest))
            });
        };
        let Some(factory) = load(&factory::folder(dir)) else {
            return ExitCode::FAILURE;
        };
        let sent = remote_ask(&runner, id, revision)
            .and_then(|_| runners::dismiss(&factory, &runner, id, revision))
            .map(|submission| {
                format!(
                    "dismiss of ask {runner}:{id} sent to {runner} as submission {submission}: it applies when {runner} reads the log"
                )
            });
        return reply("dismiss", sent);
    }
    let dismiss = match local {
        Ok(dismiss) => dismiss,
        Err(error) => return exit("dismiss", Err(error)),
    };
    let line =
        serde_json::to_string(&tools::Dismiss { dismiss, revision }).expect("a dismiss serializes");
    send("dismiss", &line)
}

/// Refuses an `answer` or `dismiss` of `ask` that lacks the ask's revision, as an argument error.
/// While this machine knows the ask open, it prints the command to run, from `command` given the
/// ask's current revision.
fn unrevised(verb: &str, ask: &str, command: impl FnOnce(i64) -> String) -> ExitCode {
    let run = current_revision(ask)
        .map(|revision| format!("; run: {}", command(revision)))
        .unwrap_or_default();
    eprintln!(
        "priory {verb}: ask {ask} needs --revision N, the revision `priory asks` prints for it{run}"
    );
    ExitCode::from(2)
}

/// The revision of `ask` while it is open: this machine's ask in its state, or another runner's
/// in this machine's mirror. None when neither holds it open.
fn current_revision(ask: &str) -> Option<i64> {
    let db = state::read_only().ok()?;
    let local = match ask.bytes().all(|byte| byte.is_ascii_digit()) {
        true => ask.parse().map_err(|_| String::new()),
        false => tools::ask_named(&db, ask),
    };
    if let Some((runner, id)) = remote_id(ask, &local) {
        let mirrored = runners::mirrored(&db, &runner, id).ok()??;
        return (mirrored["state"] == "open")
            .then(|| mirrored["revision"].as_i64())
            .flatten();
    }
    tools::open_revision(&db, local.ok()?).ok()?
}

/// `words` as one shell command line's arguments: each quoted when the shell would split or expand
/// it, after `--` when one starts with `-`.
fn words(words: &[Option<&str>]) -> String {
    let words: Vec<&str> = words.iter().flatten().copied().collect();
    let quoted = words.iter().map(|word| {
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c));
        match plain {
            true => (*word).to_owned(),
            false => format!("'{}'", word.replace('\'', "'\\''")),
        }
    });
    let dashes = words.iter().any(|word| word.starts_with('-'));
    dashes
        .then(|| "--".to_owned())
        .into_iter()
        .chain(quoted)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits `[--revision N | --expected N] [--] ARGS` into the revision and the other arguments.
fn revised(args: Vec<String>) -> Option<(Option<i64>, Vec<String>)> {
    let (mut revision, mut rest, mut args) = (None, Vec::new(), args.into_iter());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                rest.extend(args);
                break;
            }
            "--revision" | "--expected" if revision.is_none() => {
                revision = Some(args.next()?.parse().ok()?);
            }
            _ => rest.push(arg),
        }
    }
    Some((revision, rest))
}

/// Takes `--expected N` out of a task verb's arguments, which carry no free text.
fn expected(mut args: Vec<String>) -> Option<(Option<i64>, Vec<String>)> {
    let Some(at) = args.iter().position(|arg| arg == "--expected") else {
        return Some((None, args));
    };
    let revision = args.get(at + 1)?.parse().ok()?;
    args.drain(at..at + 2);
    Some((Some(revision), args))
}

/// An ask's number, or the number of the outside ask that `priory ask` named so.
fn ask_id(ask: &str) -> Result<i64, String> {
    // `priory ask` refuses an all-digit ID, so only one is a number.
    if let Some(id) = ask
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| ask.parse().ok())
        .flatten()
    {
        return Ok(id);
    }
    tools::ask_named(&state::open()?, ask)
}

/// Stores or revises an ask from a session Priory did not start, from Fleet's JSON shape.
fn ask(name: &str, fields: &str) -> ExitCode {
    let stored = serde_json::from_str(fields)
        .map_err(|error| format!("ask fields must be a JSON object: {error}"))
        .map(|mut fields: serde_json::Value| {
            if let (Some(fields), Some(session)) = (fields.as_object_mut(), asking_session()) {
                fields.entry("session").or_insert(session.into());
            }
            fields
        })
        .and_then(|fields| {
            let mut db = state::open()?;
            tools::ask_outside(&mut db, name, &fields).map_err(|error| error.to_string())?
        })
        .map(|ignored| {
            for field in ignored {
                eprintln!("priory ask: ignored unknown field {field}");
            }
            println!("{name}");
        });
    exit("ask", stored)
}

/// The hc terminal that runs `priory ask`, then the Claude session in it when there is one, so the
/// answer can reach it, or resume it once the terminal closed.
fn asking_session() -> Option<String> {
    let terminal = std::env::var("HC_TERMINAL_ID")
        .ok()
        .filter(|id| !id.is_empty())?;
    Some(match std::env::var("CLAUDE_CODE_SESSION_ID") {
        Ok(claude) if !claude.is_empty() => format!("{terminal} {claude}"),
        _ => terminal,
    })
}

/// Sets a task's TL;DR by hand.
/// Prints `priory task TASK --json`: the body `/api/tasks/TASK` returns.
/// Prints `priory inbox --unrouted`: each group of a resource a task owned, with that task, its
/// events and the command that routes the next one, then each request a `pre` script dropped, then
/// each task that kept events no agent read, then one count per source for the rest.
fn print_unrouted(unrouted: &serde_json::Value) {
    let events = |count: u64, what: &str| {
        priory_factory::plural(
            count as usize,
            &format!("{what}event"),
            &format!("{what}events"),
        )
    };
    for group in unrouted["owned"].as_array().into_iter().flatten() {
        let task = &group["task"];
        println!(
            "{}: {}, task {} {}",
            group["resource"].as_str().unwrap_or_default(),
            events(group["events"].as_array().map_or(0, Vec::len) as u64, ""),
            task["id"],
            task["state"].as_str().unwrap_or_default()
        );
        for event in group["events"].as_array().into_iter().flatten() {
            println!(
                "  {}  {}  {}  {}",
                event["seq"],
                event["type"].as_str().unwrap_or_default(),
                event["subject"].as_str().unwrap_or("-"),
                event["line"].as_str().unwrap_or("")
            );
        }
        println!(
            "  {}",
            group["follow_up"]
                .as_str()
                .unwrap_or("no command can key a new task by it: start one from its thread")
        );
    }
    let dropped = unrouted["dropped"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if !dropped.is_empty() {
        println!("Dropped by a pre script, each judged once:");
    }
    for drop in dropped {
        let why = drop["reason"].as_str().or(drop["failed"].as_str());
        println!(
            "  {}  {}  {}  dropped by {}{}",
            drop["seq"],
            drop["trigger"].as_str().unwrap_or_default(),
            drop["key"].as_str().unwrap_or("-"),
            drop["script"].as_str().unwrap_or_default(),
            why.map(|why| format!(": {why}")).unwrap_or_default()
        );
    }
    for group in unrouted["unheard"].as_array().into_iter().flatten() {
        let task = &group["task"];
        println!(
            "task {} {} kept {} no agent read:",
            task["id"],
            task["state"].as_str().unwrap_or_default(),
            events(group["events"].as_array().map_or(0, Vec::len) as u64, ""),
        );
        for event in group["events"].as_array().into_iter().flatten() {
            println!(
                "  {}  {}  {}  {}",
                event["seq"],
                event["type"].as_str().unwrap_or_default(),
                event["subject"].as_str().unwrap_or("-"),
                event["line"].as_str().unwrap_or("")
            );
        }
        println!(
            "  {}",
            group["follow_up"]
                .as_str()
                .unwrap_or("its next run reads them")
        );
    }
    let other: Vec<String> = unrouted["other"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|source| {
            let source_name = format!("{} ", source["source"].as_str().unwrap_or_default());
            events(source["events"].as_u64().unwrap_or(0), &source_name)
        })
        .collect();
    if !other.is_empty() {
        println!("No task ever owned: {}.", other.join(", "));
    }
}

/// `priory task ID`: the task's key, workflow, step and state, then its Inbox section.
fn task_text(task: &str) -> ExitCode {
    let Ok(id) = task.parse() else {
        return usage();
    };
    let shown = state::open().and_then(|db| {
        let task = listing::revised(&db, "task", |db| listing::task(db, id))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no task {id}"))?;
        let text = |value: &serde_json::Value| value.as_str().unwrap_or("-").to_owned();
        println!("task {id}  {}", text(&task["key"]));
        println!(
            "workflow {}, step {}, {}",
            text(&task["workflow"]),
            text(&task["step"]),
            text(&task["state"])
        );
        println!("\nInbox");
        for event in task["inbox"].as_array().into_iter().flatten() {
            println!(
                "  {}  {}  {}  {}  {}",
                event["seq"],
                text(&event["time"]),
                text(&event["type"]),
                text(&event["subject"]),
                handled(event, id)
            );
        }
        Ok(())
    });
    exit("task", shown)
}

/// What one event of a task's inbox did: its outcome, what became of its prompt, the wait it fired
/// and the task routing gave it to when that is another task.
fn handled(event: &serde_json::Value, task: i64) -> String {
    let mut text = event["outcome"].as_str().unwrap_or("-").to_owned();
    let prompt = match event["prompt"].as_str() {
        Some("confirmed") => "delivered as prompt",
        Some("queued") => "still queued",
        Some("attempted") => "being typed",
        Some("uncertain") => "typed, unconfirmed",
        Some("withdrawn") => "prompt withdrawn",
        _ if text == "kept" && event["task"].as_i64() == Some(task) => "for its next run",
        _ => "",
    };
    if !prompt.is_empty() {
        text = format!("{text}, {prompt}");
    }
    if let Some(wait) = event["wait"].as_i64() {
        text = format!("{text}, fired wait {wait}");
    }
    if let Some(other) = event["task"].as_i64().filter(|other| *other != task) {
        text = format!("{text} (routed to task {other})");
    }
    text
}

/// Prints the comments and reviews from outside the repository as a group of their own: its count
/// and one line per event, with who wrote it. No command routes them: they reach no task.
fn print_outsider(events: &serde_json::Value) {
    let Some(events) = events.as_array().filter(|events| !events.is_empty()) else {
        return;
    };
    println!(
        "from outside the repository: {}",
        plural(events.len(), "event", "events")
    );
    for event in events {
        println!(
            "  {}  {}  {}  {}",
            event["seq"],
            event["type"].as_str().unwrap_or_default(),
            event["subject"].as_str().unwrap_or("-"),
            event["actor"].as_str().unwrap_or("-")
        );
    }
}

fn task_json(task: &str) -> ExitCode {
    let Ok(id) = task.parse() else {
        return usage();
    };
    let shown = state::open().and_then(|db| {
        match listing::revised(&db, "task", |db| listing::task(db, id))
            .map_err(|error| error.to_string())?
        {
            Some(task) => {
                state::emit(&task.to_string());
                Ok(())
            }
            None => Err(format!("no task {id}")),
        }
    });
    exit("task", shown)
}

/// `priory runs --json`: every run with its task, outcome, model and tokens, as `/api/runs?all=1`.
fn runs_json() -> ExitCode {
    let shown = state::open().and_then(|db| {
        let runs = listing::revised(&db, "runs", |db| listing::recent_runs(db, None).map(Some))
            .map_err(|error| error.to_string())?
            .expect("a listing always reads");
        state::emit(&runs.to_string());
        Ok(())
    });
    exit("runs", shown)
}

/// `priory subscribe` or `priory unsubscribe`: starts or ends `task`'s subscription to `resource`.
/// `expected` refuses a subscribe once the state moved past that revision.
fn subscription(verb: &str, task: &str, resource: &str, expected: Option<i64>) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let Some(resource) = subscribe::resource(resource) else {
        eprintln!(
            "priory {verb}: `{resource}` is not a resource: name github:owner/name#7, linear:ENG-7 or slack:C0123:1763500000.000100"
        );
        return ExitCode::from(2);
    };
    let done = state::open().and_then(|mut db| {
        let done = match verb {
            "subscribe" => subscribe::subscribe(&mut db, task, &resource, expected),
            _ => subscribe::unsubscribe(&mut db, task, &resource),
        };
        done.map_err(|error| error.to_string())?
            .map(|text| println!("{text}"))
    });
    exit(verb, done)
}

/// `priory tasks --graph [TASK] [--mermaid]`.
fn graph(args: &[String]) -> ExitCode {
    let (mermaid, rest): (Vec<_>, Vec<_>) = args.iter().partition(|arg| *arg == "--mermaid");
    let root = match &rest[..] {
        [] => None,
        [task] => match task.parse() {
            Ok(task) => Some(task),
            Err(_) => return usage(),
        },
        _ => return usage(),
    };
    let shown = state::open().and_then(|db| {
        relate::graph(&db, root, !mermaid.is_empty())
            .map_err(|error| error.to_string())?
            .map(|graph| print!("{graph}"))
    });
    exit("tasks", shown)
}

/// `priory label [DIR] TASK wrong (--evidence URL [--happened DATE] | --clear)`: a person marks a
/// task wrong, or clears the mark. It reads the factory for the records' name and revision.
fn label(args: Vec<String>) -> ExitCode {
    let (mut evidence, mut happened, mut clear) = (None, None, false);
    let mut words = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--clear" => clear = true,
            "--evidence" => match args.next() {
                Some(url) => evidence = Some(url),
                None => return usage(),
            },
            "--happened" => match args.next() {
                Some(date) => happened = Some(date),
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => words.push(arg),
        }
    }
    let (dir, task) = match &words[..] {
        [task, kind] if kind == "wrong" => (None, task),
        [dir, task, kind] if kind == "wrong" => (Some(dir.as_str()), task),
        _ => return usage(),
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    if clear != evidence.is_none() || (clear && happened.is_some()) {
        return usage();
    }
    let Some(factory) = load(&factory::folder(dir)) else {
        return ExitCode::FAILURE;
    };
    let done = intake::now().and_then(|now| {
        let mut db = state::open()?;
        let text = match &evidence {
            Some(evidence) => {
                label::mark(&mut db, &factory, task, evidence, happened.as_deref(), now)?
            }
            None => label::clear(&mut db, &factory, task, now)?,
        };
        println!("{text}");
        Ok(())
    });
    exit("label", done)
}

/// `priory relate TASK KIND OTHER [--remove]`: records or removes how two tasks relate.
fn relate(mut args: Vec<String>) -> ExitCode {
    let remove = args.iter().position(|arg| arg == "--remove");
    if let Some(at) = remove {
        args.remove(at);
    }
    let [task, kind, other] = &args[..] else {
        return usage();
    };
    let (Ok(task), Ok(other)) = (task.parse(), other.parse()) else {
        return usage();
    };
    let by = std::env::var("USER").unwrap_or_else(|_| "person".to_owned());
    let done = state::open().and_then(|mut db| {
        let done = match remove {
            Some(_) => relate::unrelate(&mut db, task, kind, other),
            None => relate::relate(&mut db, task, kind, other, &by),
        };
        done.map_err(|error| error.to_string())?
            .map(|text| println!("{text}"))
    });
    exit("relate", done)
}

fn tldr(task: &str, text: &str) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let set = state::open()
        .and_then(|db| {
            let counter = search::counter(&db, &runtime::home());
            search::set(&db, task, text, counter.as_ref())
        })
        .map(|done| println!("{done}"));
    exit("tldr", set)
}

/// Sends `priory progress TASK JSON` to the daemon.
fn progress(task: &str, fields: &str) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let progress = match serde_json::from_str(fields) {
        Ok(progress) => progress,
        Err(error) => return exit("progress", Err(format!("{fields}: {error}"))),
    };
    let line = serde_json::to_string(&progress::Put { task, progress }).expect("a put serializes");
    send("progress", &line)
}

/// Prints the tasks that match a query, best first.
fn search(args: Vec<String>) -> ExitCode {
    let (mut query, mut states, mut limit) = (None, Vec::new(), search::LIMIT);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match (arg.as_str(), query.is_some()) {
            ("--state", _) => match args.next() {
                Some(value) => states.extend(value.split(',').map(str::to_owned)),
                None => return usage(),
            },
            ("--limit", _) => match args.next().map(|value| value.parse()) {
                Some(Ok(value)) => limit = value,
                _ => return usage(),
            },
            (flag, _) if flag.starts_with("--") => return usage(),
            (_, true) => return usage(),
            (_, false) => query = Some(arg),
        }
    }
    let Some(query) = query else {
        return usage();
    };
    let mut embedder = embed::Lazy::new(&runtime::home(), false);
    let printed = state::open()
        .and_then(|db| search::search(&db, &query, &states, limit, &mut embedder))
        .map(|found| {
            if let Some(why) = found.unavailable {
                eprintln!("priory search: embeddings are unavailable, so this matched keywords only: {why}");
            }
            print!("{}", found.table);
        });
    exit("search", printed)
}

/// Sets a task's band and its place in the band's person order.
fn move_task(args: Vec<String>) -> ExitCode {
    let Some((expected, args)) = expected(args) else {
        return usage();
    };
    let mut args = args.into_iter();
    let Some(Ok(task)) = args.next().map(|task| task.parse()) else {
        return usage();
    };
    let (mut band, mut place) = (None, None);
    while let Some(flag) = args.next() {
        let mut task = || args.next().and_then(|task| task.parse().ok());
        let parsed = match flag.as_str() {
            "--band" if band.is_none() => {
                band = args.next();
                band.is_some()
            }
            _ if place.is_some() => false,
            "--top" => {
                place = Some(state::Place::Top);
                true
            }
            "--before" => {
                place = task().map(state::Place::Before);
                place.is_some()
            }
            "--after" => {
                place = task().map(state::Place::After);
                place.is_some()
            }
            _ => false,
        };
        if !parsed {
            return usage();
        }
    }
    if band.is_none() && place.is_none() {
        return usage();
    }
    let moved = state::open()
        .and_then(|mut db| state::move_task(&mut db, task, band.as_deref(), place, expected));
    exit("move", moved)
}

/// Sends `priory hold TASK --reason TEXT [CONDITION]` to the daemon, which applies it.
fn hold(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter();
    let Some(task) = args.next() else {
        return usage();
    };
    let mut hold = factory::Hold::default();
    let (mut reason, mut task) = (None, vec![task]);
    while let Some(flag) = args.next() {
        if flag == "--blockers" {
            hold.blockers = true;
            continue;
        }
        let Some(value) = args.next() else {
            return usage();
        };
        match flag.as_str() {
            "--reason" => reason = Some(value),
            "--until" if hold.until.is_none() => hold.until = Some(value),
            "--after" => match value.parse() {
                Ok(other) => hold.after.push(other),
                Err(error) => {
                    eprintln!("priory hold: {error}");
                    return ExitCode::FAILURE;
                }
            },
            "--merge" => hold.merge.push(value),
            "--if-closed" => hold.if_closed = Some(value),
            "--expected" if task.len() == 1 => task.extend([flag, value]),
            _ => return usage(),
        }
    }
    let Some(reason) = reason else {
        return usage();
    };
    hold.reason = reason;
    act("hold", task, Some(hold))
}

/// Sends `priory resume`, `cancel`, `revive TASK [--step STEP]`, `release`, `say TASK TEXT` or `hold` for one task to
/// the daemon, which applies it. Each but `hold` also takes `[DIR] RUNNER:KEY [--generation N]`,
/// another runner's task, which it publishes on the log for that runner to apply.
fn act(act: &str, mut args: Vec<String>, hold: Option<factory::Hold>) -> ExitCode {
    // Taken first, so a text such as `--step` stays text.
    let text = match act {
        "say" if (2..=5).contains(&args.len()) => args.pop(),
        "say" => return usage(),
        _ => None,
    };
    let generation = match args.iter().position(|arg| arg == "--generation") {
        Some(flag) if hold.is_none() => match args.get(flag + 1).map(|value| value.parse()) {
            Some(Ok(generation)) => {
                args.drain(flag..flag + 2);
                Some(generation)
            }
            _ => return usage(),
        },
        Some(_) => return usage(),
        None => None,
    };
    let (expected, mut args) = match act {
        "hold" | "release" | "cancel" => match expected(args) {
            Some(split) => split,
            None => return usage(),
        },
        _ => (None, args),
    };
    let step = match args.iter().position(|arg| arg == "--step") {
        Some(flag) if act == "revive" && flag + 1 < args.len() => args.drain(flag..flag + 2).nth(1),
        Some(_) => return usage(),
        None => None,
    };
    let remote = match &args[..] {
        [task] if task.parse::<i64>().is_err() => Some((None, task)),
        [dir, task] => Some((Some(dir.as_str()), task)),
        _ => None,
    };
    if let Some((dir, task)) = remote.filter(|_| hold.is_none()) {
        let Some((runner, key)) = task.split_once(':') else {
            return usage();
        };
        if expected.is_some() {
            return exit(
                act,
                Err(format!(
                    "task {task} is another runner's: name its --generation N, not --expected"
                )),
            );
        }
        let Some(factory) = load(&factory::folder(dir)) else {
            return ExitCode::FAILURE;
        };
        let sent = runners::known(&factory, runner)
            .and_then(|()| remote_generation(runner, key, generation))
            .and_then(|generation| {
                let sent = runners::TaskAct {
                    act,
                    runner,
                    key,
                    generation,
                    text: text.as_deref(),
                    step: step.as_deref(),
                    by: "cli",
                };
                runners::act(factory.log(), &sent).map_err(|why| why.said("task"))
            })
            .map(|submission| {
                format!(
                    "{act} of task {task} sent to {runner} as submission {submission}: it applies when {runner} reads the log"
                )
            });
        return reply(act, sent);
    }
    if generation.is_some() {
        return usage();
    }
    let [task] = &args[..] else {
        return usage();
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    let line = serde_json::to_string(&tools::Act {
        act: act.to_owned(),
        task,
        step,
        hold,
        text,
        expected,
    })
    .expect("an action serializes");
    send(act, &line)
}

/// Sends `line` to the daemon and prints its reply.
fn send(verb: &str, line: &str) -> ExitCode {
    let reply = runtime::call(&runtime::home(), line, runtime::REPLY).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    });
    match reply {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `priory check [DIR] [--explain [--json]]`.
fn check(args: Vec<OsString>) -> ExitCode {
    if args.iter().any(|arg| arg == "--repo") {
        return check_repo(args);
    }
    let (flags, dirs): (Vec<_>, Vec<_>) = args
        .iter()
        .partition(|arg| arg.to_str().is_some_and(|arg| arg.starts_with("--")));
    let flags: Vec<_> = flags.iter().filter_map(|flag| flag.to_str()).collect();
    let (explain, json) = match flags[..] {
        [] => (false, false),
        ["--explain"] => (true, false),
        ["--explain", "--json"] | ["--json", "--explain"] => (true, true),
        _ => return usage(),
    };
    let dir = match &dirs[..] {
        [] => factory::folder(None),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    if let (Some(name), Some(guide)) = (
        factory
            .settings
            .as_ref()
            .and_then(|s| s.value.writing.as_ref()),
        &factory.writing,
    ) && guide.len() > factory::WRITING_LIMIT
    {
        eprintln!(
            "warning: {name} is {} bytes: every agent's prompt carries it, so keep it under {}",
            guide.len(),
            factory::WRITING_LIMIT
        );
    }
    // Only here: a runner without bubblewrap blocks each jailed task, and runs the others.
    let missing = rules::missing_bwrap(&factory, priory_runtime::jail::on_path("bwrap"));
    if !missing.is_empty() {
        for problem in &missing {
            eprintln!("{problem}");
        }
        return ExitCode::FAILURE;
    }
    warn_of_stale_herdr_layout(&factory, &dir);
    for warning in priory_factory::rules::warnings(&factory) {
        eprintln!("warning: {warning}");
    }
    if explain {
        print_explanation(&factory, json);
        return ExitCode::SUCCESS;
    }
    let skills = match factory.skills.len() {
        0 => String::new(),
        n => format!(", {}", plural(n, "skill", "skills")),
    };
    let memories = match factory.memories.len() {
        0 => String::new(),
        1 => ", 1 memory".to_owned(),
        n => format!(", {n} memories"),
    };
    let scorers = match factory.scorers.len() {
        0 => String::new(),
        n => format!(", {}", plural(n, "scorer", "scorers")),
    };
    let benchmarks = match factory.benchmarks.len() {
        0 => String::new(),
        n => format!(", {}", plural(n, "benchmark", "benchmarks")),
    };
    println!(
        "ok: {}, {}, {}, {}{skills}{memories}{scorers}{benchmarks}",
        plural(factory.agents.len(), "agent", "agents"),
        plural(factory.triggers.len(), "trigger", "triggers"),
        plural(factory.workflows.len(), "workflow", "workflows"),
        plural(factory.runners.len(), "runner", "runners"),
    );
    ExitCode::SUCCESS
}

/// `priory check [DIR] --repo PATH [--json]`: scores what an unattended agent needs from the
/// repository at `PATH`, and the gate of the factory in `DIR` when given. It only reads.
fn check_repo(args: Vec<OsString>) -> ExitCode {
    let mut repo = None;
    let mut dir = None;
    let mut json = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--repo") if repo.is_none() => match args.next() {
                Some(path) => repo = Some(PathBuf::from(path)),
                None => return usage(),
            },
            Some("--json") if !json => json = true,
            Some(flag) if flag.starts_with("--") => return usage(),
            _ if dir.is_none() => dir = Some(PathBuf::from(arg)),
            _ => return usage(),
        }
    }
    let Some(repo) = repo else {
        return usage();
    };
    if !repo.is_dir() {
        eprintln!("priory check: {} is not a folder", repo.display());
        return ExitCode::FAILURE;
    }
    let dir = dir.or_else(|| factory::named_folder(None));
    let factory = match &dir {
        Some(dir) => match load(dir) {
            Some(factory) => Some(factory),
            None => return ExitCode::FAILURE,
        },
        None => None,
    };
    let gate = match &factory {
        None => readiness::Gate::NoFactory,
        Some(factory) => match factory
            .settings
            .as_ref()
            .and_then(|settings| settings.value.gate.as_deref())
        {
            Some(command) => readiness::Gate::Set(command),
            None => readiness::Gate::Unset,
        },
    };
    let required = priory_runtime::protection::required_checks(&repo);
    let report = readiness::assess(&repo, gate, required);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return ExitCode::SUCCESS;
    }
    println!(
        "readiness: {} of {} met, {} missing, {} unknown",
        report.met,
        report.criteria.len(),
        report.missing,
        report.unknown
    );
    for criterion in &report.criteria {
        let status = match criterion.status {
            readiness::Status::Met => "met",
            readiness::Status::Missing => "missing",
            readiness::Status::Unknown => "unknown",
        };
        println!("{status}\t{}\t{}", criterion.id, criterion.detail);
        if criterion.status != readiness::Status::Met {
            println!("\tfix: {}", criterion.fix);
        }
    }
    ExitCode::SUCCESS
}

/// Warns when the Herdr layout `adapters/herdr/install` wrote on this machine was built from other
/// view files: its root id ends in 12 hex digits of the SHA-256 of what `priory views --json`
/// prints, and an id without them predates the hash. Another machine's layout is not checked.
fn warn_of_stale_herdr_layout(factory: &factory::Factory, dir: &Path) {
    let Some(app) = std::env::var_os("HERDR_APP_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| Path::new(&home).join(".config/herdr-app"))
        })
    else {
        return;
    };
    let path = app.join("layouts/priory.json");
    let Some(id) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|layout| layout["id"].as_str().map(str::to_owned))
    else {
        return;
    };
    let hash: String = Sha256::digest(listing::views(factory).to_string().as_bytes())[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let current = id == format!("priory-generated-{hash}");
    let ours = id == "priory-generated"
        || id
            .strip_prefix("priory-generated-")
            .is_some_and(|rest| rest.len() == 12 && rest.bytes().all(|b| b.is_ascii_hexdigit()));
    if ours && !current {
        eprintln!(
            "warning: the Herdr layout {} was built from other view files: run `priory views {} --json | adapters/herdr/install` again, with --copy if you installed copies",
            path.display(),
            dir.display()
        );
    }
}

/// Prints each workflow's irreversible steps with their gates, its paths to one with no human edge,
/// the guarded calls each agent can make and the outcomes the daemon verifies, as four tables or
/// as JSON.
fn print_explanation(factory: &factory::Factory, json: bool) {
    let risk = history(factory);
    if json {
        let mut flows = listing::flows(factory);
        listing::with_risk_estimates(&mut flows, risk.as_ref());
        if let Some(flows) = flows.as_object_mut() {
            flows.remove("workflows");
            let investigators: Vec<_> = explain::investigators(factory)
                .into_iter()
                .map(|investigator| {
                    serde_json::json!({
                        "agent": investigator.agent,
                        "tools": investigator.tools,
                        "servers": investigator.servers.iter().map(|(name, read_only)| {
                            serde_json::json!({ "name": name, "read_only": read_only })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            if !investigators.is_empty() {
                flows.insert("investigators".to_owned(), investigators.into());
            }
        }
        state::emit(&flows.to_string());
        return;
    }
    let gates: Vec<_> = factory
        .workflows
        .iter()
        .flat_map(|(name, workflow)| {
            explain::gates(&workflow.value)
                .into_iter()
                .map(move |gate| (name, gate))
        })
        .collect();
    let paths: Vec<_> = factory
        .workflows
        .iter()
        .flat_map(|(name, workflow)| {
            explain::unattended_paths(&workflow.value)
                .into_iter()
                .map(move |path| (name, path))
        })
        .collect();
    let tools = explain::tools(factory);
    let cell = |text: &str| {
        text.replace('\t', "\\t")
            .replace('\r', "\\r")
            .replace('\n', "\\n")
    };
    let history = |workflow: &str, step: &str| {
        let Some(risk) = &risk else {
            return "-\t-\t-".to_owned();
        };
        let history = risk.step_history(workflow, step);
        let share = |outcome: &str| {
            let (k, n) = (&history[outcome]["k"], &history["trials"]);
            match history["shown"].as_bool() {
                Some(true) => format!(
                    "{k} of {n}, 95% bound {:.1}%",
                    history[outcome]["p95"].as_f64().unwrap_or_default() * 100.0
                ),
                _ => format!("{k} of {n}, needs {}", risk::FLOOR),
            }
        };
        format!(
            "{}\t{}\t{}",
            history["caught"],
            share("judged_wrong"),
            share("landed_wrong")
        )
    };
    state::emit("workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong");
    for (name, gate) in &gates {
        let shown = match &gate.autonomous {
            Some(reason) => format!("autonomous: {reason}"),
            None => gate.approved_by.join(", "),
        };
        state::emit(&format!(
            "{}\t{}\t{}\t{}",
            cell(name),
            cell(&gate.step),
            cell(&shown),
            history(name, &gate.step)
        ));
    }
    state::emit("");
    state::emit("workflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong");
    for (name, path) in &paths {
        let step = path.last().map_or("", String::as_str);
        state::emit(&format!(
            "{}\t{}\t{}",
            cell(name),
            cell(&path.join(" -> ")),
            history(name, step)
        ));
    }
    state::emit("");
    state::emit("agent\tguard\tworkflow\twhy");
    for reach in &tools {
        state::emit(&format!(
            "{}\t{}\t{}\t{}",
            cell(&reach.agent),
            cell(&reach.guard),
            cell(&reach.workflow),
            cell(&reach.why)
        ));
    }
    let investigators = explain::investigators(factory);
    if !investigators.is_empty() {
        state::emit("");
        state::emit("investigator\tallows\tread only");
    }
    for investigator in investigators {
        for tool in &investigator.tools {
            state::emit(&format!(
                "{}\ttool {}\t-",
                cell(&investigator.agent),
                cell(tool)
            ));
        }
        for (server, read_only) in &investigator.servers {
            state::emit(&format!(
                "{}\tserver {}\t{}",
                cell(&investigator.agent),
                cell(server),
                cell(read_only.as_deref().unwrap_or("-"))
            ));
        }
    }
    state::emit("");
    state::emit("workflow\tstep\toutcome\tproof");
    for (name, workflow) in &factory.workflows {
        for (step, outcome, proof) in explain::proofs(&workflow.value) {
            state::emit(&format!(
                "{}\t{}\t{}\t{}",
                cell(name),
                cell(step),
                cell(outcome),
                proof.name()
            ));
        }
    }
    let sandboxes = factory
        .settings
        .as_ref()
        .map(|settings| &settings.value.sandboxes)
        .filter(|sandboxes| !sandboxes.is_empty());
    if sandboxes.is_some() {
        state::emit("");
        state::emit("workflow\tsandbox\tkind\tread\twrite\tenv");
        let listed = |items: &[String]| match items {
            [] => "-".to_owned(),
            items => cell(&items.join(", ")),
        };
        for name in factory
            .workflows
            .keys()
            .filter(|name| !name.starts_with("priory/"))
        {
            match factory.sandbox_of(name) {
                Some((sandbox, value)) => state::emit(&format!(
                    "{}\t{}\tbwrap\t{}\t{}\t{}",
                    cell(name),
                    cell(sandbox),
                    listed(&value.read),
                    listed(&value.write),
                    listed(&value.env)
                )),
                None => state::emit(&format!("{}\t-\t-\t-\t-\t-", cell(name))),
            }
        }
    }
    let debates: Vec<_> = factory
        .workflows
        .iter()
        .flat_map(|(name, workflow)| {
            explain::debates(factory, &workflow.value)
                .into_iter()
                .map(move |(step, line)| (name, step, line))
        })
        .collect();
    if !debates.is_empty() {
        state::emit("");
        state::emit("workflow\tstep\tdebate");
        for (name, step, line) in &debates {
            state::emit(&format!("{}\t{}\t{}", cell(name), cell(step), cell(line)));
        }
    }
}

fn draw(dir: &Path, workflow: &std::ffi::OsStr) -> ExitCode {
    let Some(factory) = load(dir) else {
        return ExitCode::FAILURE;
    };
    let name = workflow.to_string_lossy();
    match factory.workflows.get(name.as_ref()) {
        Some(workflow) => {
            print!("{}", priory_factory::draw::mermaid(&workflow.value));
            ExitCode::SUCCESS
        }
        None => exit("draw", Err(format!("unknown workflow `{name}`"))),
    }
}

/// How long the tool guard waits for the daemon before it denies the call.
const GUARD_WAIT: Duration = Duration::from_secs(5);

/// How long the whole tool guard hook may take, well under Claude's 30-second hook timeout.
const GUARD_DEADLINE: Duration = Duration::from_secs(8);

/// The `PreToolUse` hook's reason to deny the call, if any: none for a call no guard matches, and
/// the daemon's answer for one that matches. Anything but the daemon's yes denies, since a hook
/// that fails lets the call through.
fn guard(input: &Value) -> Option<String> {
    let deny = |reason: &str| Some(reason.to_owned());
    let guards: priory_factory::guard::Guards = match std::env::var("PRIORY_GUARD") {
        Ok(json) if !json.is_empty() => match serde_json::from_str(&json) {
            Ok(guards) => guards,
            Err(error) => {
                return deny(&format!(
                    "Priory's tool guard could not read PRIORY_GUARD ({error}), so it denied this call."
                ));
            }
        },
        _ => Default::default(),
    };
    let tool = input["tool_name"].as_str().unwrap_or_default();
    if let Some(reason) = unnamed_server(tool, std::env::var("PRIORY_MCP_SERVERS").ok().as_deref())
    {
        return deny(&reason);
    }
    let command = shell_command(input);
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let git = |dir: Option<&str>, args: &[&str]| {
        let mut git = Command::new("git");
        if let Some(cwd) = &cwd {
            git.current_dir(cwd);
        }
        if let Some(dir) = dir {
            git.args(["-C", dir]);
        }
        let output = git
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let hits = guards.hits(tool, command.as_deref(), &git);
    let moves_head = command
        .as_deref()
        .is_some_and(priory_factory::guard::moves_head);
    let session = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| input["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    // Each guard the call hits needs its own yes: one allowed command must not carry another.
    for hit in hits {
        let line = serde_json::json!({ "guard": {
            "session": session, "guard": hit.guard, "call": hit.call, "tool": tool,
            "moves_head": moves_head,
        } });
        let reply =
            runtime::call(&runtime::home(), &line.to_string(), GUARD_WAIT).and_then(|reply| {
                serde_json::from_str::<Result<String, String>>(&reply)
                    .map_err(|error| format!("unreadable reply {reply:?}: {error}"))
            });
        match reply {
            Ok(Ok(_)) => {}
            Ok(Err(reason)) => return deny(&reason),
            Err(error) => {
                return deny(&format!(
                    "Priory denied `{}`: it is irreversible (guard `{}`) and the daemon did not confirm an approval within {}s ({error}). Call the ask tool of the priory MCP server if a person must do or approve it.",
                    hit.call,
                    hit.guard,
                    GUARD_WAIT.as_secs()
                ));
            }
        }
    }
    report_connector_use(tool);
    None
}

/// The denial of an MCP tool whose server is none of `servers`, the comma-separated servers a run
/// with claude.ai connectors gets, as its tools name them. Such a run starts without
/// `--strict-mcp-config`, so every connector of the person's account and every plugin's server
/// load too, and no permission rule can name the ones Priory cannot list.
fn unnamed_server(tool: &str, servers: Option<&str>) -> Option<String> {
    let servers = servers.filter(|servers| !servers.is_empty())?;
    let name = tool.strip_prefix("mcp__")?;
    if servers
        .split(',')
        .any(|server| name.starts_with(&format!("{server}__")))
    {
        return None;
    }
    let server = name.rsplit_once("__").map_or(name, |(server, _)| server);
    Some(format!(
        "Priory denied `{tool}`: this run gets only the MCP servers and claude.ai connectors its agent names ({}), and `{server}` is none of them. Do not try it another way: carry on with your other sources.",
        servers.split(',').collect::<Vec<_>>().join(", ")
    ))
}

/// A shell tool's command in a hook's input: Claude's string, or Codex's words quoted for a shell.
fn shell_command(input: &Value) -> Option<String> {
    match &input["tool_input"]["command"] {
        Value::String(command) => Some(command.clone()),
        Value::Array(words) => Some(
            words
                .iter()
                .map(|word| {
                    format!(
                        "'{}'",
                        word.as_str().unwrap_or_default().replace('\'', r"'\''")
                    )
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

/// Prints the `PreToolUse` hook's deny for `reason`.
fn print_deny(reason: &str) {
    let output = serde_json::json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    } });
    println!("{output}");
}

/// The `PreToolUse` hook of a person's own session: it denies a verb only a person runs, since the
/// model decided the call, and asks no daemon. It exits 0 with a deny, as a failed hook lets a call through.
fn outside_hook() -> ExitCode {
    let reason = match serde_json::from_reader::<_, Value>(std::io::stdin()) {
        Err(error) => Some(format!(
            "Priory's person guard could not read this call ({error}), so it denied it."
        )),
        Ok(input) => shell_command(&input)
            .and_then(|command| priory_factory::guard::person(&command).into_iter().next())
            .map(|hit| match hit.guard.as_str() {
                priory_factory::guard::PERSON => format!(
                    "Priory denied `{}`: only a person runs it, and a model decided this call. Tell the person, who can run it themselves: ! {}",
                    hit.call, hit.call
                ),
                _ => format!(
                    "Priory denied {}: the guard cannot read it, so it cannot tell whether it runs a verb only a person runs.",
                    hit.call
                ),
            }),
    };
    if let Some(reason) = reason {
        print_deny(&reason);
    }
    ExitCode::SUCCESS
}

/// `priory mcp --outside`: the MCP server of a session Priory did not start, a person verb that
/// `main` refuses in a run. The daemon's home and state name the factory it reaches.
fn outside_mcp() -> ExitCode {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if !set("PRIORY_HOME") || !set("PRIORY_DB") {
        eprintln!(
            "priory mcp --outside: set PRIORY_HOME and PRIORY_DB to the daemon's, so the server reaches its factory"
        );
        return ExitCode::from(2);
    }
    exit("mcp", mcp::serve_outside(asking_session().as_deref()))
}

/// How long the hook waits for the daemon to say which matching files a run has not got yet.
const SCOPED_WAIT: Duration = Duration::from_secs(1);

/// How long the whole lookup of a file tool's skills and memories may take.
const SCOPED_DEADLINE: Duration = Duration::from_secs(3);

/// The Claude tools that read, edit or create the file their input names.
const FILE_TOOLS: [&str; 5] = ["Read", "Edit", "Write", "MultiEdit", "NotebookEdit"];

/// The text of each skill and memory whose `paths` match the file a file tool's call names, that
/// the run has not got yet. Without a daemon to ask, every match counts as new.
fn scoped_context(input: &Value) -> Option<String> {
    let tool = input["tool_name"].as_str()?;
    if !FILE_TOOLS.contains(&tool) {
        return None;
    }
    let arguments = &input["tool_input"];
    let path = arguments["file_path"]
        .as_str()
        .or_else(|| arguments["notebook_path"].as_str())?;
    let listed = std::env::var_os("PRIORY_PATHS").filter(|file| !file.is_empty())?;
    let root = std::env::var_os("PRIORY_WORKTREE").filter(|root| !root.is_empty())?;
    let cwd = input["cwd"]
        .as_str()
        .map_or_else(|| PathBuf::from(&root), PathBuf::from);
    let relative = relative_path(&cwd.join(path), Path::new(&root))?;
    let scoped: Vec<priory_factory::paths::Scoped> =
        serde_json::from_slice(&std::fs::read(listed).ok()?).ok()?;
    let matched: Vec<(&priory_factory::paths::Scoped, &str)> = scoped
        .iter()
        .filter_map(|entry| {
            let glob = entry
                .globs
                .iter()
                .find(|glob| priory_factory::paths::matches(glob, &relative))?;
            Some((entry, glob.as_str()))
        })
        .collect();
    if matched.is_empty() {
        return None;
    }
    let session = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| input["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let files: Vec<&str> = matched
        .iter()
        .map(|(entry, _)| entry.file.as_str())
        .collect();
    let line = serde_json::json!({ "scoped": { "session": session, "files": files } });
    let new: Option<Vec<String>> = runtime::call(&runtime::home(), &line.to_string(), SCOPED_WAIT)
        .ok()
        .and_then(|reply| serde_json::from_str::<Result<String, String>>(&reply).ok())
        .and_then(Result::ok)
        .map(|new| new.lines().map(str::to_owned).collect());
    let texts: Vec<String> = matched
        .iter()
        .filter(|(entry, _)| new.as_ref().is_none_or(|new| new.contains(&entry.file)))
        .map(|(entry, glob)| {
            format!(
                "Priory adds the {} `{}` ({}) because `{relative}` matches `{glob}`. This run gets it once.\n\n{}",
                entry.kind,
                entry.name,
                entry.file,
                entry.text.trim_end()
            )
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n\n"))
}

/// `path` relative to `root`, with `/` between its parts, once `.` and `..` are resolved; none for
/// a path outside `root`. A link in either, such as macOS's `/tmp`, resolves through the parts
/// that exist.
fn relative_path(path: &Path, root: &Path) -> Option<String> {
    let lexical = |path: &Path| {
        let mut out = PathBuf::new();
        for part in path.components() {
            match part {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                part => out.push(part),
            }
        }
        out
    };
    let resolved = |path: &Path| {
        let path = lexical(path);
        let mut existing = path.as_path();
        let mut rest = Vec::new();
        loop {
            if let Ok(real) = existing.canonicalize() {
                return Some(
                    rest.iter()
                        .rev()
                        .fold(real, |real: PathBuf, part| real.join(part)),
                );
            }
            rest.push(existing.file_name()?.to_owned());
            existing = existing.parent()?;
        }
    };
    let inside = |path: &Path, root: &Path| {
        let relative = path.strip_prefix(root).ok()?;
        let parts: Option<Vec<&str>> = relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect();
        Some(parts?.join("/")).filter(|relative| !relative.is_empty())
    };
    inside(&lexical(path), &lexical(root)).or_else(|| inside(&resolved(path)?, &resolved(root)?))
}

/// Tells the daemon a run used a connector with an open sign-in to-do, so it can clear the item.
/// Best effort after the guards allowed the call: a daemon that does not answer holds it for 1s at most.
fn report_connector_use(tool: &str) {
    let Some(name) = tool.strip_prefix("mcp__") else {
        return;
    };
    let Some((server, short)) = name.rsplit_once("__") else {
        return;
    };
    let watched = std::env::var("PRIORY_CONNECTORS").unwrap_or_default();
    if priory_factory::guard::SIGN_IN_TOOLS.contains(&short)
        || !watched.split(',').any(|name| name == server)
    {
        return;
    }
    let line = serde_json::json!({ "connector_used": server });
    let _ = runtime::call(&runtime::home(), &line.to_string(), Duration::from_secs(1));
}

/// Reports a harness hook to the daemon: `event`, with the session and prompt from the hook's JSON
/// on stdin. A run's pane sets `PRIORY_SESSION`; a stdin session apart from it is the harness's own.
fn hook(event: &str) -> ExitCode {
    if event == "PreToolUse" {
        // A hook that fails or that Claude kills lets the call through, so a guard that panics or
        // hangs anywhere, in git, the connect or the reply, denies before Claude's timeout.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let checked = match serde_json::from_reader::<_, Value>(std::io::stdin()) {
                Ok(input) => (guard(&input), Some(input)),
                Err(error) => (
                    Some(format!(
                        "Priory's tool guard could not read this call ({error}), so it denied it."
                    )),
                    None,
                ),
            };
            sender.send(checked)
        });
        let (denied, input) = receiver.recv_timeout(GUARD_DEADLINE).unwrap_or_else(|_| {
            let reason = format!(
                "Priory's tool guard failed or did not finish within {}s on this call, so it denied it.",
                GUARD_DEADLINE.as_secs()
            );
            (Some(reason), None)
        });
        if let Some(reason) = denied {
            print_deny(&reason);
        } else if let Some(context) = input.and_then(|input| {
            // Best effort: a lookup that fails or hangs adds nothing and never holds the call.
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || sender.send(scoped_context(&input)));
            receiver.recv_timeout(SCOPED_DEADLINE).ok().flatten()
        }) {
            let output = serde_json::json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": context,
            } });
            println!("{output}");
        }
        return ExitCode::SUCCESS;
    }
    let input: serde_json::Value = match serde_json::from_reader(std::io::stdin()) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("priory hook: stdin: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(session) = input["session_id"].as_str() else {
        eprintln!("priory hook: stdin has no session_id");
        return ExitCode::FAILURE;
    };
    let priory = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty());
    let thread = priory
        .as_ref()
        .filter(|id| *id != session)
        .map(|_| session.to_owned());
    let signal = state::Signal {
        event: event.to_owned(),
        session: priory.unwrap_or_else(|| session.to_owned()),
        prompt: input["prompt"].as_str().map(str::to_owned),
        message: input["message"].as_str().map(str::to_owned),
        transcript: input["transcript_path"].as_str().map(str::to_owned),
        thread,
        source: input["source"].as_str().map(str::to_owned),
    };
    let line = serde_json::to_string(&signal).expect("signal serializes");
    match runtime::send(&runtime::home(), &line) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory hook: {error}");
            ExitCode::FAILURE
        }
    }
}

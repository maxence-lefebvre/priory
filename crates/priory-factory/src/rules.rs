use crate::factory::{
    ASK_SHAPE, Agent, Backend, CODEX_PROMPT_LIMIT, Decider, Effect, Factory, Harness, INSTRUCTIONS,
    Intervention, Loaded, OPEN_MEMORIES_PR, OnChange, PermissionMode, Problem, Proof, ScoreKind,
    ScoreOn, Scorer, Session, Step, Trigger, Until, Workflow,
};
use crate::guard::{NEED_APPROVAL, PERSON, SIGN_IN, UNREADABLE};
use crate::lanes::{self, Lanes};
use std::collections::{BTreeMap, BTreeSet};

const ENDS: [&str; 2] = ["done", "stop"];

pub fn check(factory: &Factory) -> Vec<Problem> {
    let mut problems = Vec::new();
    let bands = factory.bands();
    if let Some(settings) = &factory.settings {
        let unique: BTreeSet<_> = bands.iter().collect();
        if bands.is_empty() || unique.len() < bands.len() {
            problems.push(problem(
                settings,
                settings.lines.field("bands"),
                "`bands` needs at least one band, each named once".into(),
            ));
        }
        if let Some(agent) = settings
            .value
            .sweep
            .agent
            .as_ref()
            .filter(|agent| !factory.agents.contains_key(*agent))
        {
            problems.push(problem(
                settings,
                settings.lines.field("sweep"),
                format!("the sweeper agent `{agent}` does not exist"),
            ));
        }
        if let Some(kit) = &settings.value.kit {
            let at = settings.lines.field("kit");
            for message in kit_problems(kit) {
                problems.push(problem(settings, at, message));
            }
        }
        let one_shots = [
            ("handler", "handler", settings.value.handler.as_ref()),
            ("search", "summarizer", settings.value.search.tldr.as_ref()),
        ];
        for (field, role, name) in one_shots {
            let Some(name) = name else {
                continue;
            };
            let refused = match factory.agents.get(name) {
                None => Some(format!("the {role} agent `{name}` does not exist")),
                Some(agent) if !matches!(agent.value.harness, Harness::ClaudeCode) => Some(
                    format!("the {role} agent `{name}` needs harness = \"claude-code\""),
                ),
                Some(_) => None,
            };
            if let Some(message) = refused {
                problems.push(problem(settings, settings.lines.field(field), message));
            }
        }
        // `sh -c ""` exits 0, so a blank gate would pass every proof.
        if settings
            .value
            .gate
            .as_deref()
            .is_some_and(|gate| gate.trim().is_empty())
        {
            problems.push(problem(
                settings,
                settings.lines.field("gate"),
                "`gate` is empty".to_owned(),
            ));
        }
        if let (Some(named), Some(path)) = (&settings.value.asks.deliver, factory.deliver()) {
            use std::os::unix::fs::PermissionsExt;
            let named = named.display();
            let refused = match std::fs::metadata(&path) {
                Ok(file) if file.is_file() && file.permissions().mode() & 0o111 == 0 => {
                    Some(format!("{named} is not executable: chmod +x it"))
                }
                Ok(file) if file.is_file() => None,
                _ => Some(format!("{named} is not a file in the factory folder")),
            };
            if let Some(refused) = refused {
                problems.push(problem(
                    settings,
                    settings.lines.field("asks"),
                    format!("`[asks] deliver`: {refused}"),
                ));
            }
        }
        if !cfg!(feature = "postgres") && settings.value.log.backend == Backend::Postgres {
            problems.push(problem(
                settings,
                settings.lines.field("log"),
                "[log] backend = \"postgres\" needs a priory built with `--features postgres`"
                    .to_owned(),
            ));
        }
        let search = &settings.value.search;
        if search.model.is_some() && search.command.is_some() {
            problems.push(problem(
                settings,
                settings.lines.field("search"),
                "[search] names both a model and a command: keep one".to_owned(),
            ));
        }
        if let Some(Err(message)) = settings
            .value
            .serve
            .as_ref()
            .map(|serve| serve.allows(serve.listen))
        {
            problems.push(problem(settings, settings.lines.field("serve"), message));
        }
        let variable = |name: &str| {
            name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        };
        if factory
            .otlp_headers_env()
            .is_some_and(|name| !variable(name))
        {
            problems.push(problem(
                settings,
                settings.lines.field("telemetry"),
                "`headers_env` names the environment variable that holds the headers, such as \
                 OTEL_EXPORTER_OTLP_HEADERS, never the headers themselves"
                    .to_owned(),
            ));
        }
        if settings
            .value
            .log
            .url_env
            .as_deref()
            .is_some_and(|name| !variable(name))
        {
            problems.push(problem(
                settings,
                settings.lines.field("log"),
                "`[log] url_env` names the environment variable that holds the connection \
                 string, such as PRIORY_LOG_URL, never the string itself"
                    .to_owned(),
            ));
        }
        if settings.value.webhooks.is_some() && settings.value.serve.is_none() {
            problems.push(problem(
                settings,
                settings.lines.field("webhooks"),
                "`[webhooks]` needs a `[serve]` table to listen on".to_owned(),
            ));
        }
        for refused in settings
            .value
            .links
            .iter()
            .filter_map(|link| link.check().err())
        {
            problems.push(problem(settings, settings.lines.field("links"), refused));
        }
        let shortcut = settings
            .value
            .slack
            .as_ref()
            .and_then(|slack| slack.shortcut.as_ref());
        if let Some(shortcut) = shortcut {
            let line = settings.lines.field("slack");
            if shortcut.users.is_empty() {
                let message = "`[slack.shortcut] users` is empty, so nobody could use the shortcut";
                problems.push(problem(settings, line, message.to_owned()));
            }
            let slack_id = |user: &String| {
                user.len() > 1
                    && user.starts_with(['U', 'W'])
                    && user
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            };
            for user in shortcut.users.iter().filter(|user| !slack_id(user)) {
                let message = format!(
                    "`[slack.shortcut] users` `{user}` is not a Slack user id such as U0123ABCD"
                );
                problems.push(problem(settings, line, message));
            }
        }
        if let Some(request) = &settings.value.request {
            let line = settings.lines.field("request");
            if !factory.workflows.contains_key(&request.workflow) {
                let message = format!("`[request] workflow` `{}` does not exist", request.workflow);
                problems.push(problem(settings, line, message));
            }
            if let Some(band) = request.band.as_deref().filter(|band| !bands.contains(band)) {
                let message = format!(
                    "`[request] band` `{band}` is not one of {}",
                    bands.join(", ")
                );
                problems.push(problem(settings, line, message));
            }
        }
        if let Some(batch) = &settings.value.batch {
            let line = settings.lines.field("batch");
            match factory.workflows.get(&batch.workflow) {
                None => {
                    let message = format!("`[batch] workflow` `{}` does not exist", batch.workflow);
                    problems.push(problem(settings, line, message));
                }
                Some(flow) if !has_go(&flow.value) => {
                    let message = format!(
                        "`[batch] workflow` `{}` needs a human step after a model step: the go that starts the batch",
                        batch.workflow
                    );
                    problems.push(problem(settings, line, message));
                }
                Some(_) => {}
            }
            if let Some(entries) = batch
                .entries
                .as_ref()
                .filter(|entries| !factory.workflows.contains_key(*entries))
            {
                let message = format!("`[batch] entries` `{entries}` does not exist");
                problems.push(problem(settings, line, message));
            }
        }
        if !owner_name(settings.value.sweep.repo()) {
            problems.push(problem(
                settings,
                settings.lines.field("sweep"),
                "`[sweep] repo` needs to be `owner/name`".into(),
            ));
        }
        let mut named: Vec<&str> = crate::guard::BUILT_IN.to_vec();
        for command in &settings.value.guard.commands {
            let message = if named.contains(&command.name.as_str()) {
                format!(
                    "`[guard]` command `{}` has the name of another guard",
                    command.name
                )
            } else if command.run.is_empty() {
                format!(
                    "`[guard]` command `{}` needs a program in `run`",
                    command.name
                )
            } else {
                named.push(&command.name);
                continue;
            };
            problems.push(problem(settings, settings.lines.field("guard"), message));
        }
    }
    for trigger in factory.triggers.values() {
        check_band(
            trigger,
            trigger.value.band.as_deref(),
            &bands,
            &mut problems,
        );
        if !factory.workflows.contains_key(&trigger.value.workflow) {
            problems.push(problem(
                trigger,
                trigger.lines.field("workflow"),
                format!("workflow `{}` does not exist", trigger.value.workflow),
            ));
        }
        if let Some(lane) = trigger
            .value
            .lane
            .as_deref()
            .filter(|lane| !lanes::well_formed(lane))
        {
            problems.push(problem(
                trigger,
                trigger.lines.field("lane"),
                not_a_lane(lane),
            ));
        }
        check_lead(factory, trigger, &mut problems);
        match trigger.value.source.as_str() {
            "github" => check_github(trigger, &mut problems),
            "linear" | "linear.view" => check_linear(trigger, &mut problems),
            "slack" => check_slack(trigger, &mut problems),
            "sentry" => check_hooked(
                trigger,
                &SENTRY,
                factory.sentry_webhook().is_some(),
                &mut problems,
            ),
            "honeycomb" => check_hooked(
                trigger,
                &HONEYCOMB,
                factory.honeycomb_webhook().is_some(),
                &mut problems,
            ),
            "cloudwatch" => check_hooked(
                trigger,
                &CLOUDWATCH,
                factory.cloudwatch_webhook().is_some(),
                &mut problems,
            ),
            _ if trigger.value.backfill.is_some() => problems.push(problem(
                trigger,
                trigger.lines.field("backfill"),
                "only a GitHub, Linear or Slack trigger has `backfill`".into(),
            )),
            _ => {}
        }
        let checked = [
            "github",
            "linear",
            "linear.view",
            "sentry",
            "cloudwatch",
            "honeycomb",
            "slack",
            "schedule",
        ];
        if !checked.contains(&trigger.value.source.as_str())
            && trigger
                .value
                .repo
                .as_deref()
                .is_some_and(|repo| !owner_name(repo))
        {
            problems.push(problem(
                trigger,
                trigger.lines.field("repo"),
                "a trigger's `repo` is `owner/name`".into(),
            ));
        }
        if trigger.value.source != "slack" && trigger.value.channel.is_some() {
            problems.push(problem(
                trigger,
                trigger.lines.field("channel"),
                "only a Slack trigger names a `channel`".into(),
            ));
        }
        let reviews = trigger.value.source == "github" && trigger.value.on == REVIEW_REQUESTED;
        for (field, set, what) in [
            ("reviewer", trigger.value.reviewer.is_some(), "a `reviewer`"),
            ("teams", trigger.value.teams.is_some(), "`teams`"),
        ] {
            if !reviews && set {
                problems.push(problem(
                    trigger,
                    trigger.lines.field(field),
                    format!("only a GitHub `{REVIEW_REQUESTED}` trigger names {what}"),
                ));
            }
        }
        if trigger.value.source == "schedule" {
            check_schedule(trigger, &mut problems);
        } else {
            let value = &trigger.value;
            let fields = [
                ("at", value.at.is_some()),
                ("days", value.days.is_some()),
                ("every", value.every.is_some()),
                ("until", value.until.is_some()),
                ("timezone", value.timezone.is_some()),
                ("prompt", value.prompt.is_some()),
            ];
            for (field, _) in fields.into_iter().filter(|(_, set)| *set) {
                problems.push(problem(
                    trigger,
                    trigger.lines.field(field),
                    format!("only a schedule trigger has `{field}`"),
                ));
            }
        }
    }
    for agent in factory.agents.values() {
        check_codex(factory, agent, &mut problems);
        check_shared(agent, &mut problems);
    }
    for workflow in factory.workflows.values() {
        check_band(
            workflow,
            workflow.value.band.as_deref(),
            &bands,
            &mut problems,
        );
        check_workflow(factory, workflow, &mut problems);
    }
    if let Some(lanes) = &factory.lanes {
        check_lanes(lanes, &mut problems);
    }
    for scorer in factory.scorers.values() {
        check_scorer(factory, scorer, &mut problems);
    }
    problems
}

fn check_scorer(factory: &Factory, scorer: &Loaded<Scorer>, problems: &mut Vec<Problem>) {
    let value = &scorer.value;
    let kind = value.kind.name();
    let reads = match value.kind {
        ScoreKind::Outcome => ScoreOn::Result,
        ScoreKind::Command => ScoreOn::Diff,
        ScoreKind::AskShape => ScoreOn::Ask,
    };
    if value.on != reads {
        problems.push(problem(
            scorer,
            scorer.lines.field("on"),
            format!(
                "kind `{kind}` reads on = \"{}\", not \"{}\"",
                reads.name(),
                value.on.name()
            ),
        ));
    }
    let expect = scorer.lines.field("expect");
    match value.kind {
        ScoreKind::Outcome if value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            "an `outcome` scorer needs `expect`: the outcomes that pass".to_owned(),
        )),
        ScoreKind::AskShape if value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            format!(
                "an `ask-shape` scorer needs `expect`: some of {}",
                ASK_SHAPE.join(", ")
            ),
        )),
        ScoreKind::AskShape => {
            for part in value
                .expect
                .iter()
                .filter(|part| !ASK_SHAPE.contains(&part.as_str()))
            {
                problems.push(problem(
                    scorer,
                    expect,
                    format!(
                        "`{part}` is not part of an ask's shape: use {}",
                        ASK_SHAPE.join(", ")
                    ),
                ));
            }
        }
        ScoreKind::Command if !value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            "a `command` scorer takes no `expect`: its exit code is its answer".to_owned(),
        )),
        _ => {}
    }
    if value.kind != ScoreKind::Command {
        for (field, set) in [
            ("run", value.run.is_some()),
            ("timeout", value.timeout.is_some()),
        ] {
            if set {
                problems.push(problem(
                    scorer,
                    scorer.lines.field(field),
                    format!("`{field}` is for kind `command`, not `{kind}`"),
                ));
            }
        }
        return;
    }
    let Some(run) = &value.run else {
        problems.push(problem(
            scorer,
            scorer.lines.field("kind"),
            "a `command` scorer needs `run`: an executable in the factory folder".to_owned(),
        ));
        return;
    };
    use std::os::unix::fs::PermissionsExt;
    let named = run.display();
    let refused = match std::fs::metadata(factory.dir.join(run)) {
        _ if run.is_absolute() || run.components().any(|part| part.as_os_str() == "..") => Some(
            format!("`run` names {named}, which is not in the factory folder"),
        ),
        Ok(file) if file.is_file() && file.permissions().mode() & 0o111 == 0 => Some(format!(
            "`run` names {named}, which is not executable: chmod +x it"
        )),
        Ok(file) if file.is_file() => None,
        _ => Some(format!(
            "`run` names {named}, which is missing from the factory folder"
        )),
    };
    if let Some(refused) = refused {
        problems.push(problem(scorer, scorer.lines.field("run"), refused));
    }
}

fn not_a_lane(lane: &str) -> String {
    format!("lane `{lane}` is not `<team>/<theme>`")
}

fn check_lanes(lanes: &Loaded<Lanes>, problems: &mut Vec<Problem>) {
    if let Some(fallback) = lanes
        .value
        .fallback
        .as_deref()
        .filter(|lane| !lanes::well_formed(lane))
    {
        problems.push(problem(
            lanes,
            lanes.lines.field("fallback"),
            not_a_lane(fallback),
        ));
    }
    for (index, rule) in lanes.value.rules.iter().enumerate() {
        let at = lanes.lines.step(index);
        if rule.is_bare() {
            problems.push(problem(
                lanes,
                at,
                "a rule needs `key`, `team`, `project` or `branch`".into(),
            ));
        }
        if !lanes::well_formed(&rule.lane) {
            problems.push(problem(lanes, at, not_a_lane(&rule.lane)));
        }
    }
}

fn check_band<T>(
    loaded: &Loaded<T>,
    band: Option<&str>,
    bands: &[&str],
    problems: &mut Vec<Problem>,
) {
    if let Some(band) = band.filter(|band| !bands.contains(band)) {
        problems.push(problem(
            loaded,
            loaded.lines.field("band"),
            format!("band `{band}` is not one of {}", bands.join(", ")),
        ));
    }
}

/// Refuses the keys that only a shared session reads.
fn check_shared(agent: &Loaded<Agent>, problems: &mut Vec<Problem>) {
    let value = &agent.value;
    if matches!(value.session, Session::Shared) {
        return;
    }
    for (field, set) in [
        ("idleCompactAt", value.idle_compact_at.is_some()),
        ("onChange", value.on_change != OnChange::Tell),
    ] {
        if set {
            let message = format!(
                "`{field}` needs `session = \"shared\"`: only a shared session is resumed across tasks"
            );
            problems.push(problem(agent, agent.lines.field(field), message));
        }
    }
}

/// Refuses the Claude Code permissions a Codex run cannot apply.
fn check_codex(factory: &Factory, agent: &Loaded<Agent>, problems: &mut Vec<Problem>) {
    let value = &agent.value;
    if !matches!(value.harness, Harness::Codex) {
        return;
    }
    let instructions = factory.codex_instructions();
    let size = instructions.len() + "\n\n".len() + factory.prompt(value).len();
    if !instructions.is_empty() && size > CODEX_PROMPT_LIMIT {
        let mut parts = vec![INSTRUCTIONS];
        if !factory.memories.is_empty() {
            parts.push("the memory index");
        }
        if !factory.skills.is_empty() {
            parts.push("the skill index");
        }
        let parts = format!("{} and its own prompt", parts.join(", "));
        problems.push(problem(
            agent,
            agent.lines.field("harness"),
            format!("a codex agent takes {parts} on its command line, as `developer_instructions`: together they are {size} bytes, over the limit of {CODEX_PROMPT_LIMIT}"),
        ));
    }
    for (field, set) in [
        ("compactAt", value.compact_at.is_some()),
        ("idleCompactAt", value.idle_compact_at.is_some()),
    ] {
        if set {
            let message = format!(
                "a codex agent cannot take `{field}`: Priory reads no Codex tokens, so it never knows the context size"
            );
            problems.push(problem(agent, agent.lines.field(field), message));
        }
    }
    let line = agent.lines.field("permissions");
    let permissions = &value.permissions;
    if matches!(permissions.default_mode, Some(PermissionMode::Plan)) {
        let message = "a codex agent cannot take `permissions.defaultMode = \"plan\"`: use manual, acceptEdits, auto or dontAsk";
        problems.push(problem(agent, line, message.into()));
    }
    for (field, rules) in [("allow", &permissions.allow), ("deny", &permissions.deny)] {
        if !rules.is_empty() {
            problems.push(problem(
                agent,
                line,
                format!("a codex agent cannot take `permissions.{field}`: Codex has no per-tool rules, so its sandbox and approvals follow `defaultMode` alone"),
            ));
        }
    }
}

/// A `hands-off` trigger names a lead, and a lead is a Claude Code agent with a shared session.
fn check_lead(factory: &Factory, trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let Some(lead) = &trigger.value.lead else {
        if trigger.value.intervention == Some(Intervention::HandsOff) {
            problems.push(problem(
                trigger,
                trigger.lines.field("intervention"),
                "`intervention = \"hands-off\"` needs a `lead`: the agent that answers asks in the person's place".into(),
            ));
        }
        return;
    };
    let at = trigger.lines.field("lead");
    let Some(agent) = factory.agents.get(lead) else {
        problems.push(problem(
            trigger,
            at,
            format!("lead `{lead}` does not exist"),
        ));
        return;
    };
    let shared = matches!(agent.value.session, Session::Shared);
    if !shared || !matches!(agent.value.harness, Harness::ClaudeCode) {
        problems.push(problem(
            trigger,
            at,
            format!("lead `{lead}` needs harness = \"claude-code\" and session = \"shared\""),
        ));
    }
}

/// A schedule's run works in `repo`, as a GitHub trigger's does.
fn check_schedule(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    check_repo(trigger, "schedule", problems);
    let Some(at) = value.at else {
        problems.push(problem(
            trigger,
            trigger.lines.field("source"),
            "a schedule trigger needs `at = \"HH:MM\"`".into(),
        ));
        return;
    };
    if value.every.is_some_and(|every| every.as_secs() < 60) {
        problems.push(problem(
            trigger,
            trigger.lines.field("every"),
            "`every` is shorter than the intake's 1-minute poll".into(),
        ));
    }
    if matches!(value.until, Some(Until::At(until)) if until < at) {
        problems.push(problem(
            trigger,
            trigger.lines.field("until"),
            "`until` is before `at`, so it never fires".into(),
        ));
    }
}

const REVIEW_REQUESTED: &str = "pull_request.review_requested";

/// A review request trigger polls one person's requests in every repository, so it names that
/// person and no repository, filter or key.
fn check_review_request(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    let mut report = |field: &str, message: &str| {
        let line = trigger.lines.field(field).or(trigger.lines.field("on"));
        problems.push(problem(trigger, line, message.into()));
    };
    let login = |login: &str| {
        !login.is_empty()
            && !login.starts_with('-')
            && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !value.reviewer.as_deref().is_some_and(login) {
        report(
            "reviewer",
            "a `pull_request.review_requested` trigger needs `reviewer = \"<GitHub login>\"`",
        );
    }
    let slug = |slug: &str| {
        !slug.is_empty()
            && !slug.starts_with('-')
            && slug
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    for team in value.teams.iter().flatten() {
        if !team
            .split_once('/')
            .is_some_and(|(org, name)| login(org) && slug(name))
        {
            report(
                "teams",
                &format!("team `{team}` is not `org/team`, a GitHub organization and team slug"),
            );
        }
    }
    for (field, set) in [
        ("repo", value.repo.is_some()),
        ("filter", value.filter.is_some()),
        ("key", value.key.is_some()),
    ] {
        if set {
            report(
                field,
                &format!(
                    "a `pull_request.review_requested` trigger takes no `{field}`: its tasks work in the pull request's repository, keyed by it and the reviewer"
                ),
            );
        }
    }
}

fn owner_name(repo: &str) -> bool {
    priory_runtime::runtime::check_repo(repo).is_ok()
}

fn check_repo(trigger: &Loaded<Trigger>, source: &str, problems: &mut Vec<Problem>) {
    if !trigger.value.repo.as_deref().is_some_and(owner_name) {
        problems.push(problem(
            trigger,
            trigger
                .lines
                .field("repo")
                .or(trigger.lines.field("source")),
            format!("a {source} trigger needs `repo = \"owner/name\"`"),
        ));
    }
}

fn check_github(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    if value.on == REVIEW_REQUESTED {
        return check_review_request(trigger, problems);
    }
    check_repo(trigger, "GitHub", problems);
    const ON: [&str; 4] = [
        "issues.labeled",
        "issues.opened",
        "pull_request.labeled",
        "pull_request.opened",
    ];
    if !ON.contains(&value.on.as_str()) {
        problems.push(problem(
            trigger,
            trigger.lines.field("on").or(trigger.lines.field("source")),
            format!(
                "a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled`, `pull_request.opened` or `{REVIEW_REQUESTED}`, not `{}`",
                value.on
            ),
        ));
    }
    let label_only = |filter: &BTreeMap<String, String>| {
        value.on.ends_with(".labeled") && filter.keys().all(|key| key == "label")
    };
    if !value.filter.as_ref().is_none_or(label_only) {
        problems.push(problem(
            trigger,
            trigger.lines.field("filter"),
            "`filter` works only with `on = \"*.labeled\"`, on `label`".into(),
        ));
    }
    let number = if value.on.starts_with("pull_request.") {
        "pull_request.number"
    } else {
        "issue.number"
    };
    if value.key.as_deref().is_some_and(|key| key != number) {
        problems.push(problem(
            trigger,
            trigger.lines.field("key"),
            format!("a GitHub trigger's `key` is `{number}`"),
        ));
    }
}

/// A source whose events arrive only by webhook: the one event it takes, its filters and its key.
struct Hooked {
    source: &'static str,
    vendor: &'static str,
    on: &'static str,
    filters: &'static [&'static str],
    key: &'static str,
    events: &'static str,
}

const SENTRY: Hooked = Hooked {
    source: "sentry",
    vendor: "Sentry",
    on: "issue.created",
    filters: &["project", "level"],
    key: "issue.id",
    events: "events",
};

const HONEYCOMB: Hooked = Hooked {
    source: "honeycomb",
    vendor: "Honeycomb",
    on: "trigger.fired",
    filters: &["environment", "trigger"],
    key: "trigger.id",
    events: "alerts",
};

const CLOUDWATCH: Hooked = Hooked {
    source: "cloudwatch",
    vendor: "CloudWatch",
    on: "alarm.state_changed",
    filters: &["name", "state", "previous_state"],
    key: "alarm.arn",
    events: "alarm state changes",
};

/// A trigger of a webhook-only source takes its one event, its filters and its key, and needs its
/// `[webhooks.<source>]` table.
fn check_hooked(
    trigger: &Loaded<Trigger>,
    hooked: &Hooked,
    webhook: bool,
    problems: &mut Vec<Problem>,
) {
    let value = &trigger.value;
    let Hooked {
        source, vendor, on, ..
    } = hooked;
    let mut report = |field: &str, message: String| {
        let line = trigger.lines.field(field).or(trigger.lines.field("source"));
        problems.push(problem(trigger, line, message));
    };
    if value.backfill.is_some() {
        report(
            "backfill",
            "only a GitHub, Linear or Slack trigger has `backfill`".into(),
        );
    }
    if !webhook {
        report(
            "source",
            format!(
                "a `{source}` trigger needs `[webhooks.{source}]` in factory.toml: {vendor} {} arrive only by webhook",
                hooked.events
            ),
        );
    }
    if value.on != *on {
        report(
            "on",
            format!("a `{source}` trigger handles `{on}`, not `{}`", value.on),
        );
    }
    if value.filter.as_ref().is_some_and(|filter| {
        filter
            .keys()
            .any(|key| !hooked.filters.contains(&key.as_str()))
    }) {
        let filters: Vec<String> = hooked.filters.iter().map(|f| format!("`{f}`")).collect();
        let filters = match filters.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, others)) => format!("{} or {last}", others.join(", ")),
            None => String::new(),
        };
        report(
            "filter",
            format!("a `{source}` trigger filters on {filters}"),
        );
    }
    if value.repo.as_deref().is_some_and(|repo| !owner_name(repo)) {
        report(
            "repo",
            format!("a {vendor} trigger's `repo` is `owner/name`"),
        );
    }
    if value.key.as_deref().is_some_and(|key| key != hooked.key) {
        report(
            "key",
            format!("a {vendor} trigger's `key` is `{}`", hooked.key),
        );
    }
}

fn check_linear(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    let view = value.source == "linear.view";
    let (on, filters): (&[&str], &[&str]) = if view {
        (&["issue.entered", "issue.left"], &[])
    } else {
        (
            &["issue.created", "issue.state_changed", "comment.created"],
            &["team", "project", "label", "state"],
        )
    };
    let mut report = |field: &str, message: String| {
        let line = trigger.lines.field(field).or(trigger.lines.field("source"));
        problems.push(problem(trigger, line, message));
    };
    if view != value.view.is_some() {
        report(
            "view",
            "a `linear.view` trigger, and only it, names a `view`".into(),
        );
    }
    if !on.contains(&value.on.as_str()) {
        let on = on
            .iter()
            .map(|on| format!("`{on}`"))
            .collect::<Vec<_>>()
            .join(", ");
        report(
            "on",
            format!(
                "a `{}` trigger handles {on}, not `{}`",
                value.source, value.on
            ),
        );
    }
    let unknown = |filter: &BTreeMap<String, String>| {
        filter.keys().any(|key| !filters.contains(&key.as_str()))
    };
    if value.filter.as_ref().is_some_and(unknown) {
        let message = if view {
            "a `linear.view` trigger takes no `filter`: its view filters".to_string()
        } else {
            "a `linear` trigger filters on `team`, `project`, `label` or `state`".to_string()
        };
        report("filter", message);
    }
    if value.repo.as_deref().is_some_and(|repo| !owner_name(repo)) {
        report("repo", "a Linear trigger's `repo` is `owner/name`".into());
    }
    if value
        .key
        .as_deref()
        .is_some_and(|key| key != "issue.identifier")
    {
        report(
            "key",
            "a Linear trigger's `key` is `issue.identifier`".into(),
        );
    }
}

fn check_slack(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    let mut report = |field: &str, message: String| {
        let line = trigger.lines.field(field).or(trigger.lines.field("source"));
        problems.push(problem(trigger, line, message));
    };
    if value.on != "message.posted" {
        report(
            "on",
            format!(
                "a Slack trigger handles `message.posted`, not `{}`",
                value.on
            ),
        );
    }
    if value.channel.as_deref().is_none_or(str::is_empty) {
        report(
            "channel",
            "a Slack trigger needs `channel = \"<channel id>\"`".into(),
        );
    }
    if value
        .filter
        .as_ref()
        .is_some_and(|filter| filter.keys().any(|key| key != "text"))
    {
        report("filter", "a Slack trigger filters on `text` only".into());
    }
    if value.view.is_some() {
        report("source", "a Slack trigger names no `view`".into());
    }
    if value.repo.as_deref().is_some_and(|repo| !owner_name(repo)) {
        report("repo", "a Slack trigger's `repo` is `owner/name`".into());
    }
    if value.key.as_deref().is_some_and(|key| key != "message.ts") {
        report("key", "a Slack trigger's `key` is `message.ts`".into());
    }
}

fn check_workflow(factory: &Factory, workflow: &Loaded<Workflow>, problems: &mut Vec<Problem>) {
    let mut steps: BTreeMap<&str, &Step> = BTreeMap::new();
    for (index, step) in workflow.value.steps.iter().enumerate() {
        let line = workflow.lines.step(index);
        if ENDS.contains(&step.id.as_str()) {
            problems.push(problem(
                workflow,
                line,
                format!(
                    "step id `{}` is reserved for the end of a workflow",
                    step.id
                ),
            ));
            return;
        }
        if steps.insert(step.id.as_str(), step).is_some() {
            problems.push(problem(
                workflow,
                line,
                format!("step `{}` is defined twice", step.id),
            ));
            return;
        }
    }
    let guards = factory.guards().names();
    for allowed in &workflow.value.guard.allow {
        let why = if !guards.contains(allowed) {
            UNKNOWN_GUARD
        } else if allowed == PERSON {
            PERSON_ONLY
        } else if allowed == UNREADABLE {
            UNREAD
        } else if allowed == SIGN_IN {
            SIGN_IN_ONLY
        } else if NEED_APPROVAL.contains(&allowed.as_str()) {
            "which only a person's approval grants, so only an irreversible step may name it"
        } else {
            continue;
        };
        problems.push(problem(
            workflow,
            workflow.lines.field("guard"),
            format!("`guard.allow` names `{allowed}`, {why}"),
        ));
    }
    let start = workflow.value.start.as_str();
    if !steps.contains_key(start) {
        problems.push(problem(
            workflow,
            workflow.lines.field("start"),
            format!("start `{start}` is not a step"),
        ));
        return;
    }
    if steps[start].after.is_some() {
        problems.push(problem(
            workflow,
            workflow.lines.field("start"),
            format!(
                "start `{start}` waits for a merge, and a task starts before it owns a pull request"
            ),
        ));
    }
    if let Some(reopen) = &workflow.value.reopen
        && steps
            .get(reopen.as_str())
            .is_none_or(|step| step.decides != Decider::Model)
    {
        problems.push(problem(
            workflow,
            workflow.lines.field("reopen"),
            format!("reopen `{reopen}` is not a model step"),
        ));
    }
    let reached = reachable(&steps, start, |_| false);
    for (index, step) in workflow.value.steps.iter().enumerate() {
        let mut report = |message: String| {
            problems.push(problem(
                workflow,
                workflow.lines.step(index),
                format!("step `{}`: {message}", step.id),
            ))
        };
        let fields = [
            (step.agent.is_some(), Decider::Model, "an `agent`"),
            (step.prompt.is_some(), Decider::Model, "a `prompt`"),
            (
                !step.plan_requires.is_empty(),
                Decider::Model,
                "`plan_requires`",
            ),
            (!step.options.is_empty(), Decider::Human, "`options`"),
            (!step.approves.is_empty(), Decider::Human, "`approves`"),
            (step.command.is_some(), Decider::Human, "a `command`"),
            (step.action.is_some(), Decider::Code, "an `action`"),
            (step.run.is_some(), Decider::Code, "a `run` command"),
        ];
        for (_, owner, field) in fields
            .iter()
            .filter(|(set, owner, _)| *set && *owner != step.decides)
        {
            report(format!("only a {} step has {field}", owner.name()));
        }
        match step.decides {
            Decider::Model => {
                match &step.agent {
                    Some(agent) if !factory.agents.contains_key(agent) => {
                        report(format!("agent `{agent}` does not exist"))
                    }
                    Some(_) => {}
                    None => report("a model step needs an `agent`".into()),
                }
                if step.prompt.is_none() {
                    report("a model step needs a `prompt`".into());
                }
                let asked = format!(
                    "{} {}",
                    step.prompt.as_deref().unwrap_or_default(),
                    step.agent
                        .as_ref()
                        .and_then(|agent| factory.agents.get(agent))
                        .map_or("", |agent| agent.value.body.as_str())
                )
                .to_lowercase();
                for field in step
                    .plan_requires
                    .iter()
                    .filter(|field| !asked.contains(field.name()))
                {
                    report(format!(
                        "`plan_requires` names `{}`, which neither the step's prompt nor its agent's instructions mention",
                        field.name()
                    ));
                }
            }
            Decider::Human => {
                for option in step
                    .options
                    .iter()
                    .filter(|option| !step.next.contains_key(*option))
                {
                    report(format!("option `{option}` has no `next`"));
                }
                for outcome in step
                    .next
                    .keys()
                    .filter(|outcome| !step.options.contains(outcome))
                {
                    report(format!(
                        "`next` has outcome `{outcome}`, which is not an option"
                    ));
                }
                if let Some(command) = &step.command {
                    if command.argv.first().is_none_or(String::is_empty) {
                        report("command argv must start with a program".into());
                    }
                    if let Some(target) = command.target.as_deref().filter(|target| {
                        *target != "worktree" && !factory.targets().contains(target)
                    }) {
                        report(format!(
                            "command target `{target}` is not worktree nor a name in [asks.targets]"
                        ));
                    }
                    for run in command
                        .runs
                        .iter()
                        .filter(|run| !step.options.contains(run))
                    {
                        report(format!("command runs `{run}`, which is not an option"));
                    }
                }
            }
            Decider::Code if step.action.is_none() => {
                report("a code step needs an `action`".into())
            }
            Decider::Code => match (step.action.as_deref() == Some("run"), &step.run) {
                (true, None) => report("a `run` action needs a `run` command".into()),
                (false, Some(_)) => report("only a `run` action has a `run` command".into()),
                _ if step.action.as_deref() == Some(OPEN_MEMORIES_PR)
                    && factory
                        .settings
                        .as_ref()
                        .is_none_or(|settings| settings.value.kit.is_none()) =>
                {
                    report(format!(
                        "an `{OPEN_MEMORIES_PR}` action writes to the kit repository, so factory.toml needs a `[kit]`"
                    ))
                }
                _ => {}
            },
        }
        if step.next.is_empty() {
            report("`next` is empty".into());
        }
        for (outcome, target) in &step.next {
            if !steps.contains_key(target.as_str()) && !ENDS.contains(&target.as_str()) {
                report(format!(
                    "outcome `{outcome}` leads to `{target}`, which is not a step, `done` or `stop`"
                ));
            }
        }
        for outcome in step
            .by
            .keys()
            .filter(|outcome| !step.next.contains_key(*outcome))
        {
            report(format!(
                "`by` has outcome `{outcome}`, which is not in `next`"
            ));
        }
        if !step.proof.is_empty() && step.decides != Decider::Model {
            report("only a model step has a `proof`".into());
        }
        for outcome in step
            .proof
            .keys()
            .filter(|outcome| !step.next.contains_key(*outcome))
        {
            report(format!(
                "`proof` has outcome `{outcome}`, which is not in `next`"
            ));
        }
        if step.proof.values().any(|proof| *proof == Proof::GatePassed) && factory.gate().is_none()
        {
            report("`proof` names `gate-passed`, and factory.toml sets no `gate`".into());
        }
        if !reached.contains(step.id.as_str()) {
            report("no path from start reaches it".into());
        }
        for approved in &step.approves {
            if steps
                .get(approved.as_str())
                .is_none_or(|target| target.effect != Effect::Irreversible)
            {
                report(format!(
                    "approves `{approved}`, which is not an irreversible step"
                ));
            }
        }
        let autonomous = step
            .autonomous
            .as_deref()
            .filter(|reason| !reason.trim().is_empty());
        if step.autonomous.is_some() && autonomous.is_none() {
            report("`autonomous` needs a reason".into());
        }
        let grants = step.effect == Effect::Irreversible && step.decides == Decider::Model;
        if !grants && !step.guard.allow.is_empty() {
            report(
                "`guard.allow` grants nothing on a step that is not an irreversible model step"
                    .into(),
            );
        }
        for allowed in &step.guard.allow {
            if !guards.contains(allowed) {
                report(format!("`guard.allow` names `{allowed}`, {UNKNOWN_GUARD}"));
            } else if allowed == PERSON {
                report(format!("`guard.allow` names `{allowed}`, {PERSON_ONLY}"));
            } else if allowed == UNREADABLE {
                report(format!("`guard.allow` names `{allowed}`, {UNREAD}"));
            } else if allowed == SIGN_IN {
                report(format!("`guard.allow` names `{allowed}`, {SIGN_IN_ONLY}"));
            } else if step.autonomous.is_some() && NEED_APPROVAL.contains(&allowed.as_str()) {
                report(format!(
                    "`guard.allow` names `{allowed}`, which only a person's approval grants, and the step is `autonomous`"
                ));
            }
        }
        if step.effect == Effect::Irreversible && autonomous.is_none() {
            let unapproved = reachable(&steps, start, |other| {
                other.decides == Decider::Human && other.approves.contains(&step.id)
            });
            if unapproved.contains(step.id.as_str()) {
                report("irreversible, and a path from start reaches it with no human step that approves it".into());
            }
        }
    }
}

const UNKNOWN_GUARD: &str = "which is no built-in guard nor one of `[guard]` in factory.toml";
const PERSON_ONLY: &str = "which only a person runs, so nothing may grant it";
const SIGN_IN_ONLY: &str = "which only an agent's `connectorSignIn` grants";
const UNREAD: &str = "which the guard cannot read, so nothing may grant it";

/// Steps reachable from `start`, never walking past a step for which `stops` holds.
pub(crate) fn reachable<'a>(
    steps: &BTreeMap<&'a str, &'a Step>,
    start: &'a str,
    stops: impl Fn(&Step) -> bool,
) -> BTreeSet<&'a str> {
    let mut reached = BTreeSet::from([start]);
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        let Some(step) = steps.get(id).filter(|step| !stops(step)) else {
            continue;
        };
        for target in step.next.values() {
            if steps.contains_key(target.as_str()) && reached.insert(target.as_str()) {
                pending.push(target.as_str());
            }
        }
    }
    reached
}

fn problem<T>(loaded: &Loaded<T>, line: Option<usize>, message: String) -> Problem {
    Problem {
        file: loaded.file.clone(),
        at: line.map(|line| line.to_string()),
        message,
    }
}

/// Whether a model step of `workflow` leads straight to a human step, the go a batch's plan needs.
fn has_go(workflow: &Workflow) -> bool {
    workflow
        .steps
        .iter()
        .filter(|step| step.decides == Decider::Model)
        .flat_map(|step| step.next.values())
        .any(|next| {
            workflow
                .step(next)
                .is_some_and(|step| step.decides == Decider::Human)
        })
}

/// What `priory check` refuses in `[kit]`: an empty repository or ref, one git would read as an
/// option, and a path that leaves the repository.
fn kit_problems(kit: &crate::factory::Kit) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, value) in [
        ("repo", Some(kit.repo.as_str())),
        ("ref", kit.reference.as_deref()),
    ] {
        match value.map(str::trim) {
            Some("") => problems.push(format!("`[kit]` `{key}` is empty")),
            Some(value) if value.starts_with('-') => problems.push(format!(
                "`[kit]` `{key}` is `{value}`: it cannot start with `-`"
            )),
            _ => {}
        }
    }
    for (key, path) in kit.paths() {
        let inside = !path.is_empty()
            && std::path::Path::new(path)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)));
        if !inside {
            problems.push(format!(
                "`[kit]` `{key}` is `{path}`: a kit path is relative and stays inside the repository"
            ));
        }
    }
    problems
}

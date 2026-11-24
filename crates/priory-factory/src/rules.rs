use crate::factory::{
    ASK_SHAPE, Agent, Backend, Benchmark, CODEX_PROMPT_LIMIT, Confidence, Debate, Decider, Effect,
    Factory, HELD_SIGNALS, Harness, INSTRUCTIONS, Intervention, Loaded, Moment, OPEN_MEMORIES_PR,
    OnChange, PermissionMode, Problem, Proof, Recipe, ScoreKind, ScoreOn, Scorer, Session, Step,
    Trigger, Until, Workflow, join_instructions,
};
use crate::guard::{NEED_APPROVAL, PERSON, SIGN_IN, UNREADABLE};
use crate::labels;
use crate::lanes::{self, Lanes};
use priory_runtime::servers;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const ENDS: [&str; 2] = ["done", "stop"];

/// The factory in `dir` once it loads and passes every rule, or its problems.
pub fn validated(dir: &Path) -> Result<Factory, Vec<Problem>> {
    let (factory, mut problems) = crate::factory::load(dir);
    // Rules on a partly loaded factory would report references to the files that failed to parse.
    if problems.is_empty() {
        problems = check(&factory);
    }
    if problems.is_empty() {
        Ok(factory)
    } else {
        Err(problems)
    }
}

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
        for band in factory.preempt() {
            let message = if !bands.contains(&band.as_str()) {
                format!(
                    "`preempt` names band `{band}`, which is not one of {}",
                    bands.join(", ")
                )
            } else if bands.last() == Some(&band.as_str()) {
                format!("`preempt` names band `{band}`, the last band, which has no band below it")
            } else {
                continue;
            };
            problems.push(problem(settings, settings.lines.field("queue"), message));
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
        let secrets = factory.secret_envs();
        for (name, sandbox) in &settings.value.sandboxes {
            for message in sandbox_problems(name, sandbox, &secrets) {
                problems.push(problem(
                    settings,
                    settings.lines.field("sandboxes"),
                    message,
                ));
            }
        }
        if let Some(memories) = &settings.value.memories {
            let at = settings.lines.field("memories");
            for message in memories_problems(memories) {
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
        if let Some(critic) = &settings.value.critic {
            let refused = match factory.scorers.get(critic) {
                None => Some(format!("the critic `{critic}` is no scorer in scorers/")),
                Some(scorer) if scorer.value.on != ScoreOn::Result => Some(format!(
                    "the critic `{critic}` reads on = \"{}\": a critic reads a plan, which names no commit, so use one on result",
                    scorer.value.on.name()
                )),
                Some(_) => None,
            };
            if let Some(message) = refused {
                problems.push(problem(settings, settings.lines.field("critic"), message));
            }
        }
        if let Some(name) = &settings.value.pre_review {
            let refused = match factory.scorers.get(name) {
                None => Some(format!("the pre-review `{name}` is no scorer in scorers/")),
                Some(scorer) if scorer.value.kind != ScoreKind::Judge => Some(format!(
                    "the pre-review `{name}` is of kind {}: a pre-review is a judge, which reads the lessons",
                    scorer.value.kind.name()
                )),
                Some(_) => None,
            };
            if let Some(message) = refused {
                problems.push(problem(
                    settings,
                    settings.lines.field("pre_review"),
                    message,
                ));
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
        let commands = [
            ("asks", "deliver", &settings.value.asks.deliver),
            ("runs", "attach", &settings.value.runs.attach),
        ];
        for (table, key, named) in commands {
            if let Some(named) = named
                && let Some(refused) = not_executable(&factory.dir, named)
            {
                problems.push(problem(
                    settings,
                    settings.lines.field(table),
                    format!("`[{table}] {key}`: {refused}"),
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
        if let Some(telemetry) = factory.telemetry() {
            let line = settings.lines.field("telemetry");
            let mut refuse = |message: String| problems.push(problem(settings, line, message));
            for group in &telemetry.metrics {
                if !crate::factory::METRIC_GROUPS.contains(&group.as_str()) {
                    refuse(format!(
                        "`metrics` names `{group}`, which is not a metrics group: the groups are {}",
                        crate::factory::METRIC_GROUPS.join(", ")
                    ));
                }
            }
            let least = crate::factory::METRICS_EVERY_LEAST;
            if let Some(every) = telemetry.metrics_every.filter(|every| *every < least) {
                refuse(format!(
                    "`metrics_every` is {}s: it needs to be {}s or more",
                    every.as_secs(),
                    least.as_secs()
                ));
            }
            if telemetry.metrics_cap == Some(0) {
                refuse("`metrics_cap` is 0: it needs to be 1 or more".to_owned());
            }
        }
        if let Some(upgrade) = &settings.value.upgrade {
            let line = settings.lines.field("upgrade");
            let mut refuse = |message: &str| {
                problems.push(problem(settings, line, message.to_owned()));
            };
            if settings.value.log.backend == crate::factory::Backend::Postgres {
                refuse(
                    "`[upgrade]` needs the herdr-bus log: Priory cannot tell whether a release \
                     changes a Postgres log's tables, so upgrade these machines by hand",
                );
            }
            if upgrade.units[0] == upgrade.units[1] {
                refuse(
                    "`[upgrade] units` names one unit twice: give the daemon's, then the intake's",
                );
            }
            if upgrade.every.is_some_and(|every| every.as_secs() < 600) {
                refuse("`[upgrade] every` is under 10 minutes: GitHub limits anonymous requests");
            }
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
        if let Some(decision) = &settings.value.decisions.triage {
            let line = settings.lines.field("decisions");
            let mut refuse = |message: String| problems.push(problem(settings, line, message));
            if !variable(&decision.key_env) {
                refuse(
                    "`[decisions.triage] key_env` names the environment variable that holds the \
                     model's key, such as TYPESAFE_API_KEY, never the key itself"
                        .to_owned(),
                );
            }
            for (key, share) in [
                ("agreement", decision.agreement),
                ("threshold", decision.threshold),
            ] {
                if let Some(share) = share.filter(|share| !(0.5..=1.0).contains(share)) {
                    refuse(format!(
                        "`[decisions.triage] {key}` is {share}: give a share from 0.5 to 1, such as 0.9"
                    ));
                }
            }
            if decision.timeout().as_secs() > 30 {
                refuse(format!(
                    "`[decisions.triage] timeout` is {}s: give at most 30s",
                    decision.timeout().as_secs()
                ));
            }
            if decision.daily_calls == Some(0) {
                refuse("`[decisions.triage] daily_calls` is 0: give at least 1".to_owned());
            }
            let cases = factory.dir.join(&decision.evaluation);
            if let Err(why) = labels::load_triage(&cases) {
                refuse(format!(
                    "`[decisions.triage] evaluation` {}: {why}",
                    decision.evaluation.display()
                ));
            }
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
        if let Some(caps) = &settings.value.caps {
            let line = settings.lines.field("caps");
            for lane in caps
                .lanes
                .keys()
                .filter(|lane| !crate::lanes::well_formed(lane))
            {
                let message = format!("`[caps.lanes]` `{lane}` is not a lane such as `team/theme`");
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
        let at = settings.lines.field("repos");
        for (repo, recipes) in &settings.value.repos {
            if !owner_name(repo) {
                let message = format!("`[repos]` key `{repo}` needs to be `owner/name`");
                problems.push(problem(settings, at, message));
                continue;
            }
            for moment in Moment::ALL {
                if let Some(recipe) = recipes.recipe(moment) {
                    let name = format!("`[repos.\"{repo}\"] {}`", moment.name());
                    if let Some(message) = recipe_problem(&settings.value.guard, &name, recipe) {
                        problems.push(problem(settings, at, message));
                    }
                }
            }
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
    check_caps(factory, &mut problems);
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
        check_auto_answer(trigger, &mut problems);
        check_pre(factory, trigger, &mut problems);
        check_intent(trigger, &mut problems);
        // A runner's claim of a key cannot see the pull requests another runner's tasks own.
        if trigger.value.source == "github"
            && FEEDBACK.contains(&trigger.value.on.as_str())
            && factory.runners.len() > 1
        {
            problems.push(problem(
                trigger,
                trigger.lines.field("on"),
                format!(
                    "a `{}` trigger needs one runner; this factory has {}",
                    trigger.value.on,
                    factory.runners.len()
                ),
            ));
        }
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
            "figma" => check_hooked(
                trigger,
                &FIGMA,
                factory.figma_webhook().is_some(),
                &mut problems,
            ),
            "cloudwatch" => check_hooked(
                trigger,
                &CLOUDWATCH,
                factory.cloudwatch_webhook().is_some(),
                &mut problems,
            ),
            "intercom" => check_hooked(
                trigger,
                &INTERCOM,
                factory.intercom_webhook().is_some(),
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
            "figma",
            "intercom",
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
    check_consult(factory, &mut problems);
    for (name, agent) in &factory.agents {
        check_investigator(factory, name, agent, &mut problems);
    }
    for agent in factory.agents.values() {
        check_codex(factory, agent, &mut problems);
        check_shared(agent, &mut problems);
        check_skills(factory, agent, &mut problems);
        check_connectors(agent, &mut problems);
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
    for benchmark in factory.benchmarks.values() {
        check_benchmark(factory, benchmark, &mut problems);
    }
    for memory in factory.memories.values() {
        if let Some(step) = &memory.value.step
            && !approval_step(factory, step)
        {
            problems.push(problem(
                memory,
                memory.lines.field("step"),
                format!(
                    "`step` is `{step}`: a lesson names `<workflow>/<step>`, a human step with `approves`"
                ),
            ));
        }
    }
    problems
}

/// Whether `scope`, as `<workflow>/<step>`, names a human step with `approves`, which a lesson
/// is for.
pub fn approval_step(factory: &Factory, scope: &str) -> bool {
    scope
        .split_once('/')
        .and_then(|(workflow, step)| factory.workflows.get(workflow)?.value.step(step))
        .is_some_and(|step| step.decides == Decider::Human && !step.approves.is_empty())
}

fn check_benchmark(factory: &Factory, benchmark: &Loaded<Benchmark>, problems: &mut Vec<Problem>) {
    let value = &benchmark.value;
    if !factory.workflows.contains_key(&value.workflow) {
        problems.push(problem(
            benchmark,
            benchmark.lines.field("workflow"),
            format!("no workflow `{}` in workflows/", value.workflow),
        ));
    }
    match factory.scorers.get(&value.primary) {
        None => problems.push(problem(
            benchmark,
            benchmark.lines.field("primary"),
            format!("no scorer `{}` in scorers/", value.primary),
        )),
        Some(scorer) if scorer.value.on == ScoreOn::Ask => problems.push(problem(
            benchmark,
            benchmark.lines.field("primary"),
            format!(
                "the primary scorer `{}` reads asks, which not every replay makes: use one on result or diff",
                value.primary
            ),
        )),
        Some(_) => {}
    }
    // A replay ends before its first irreversible step, so it never merges.
    let named = std::iter::once(("primary", &value.primary))
        .chain(value.scorers.iter().map(|name| ("scorers", name)));
    for (field, name) in named {
        if factory
            .scorers
            .get(name)
            .is_some_and(|scorer| scorer.value.on == ScoreOn::Merge)
        {
            problems.push(problem(
                benchmark,
                benchmark.lines.field(field),
                format!("the scorer `{name}` reads what followed a merge, and a replay never merges: use one on result, diff or ask"),
            ));
        }
    }
    for name in value
        .scorers
        .iter()
        .filter(|name| !factory.scorers.contains_key(*name))
    {
        problems.push(problem(
            benchmark,
            benchmark.lines.field("scorers"),
            format!("no scorer `{name}` in scorers/"),
        ));
    }
    if value.cases.is_empty() {
        problems.push(problem(
            benchmark,
            benchmark.lines.field("cases"),
            "a benchmark needs `cases`: folders that priory bench capture wrote".to_owned(),
        ));
    }
    for case in &value.cases {
        if case.is_absolute() || case.components().any(|part| part.as_os_str() == "..") {
            problems.push(problem(
                benchmark,
                benchmark.lines.field("cases"),
                format!(
                    "the case {} is not inside the benchmarks folder",
                    case.display()
                ),
            ));
        }
    }
}

fn check_scorer(factory: &Factory, scorer: &Loaded<Scorer>, problems: &mut Vec<Problem>) {
    let value = &scorer.value;
    let kind = value.kind.name();
    let reads = match value.kind {
        ScoreKind::Outcome => ScoreOn::Result,
        ScoreKind::Command => ScoreOn::Diff,
        ScoreKind::AskShape => ScoreOn::Ask,
        ScoreKind::Judge => ScoreOn::Diff,
        ScoreKind::Held => ScoreOn::Merge,
        ScoreKind::Intent => ScoreOn::Result,
    };
    // A judge reads the result alone, as a plan's critic does, or the result and its diff.
    let judged = value.kind == ScoreKind::Judge && value.on == ScoreOn::Result;
    if value.on != reads && !judged {
        problems.push(problem(
            scorer,
            scorer.lines.field("on"),
            match value.kind {
                ScoreKind::Judge => format!(
                    "kind `judge` reads on = \"result\" or \"diff\", not \"{}\"",
                    value.on.name()
                ),
                _ => format!(
                    "kind `{kind}` reads on = \"{}\", not \"{}\"",
                    reads.name(),
                    value.on.name()
                ),
            },
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
        ScoreKind::Judge if !value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            "a `judge` scorer takes no `expect`: its answer is pass or fail".to_owned(),
        )),
        ScoreKind::Held if !value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            "a `held` scorer takes no `expect`: name what fails it in `fails`".to_owned(),
        )),
        ScoreKind::Intent if !value.expect.is_empty() => problems.push(problem(
            scorer,
            expect,
            "an `intent` scorer takes no `expect`: its questions come from the trigger".to_owned(),
        )),
        _ => {}
    }
    check_held(scorer, problems);
    if value.kind == ScoreKind::Judge {
        check_judge(factory, scorer, problems);
    } else {
        for (field, set) in [
            ("model", value.model.is_some()),
            ("prompt", value.prompt.is_some()),
            ("evaluation", value.evaluation.is_some()),
            ("agreement", value.agreement.is_some()),
        ] {
            if set {
                problems.push(problem(
                    scorer,
                    scorer.lines.field(field),
                    format!("`{field}` is for kind `judge`, not `{kind}`"),
                ));
            }
        }
    }
    match (&value.key_env, value.kind) {
        (Some(_), kind) if kind != ScoreKind::Intent => problems.push(problem(
            scorer,
            scorer.lines.field("key_env"),
            format!("`key_env` is for kind `intent`, not `{kind}`", kind = kind.name()),
        )),
        (Some(name), _) if !variable(name) => problems.push(problem(
            scorer,
            scorer.lines.field("key_env"),
            "`key_env` names the environment variable that holds the model's key, such as TYPESAFE_API_KEY, never the key itself".to_owned(),
        )),
        _ => {}
    }
    if !matches!(value.kind, ScoreKind::Command | ScoreKind::Intent) {
        for (field, set) in [
            ("run", value.run.is_some()),
            ("timeout", value.timeout.is_some()),
        ] {
            if set {
                problems.push(problem(
                    scorer,
                    scorer.lines.field(field),
                    format!("`{field}` is for kind `command` or `intent`, not `{kind}`"),
                ));
            }
        }
        return;
    }
    let Some(run) = &value.run else {
        let message = match value.kind {
            ScoreKind::Intent => {
                "an `intent` scorer needs `run`: a decision model in the factory folder"
            }
            _ => "a `command` scorer needs `run`: an executable in the factory folder",
        };
        problems.push(problem(
            scorer,
            scorer.lines.field("kind"),
            message.to_owned(),
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

/// A `held` scorer needs `after` and `fails`, each signal one it can read; no other kind takes them.
fn check_held(scorer: &Loaded<Scorer>, problems: &mut Vec<Problem>) {
    let value = &scorer.value;
    if value.kind != ScoreKind::Held {
        for (field, set) in [
            ("after", value.after.is_some()),
            ("fails", !value.fails.is_empty()),
        ] {
            if set {
                problems.push(problem(
                    scorer,
                    scorer.lines.field(field),
                    format!("`{field}` is for kind `held`, not `{}`", value.kind.name()),
                ));
            }
        }
        return;
    }
    if value.after.is_none() {
        problems.push(problem(
            scorer,
            scorer.lines.field("kind"),
            "a `held` scorer needs `after`: when to grade, such as \"merged + 7d\"".to_owned(),
        ));
    }
    let signals = HELD_SIGNALS.join(", ");
    if value.fails.is_empty() {
        problems.push(problem(
            scorer,
            scorer.lines.field("kind"),
            format!("a `held` scorer needs `fails`: some of {signals}"),
        ));
    }
    for signal in value
        .fails
        .iter()
        .filter(|signal| !HELD_SIGNALS.contains(&signal.as_str()))
    {
        problems.push(problem(
            scorer,
            scorer.lines.field("fails"),
            format!("`{signal}` is no signal a `held` scorer reads: use {signals}"),
        ));
    }
}

/// Why the scorer `name` cannot be part of a step's `gate`, if it cannot.
fn gate_refused(factory: &Factory, name: &str) -> Option<String> {
    match factory.scorers.get(name) {
        None => Some(format!("`{name}`, which is no scorer in scorers/")),
        Some(scorer) if scorer.value.on == ScoreOn::Ask => Some(format!(
            "`{name}`, which reads asks: a gate grades a result, so use a scorer on result or diff"
        )),
        Some(scorer) if scorer.value.on == ScoreOn::Merge => Some(format!(
            "`{name}`, which reads what followed a merge: a gate grades a result, so use a scorer on result or diff"
        )),
        Some(_) => None,
    }
}

/// A judge's file beside its scorer: `file` resolved from the scorer's folder, or why it cannot be.
pub fn beside(factory: &Factory, scorer: &Loaded<Scorer>, file: &Path) -> Result<PathBuf, String> {
    let named = file.display();
    if file.is_absolute() || file.components().any(|part| part.as_os_str() == "..") {
        return Err(format!("{named}, which is not beside the scorer"));
    }
    let folder = scorer.file.parent().unwrap_or(Path::new(""));
    let path = factory.dir.join(folder).join(file);
    match path.is_file() {
        true => Ok(path),
        false => Err(format!(
            "{named}, which is missing from {}",
            folder.join("").display()
        )),
    }
}

fn check_judge(factory: &Factory, scorer: &Loaded<Scorer>, problems: &mut Vec<Problem>) {
    let value = &scorer.value;
    let at = |field| scorer.lines.field(field).or(scorer.lines.field("kind"));
    if value
        .model
        .as_deref()
        .is_none_or(|model| model.trim().is_empty())
    {
        problems.push(problem(
            scorer,
            at("model"),
            "a `judge` scorer needs `model`: the model that grades".to_owned(),
        ));
    }
    match &value.prompt {
        None => problems.push(problem(
            scorer,
            at("kind"),
            "a `judge` scorer needs `prompt`: a file beside it with the judge's whole prompt"
                .to_owned(),
        )),
        Some(file) => {
            if let Err(why) = beside(factory, scorer, file) {
                problems.push(problem(
                    scorer,
                    at("prompt"),
                    format!("`prompt` names {why}"),
                ));
            }
        }
    }
    match &value.evaluation {
        None => problems.push(problem(
            scorer,
            at("kind"),
            "a `judge` scorer needs `evaluation`: a file beside it with its labeled cases"
                .to_owned(),
        )),
        Some(file) => match beside(factory, scorer, file) {
            Err(why) => problems.push(problem(
                scorer,
                at("evaluation"),
                format!("`evaluation` names {why}"),
            )),
            Ok(path) => {
                if let Err(why) = labels::load(&path) {
                    problems.push(problem(
                        scorer,
                        at("evaluation"),
                        format!("`evaluation` {}: {why}", file.display()),
                    ));
                }
            }
        },
    }
    if let Some(agreement) = value.agreement
        && !(agreement > 0.0 && agreement <= 1.0)
    {
        problems.push(problem(
            scorer,
            at("agreement"),
            format!("`agreement` is {agreement}: give a share above 0 and at most 1, such as 0.8"),
        ));
    }
}

/// The warnings `priory check` prints and does not refuse: a judge whose model is also an agent's,
/// since a model tends to prefer its own work.
pub fn warnings(factory: &Factory) -> Vec<Problem> {
    let mut warnings = Vec::new();
    for scorer in factory.scorers.values() {
        let Some(model) = scorer.value.model.as_deref() else {
            continue;
        };
        if scorer.value.kind != ScoreKind::Judge {
            continue;
        }
        let same: Vec<&str> = factory
            .agents
            .iter()
            .filter(|(_, agent)| agent.value.model.as_deref() == Some(model))
            .map(|(name, _)| name.as_str())
            .collect();
        if !same.is_empty() {
            warnings.push(problem(
                scorer,
                scorer.lines.field("model"),
                format!(
                    "the judge's model `{model}` is also the model of {}: a model tends to prefer its own work",
                    same.iter()
                        .map(|name| format!("agent `{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    warnings.extend(caps_warnings(factory));
    warnings
}

/// Claude Code's model aliases: a transcript names the full model, which no price key can predict.
const CLAUDE_ALIASES: &[&str] = &[
    "default",
    "sonnet",
    "opus",
    "haiku",
    "opusplan",
    "sonnet[1m]",
];

const CAPS_PRICED: &str = "needs prices.toml, which turns the steps' tokens into dollars";

/// A spend cap counts dollars, so a factory with a cap and no `prices.toml` would never reach it.
fn check_caps(factory: &Factory, problems: &mut Vec<Problem>) {
    if !factory.prices.is_empty() {
        return;
    }
    if let Some(settings) = &factory.settings
        && settings.value.caps.is_some()
    {
        let message = format!("`[caps]` {CAPS_PRICED}");
        problems.push(problem(settings, settings.lines.field("caps"), message));
    }
    for trigger in factory.triggers.values() {
        if trigger.value.caps.is_some() {
            let message = format!("`caps` {CAPS_PRICED}");
            problems.push(problem(trigger, trigger.lines.field("caps"), message));
        }
    }
}

/// The agents of model steps whose spend no cap can count: one whose model `prices.toml` does not
/// price. A Claude Code alias is let through, since Claude's transcript names the full model; Codex
/// names the model it was given. Only for a factory with a cap and a price table.
fn caps_warnings(factory: &Factory) -> Vec<Problem> {
    let capped = factory
        .settings
        .as_ref()
        .is_some_and(|settings| settings.value.caps.is_some())
        || factory
            .triggers
            .values()
            .any(|trigger| trigger.value.caps.is_some());
    if !capped || factory.prices.is_empty() {
        return Vec::new();
    }
    let named: BTreeSet<&str> = factory
        .workflows
        .values()
        .flat_map(|workflow| &workflow.value.steps)
        .filter(|step| step.decides == Decider::Model)
        .filter_map(|step| step.agent.as_deref())
        .collect();
    let mut warnings = Vec::new();
    for name in named {
        let Some(agent) = factory.agents.get(name) else {
            continue;
        };
        let alias = |model: &str| {
            matches!(agent.value.harness, Harness::ClaudeCode) && CLAUDE_ALIASES.contains(&model)
        };
        let Some(model) = agent
            .value
            .model
            .as_deref()
            .filter(|model| !alias(model))
            .filter(|model| !factory.prices.contains_key(*model))
        else {
            continue;
        };
        warnings.push(problem(
            agent,
            agent.lines.field("model"),
            format!(
                "the spend caps count nothing for agent `{name}`: prices.toml has no price for its model `{model}`"
            ),
        ));
    }
    warnings
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
    for name in lanes
        .value
        .lane
        .keys()
        .filter(|name| !lanes::well_formed(name))
    {
        problems.push(problem(
            lanes,
            lanes.lines.field("lane"),
            format!("`[lane]` `{name}` is not a lane such as `team/theme`"),
        ));
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

/// Refuses an empty or repeated name in an agent's `skills`, and, without a kit, a name that is no
/// skill of the factory.
fn check_skills(factory: &Factory, agent: &Loaded<Agent>, problems: &mut Vec<Problem>) {
    let Some(skills) = &agent.value.skills else {
        return;
    };
    let line = agent.lines.field("skills");
    let kit = factory
        .settings
        .as_ref()
        .is_some_and(|settings| settings.value.kit.is_some());
    let mut seen = BTreeSet::new();
    for name in skills {
        let message = if name.is_empty() {
            "`skills` holds an empty name".to_owned()
        } else if !seen.insert(name) {
            format!("`skills` names `{name}` twice")
        } else if !kit && !factory.skills.contains_key(name) {
            let known: Vec<&str> = factory.skills.keys().map(String::as_str).collect();
            format!(
                "`skills` names `{name}`, which is not a skill of this factory: {}",
                if known.is_empty() {
                    "it has none".to_owned()
                } else {
                    known.join(", ")
                }
            )
        } else {
            continue;
        };
        problems.push(problem(agent, line, message));
    }
}

/// Refuses a connector name that is empty, repeated, written with Claude Code's `claude.ai `
/// prefix, or whose tools Claude Code would name as another's, and any connector on a Codex agent.
/// Whether the person's account has the connector only the runner can tell, when the run starts.
fn check_connectors(agent: &Loaded<Agent>, problems: &mut Vec<Problem>) {
    let connectors = &agent.value.connectors;
    let line = agent.lines.field("connectors");
    if !connectors.is_empty() && matches!(agent.value.harness, Harness::Codex) {
        let message =
            "a codex agent cannot take `connectors`: claude.ai connectors reach Claude Code alone";
        problems.push(problem(agent, line, message.into()));
    }
    let mut seen: Vec<(&String, String)> = Vec::new();
    for name in connectors {
        let tools = servers::tool_server(&servers::connector_server(name));
        let message = if name.trim().is_empty() {
            "`connectors` holds an empty name".to_owned()
        } else if let Some(bare) = name.strip_prefix(servers::CONNECTOR) {
            format!(
                "`connectors` names `{name}`: write the connector's name as claude.ai shows it, `{bare}`, since Claude Code adds `{}` itself",
                servers::CONNECTOR
            )
        } else if seen.iter().any(|(other, _)| *other == name) {
            format!("`connectors` names `{name}` twice")
        } else if let Some((other, _)) = seen.iter().find(|(_, other)| *other == tools) {
            format!(
                "`connectors` names `{other}` and `{name}`, whose tools Claude Code names alike, mcp__{tools}__<tool>: keep one"
            )
        } else {
            seen.push((name, tools));
            continue;
        };
        problems.push(problem(agent, line, message));
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
    // What a Codex run gets as `developer_instructions`, as the daemon joins it.
    let name = file_name_of(agent);
    let instructions = join_instructions(
        &factory.codex_instructions(value),
        &factory.consult_index(&name),
    );
    let size = join_instructions(&instructions, &factory.prompt(value)).len();
    if size > CODEX_PROMPT_LIMIT {
        let mut parts = Vec::new();
        if !factory.instructions().is_empty() {
            parts.push(INSTRUCTIONS);
        }
        if !factory.memories.is_empty() {
            parts.push("the memory index");
        }
        if factory.skills.keys().any(|name| value.gets_skill(name)) {
            parts.push("the skill index");
        }
        if !factory.consult_index(&name).is_empty() {
            parts.push("the investigator index");
        }
        let (parts, they) = if parts.is_empty() {
            ("its own prompt".to_owned(), "it is")
        } else {
            (
                format!("{} and its own prompt", parts.join(", ")),
                "together they are",
            )
        };
        problems.push(problem(
            agent,
            agent.lines.field("harness"),
            format!("a codex agent takes {parts} on its command line, as `developer_instructions`: {they} {size} bytes, over the limit of {CODEX_PROMPT_LIMIT}"),
        ));
    }
    if value.idle_compact_at.is_some() {
        let message = "a codex agent cannot take `idleCompactAt`: a resumed Codex session sends no `SessionStart` before its first prompt, so Priory never knows when to type `/compact`";
        problems.push(problem(
            agent,
            agent.lines.field("idleCompactAt"),
            message.into(),
        ));
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

/// `auto_answer = "low"` would accept every ask an agent makes, which is `hands-off` with no reader.
fn check_auto_answer(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    if trigger.value.auto_answer == Some(Confidence::Low) {
        problems.push(problem(
            trigger,
            trigger.lines.field("auto_answer"),
            "`auto_answer = \"low\"` would answer every ask an agent makes: use `high` or `medium`, or leave it unset".into(),
        ));
    }
}

/// The agent's name: the folder that holds its `agent.md`.
fn file_name_of(agent: &Loaded<Agent>) -> String {
    agent
        .file
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The tools an investigator's `allow` may never name: each one writes. Its run's `claude` gets
/// them, and [`SUBAGENT_TOOLS`], in `--disallowedTools` too.
const WRITING_TOOLS: [&str; 5] = ["Edit", "MultiEdit", "Write", "NotebookEdit", "Bash"];

/// The tools that start a Claude Code subagent, whose own `permissionMode` an investigator's
/// rules would not bind.
const SUBAGENT_TOOLS: [&str; 2] = ["Agent", "Task"];

/// An investigator reads and never writes: every tool it may call is named, none of them writes,
/// and each MCP server it names carries a person's statement of where its limit is enforced.
fn check_investigator(
    factory: &Factory,
    name: &str,
    agent: &Loaded<Agent>,
    problems: &mut Vec<Problem>,
) {
    let value = &agent.value;
    if !value.investigator {
        return;
    }
    let mut refuse = |field: &str, message: String| {
        problems.push(problem(
            agent,
            agent
                .lines
                .field(field)
                .or(agent.lines.field("investigator")),
            format!("investigator `{name}` {message}"),
        ));
    };
    if !matches!(value.harness, Harness::ClaudeCode) {
        refuse(
            "harness",
            "needs harness = \"claude-code\": only Claude Code takes per-tool rules".into(),
        );
    }
    if !matches!(
        value.permissions.default_mode,
        Some(PermissionMode::DontAsk)
    ) {
        refuse(
            "permissions",
            "needs permissions.defaultMode = \"dontAsk\", so every tool it may call is named in `allow`".into(),
        );
    }
    for rule in &value.permissions.allow {
        let tool = rule.split('(').next().unwrap_or_default().trim();
        let why = if WRITING_TOOLS.contains(&tool) {
            "names a tool that writes"
        } else if SUBAGENT_TOOLS.contains(&tool) {
            "starts a subagent, which can carry its own permission mode"
        } else if rule.contains('*') {
            "holds a `*`: write out each tool by its exact name"
        } else if tool.starts_with("mcp__") && !tool["mcp__".len()..].contains("__") {
            "names a whole MCP server: write out each of its tools by its exact name"
        } else {
            continue;
        };
        refuse("permissions", format!("may not allow `{rule}`: it {why}"));
    }
    let stated = |server: &String| {
        factory
            .settings
            .as_ref()
            .and_then(|settings| settings.value.mcp.get(server))
            .and_then(|mcp| mcp.read_only.as_deref())
            .is_some_and(|sentence| !sentence.trim().is_empty())
    };
    for server in value.mcp_servers.iter().filter(|server| !stated(server)) {
        refuse(
            "mcpServers",
            format!(
                "names MCP server `{server}`, which needs a `read_only` sentence under [mcp.{server}] in factory.toml: where the service enforces that it only reads"
            ),
        );
    }
    // A connector loads with every other one of the account, so only its exact tools may be allowed.
    for connector in &value.connectors {
        let server = servers::connector_server(connector);
        let tools = format!("mcp__{}__", servers::tool_server(&server));
        if !stated(&server) {
            refuse(
                "connectors",
                format!(
                    "names connector `{connector}`, which needs a `read_only` sentence under [mcp.\"{server}\"] in factory.toml: where the service enforces that it only reads"
                ),
            );
        } else if !value
            .permissions
            .allow
            .iter()
            .any(|rule| rule.starts_with(&tools))
        {
            refuse(
                "connectors",
                format!(
                    "names connector `{connector}`, but its `allow` names none of the connector's tools: write each as {tools}<tool>"
                ),
            );
        }
    }
    if matches!(value.session, Session::Shared) {
        refuse(
            "session",
            "cannot take session = \"shared\": each consult starts a fresh session, so no task's data reaches another's report".into(),
        );
    }
    if value.connector_sign_in {
        refuse(
            "connectorSignIn",
            "cannot take `connectorSignIn`: an unattended investigator never signs in".into(),
        );
    }
    match &value.model {
        None => refuse(
            "model",
            "needs a `model` with a price in prices.toml, so its cap can be enforced".into(),
        ),
        Some(model) if !factory.prices.contains_key(model) => refuse(
            "model",
            format!(
                "runs model `{model}`, which prices.toml does not price, so its cap cannot be enforced"
            ),
        ),
        Some(_) => {}
    }
}

/// Each line of `[consult.allow]` names an agent that is no investigator, and investigators only.
fn check_consult(factory: &Factory, problems: &mut Vec<Problem>) {
    let Some(settings) = &factory.settings else {
        return;
    };
    let Some(consult) = &settings.value.consult else {
        return;
    };
    let line = settings.lines.field("consult");
    let investigator = |name: &str| {
        factory
            .agents
            .get(name)
            .map(|agent| agent.value.investigator)
    };
    for (asker, targets) in &consult.allow {
        let refused = match investigator(asker) {
            None => Some(format!(
                "`[consult.allow]` names `{asker}`, which is no agent"
            )),
            Some(true) => Some(format!(
                "`[consult.allow]` names investigator `{asker}` as an asker: an investigator cannot consult, so a consult is one hop"
            )),
            Some(false) => None,
        };
        if let Some(message) = refused {
            problems.push(problem(settings, line, message));
        }
        for target in targets {
            let refused = match investigator(target) {
                None => format!(
                    "`[consult.allow]` lets `{asker}` consult `{target}`, which is no agent"
                ),
                Some(false) => format!(
                    "`[consult.allow]` lets `{asker}` consult `{target}`, which is no investigator: set `investigator = true` on it"
                ),
                Some(true) => continue,
            };
            problems.push(problem(settings, line, refused));
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
    if agent.value.investigator {
        problems.push(problem(
            trigger,
            at,
            format!("lead `{lead}` is an investigator, which only answers consults"),
        ));
        return;
    }
    let shared = matches!(agent.value.session, Session::Shared);
    if !shared || !matches!(agent.value.harness, Harness::ClaudeCode) {
        problems.push(problem(
            trigger,
            at,
            format!("lead `{lead}` needs harness = \"claude-code\" and session = \"shared\""),
        ));
    }
}

/// An `intent` is one sentence on one line, and each `noop` case says something.
fn check_intent(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    if let Some(intent) = &trigger.value.intent {
        let lines = intent.trim().lines().count();
        let message = match lines {
            0 => Some(
                "`intent` is empty: say in one sentence the outcome the trigger exists for"
                    .to_owned(),
            ),
            1 => None,
            n => Some(format!(
                "`intent` is one sentence on one line, not {n} lines"
            )),
        };
        if let Some(message) = message {
            problems.push(problem(trigger, trigger.lines.field("intent"), message));
        }
    }
    for (at, case) in trigger.value.noop.iter().enumerate() {
        if case.trim().is_empty() {
            problems.push(problem(
                trigger,
                trigger.lines.field("noop"),
                format!(
                    "`noop` case {} is empty: say which case must create no ask and no write",
                    at + 1
                ),
            ));
        }
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

/// The comments and reviews on a pull request a GitHub trigger can start a task from.
pub const FEEDBACK: [&str; 3] = [
    "pull_request.comment",
    "pull_request_review_comment",
    "pull_request_review.submitted",
];

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

/// Why a repository's recipe `name` cannot run: no command, two, or one the guard holds
/// irreversible, since a recipe runs with no person's approval.
fn recipe_problem(guards: &crate::guard::Guards, name: &str, recipe: &Recipe) -> Option<String> {
    match (&recipe.argv, &recipe.shell) {
        (Some(_), Some(_)) => {
            return Some(format!("{name} needs one of `argv` or `shell`, not both"));
        }
        (None, None) => return Some(format!("{name} needs one of `argv` or `shell`")),
        (Some(argv), None) if argv.first().is_none_or(String::is_empty) => {
            return Some(format!("{name} needs a program in `argv`"));
        }
        (None, Some(shell)) if shell.trim().is_empty() => {
            return Some(format!("{name} needs a command in `shell`"));
        }
        _ => {}
    }
    let command = recipe.command();
    let hit = guards
        .hits("Bash", Some(&command), &|_, _| None)
        .into_iter()
        .next()?;
    Some(format!(
        "{name} runs `{}`, which the guard `{}` holds irreversible: a recipe runs with no person's approval",
        hit.call, hit.guard
    ))
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
    let feedback = FEEDBACK.contains(&value.on.as_str());
    if !ON.contains(&value.on.as_str()) && !feedback {
        problems.push(problem(
            trigger,
            trigger.lines.field("on").or(trigger.lines.field("source")),
            format!(
                "a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled`, `pull_request.opened`, `{}` or `{REVIEW_REQUESTED}`, not `{}`",
                FEEDBACK.join("`, `"),
                value.on
            ),
        ));
    }
    let label_only = |filter: &BTreeMap<String, String>| {
        value.on.ends_with(".labeled") && filter.keys().all(|key| key == "label")
    };
    let by_people = |filter: &BTreeMap<String, String>| {
        filter.contains_key("author")
            && filter
                .keys()
                .all(|key| ["author", "commenter"].contains(&key.as_str()))
    };
    if feedback && !value.filter.as_ref().is_some_and(by_people) {
        problems.push(problem(
            trigger,
            trigger.lines.field("filter").or(trigger.lines.field("on")),
            format!(
                "a `{}` trigger needs `filter = {{ author = \"<GitHub login>\" }}`, and takes `commenter` too",
                value.on
            ),
        ));
    }
    if !feedback && !value.filter.as_ref().is_none_or(label_only) {
        problems.push(problem(
            trigger,
            trigger.lines.field("filter"),
            "`filter` works only with `on = \"*.labeled\"`, on `label`".into(),
        ));
    }
    let number = if value.on.starts_with("pull_request") {
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

const FIGMA: Hooked = Hooked {
    source: "figma",
    vendor: "Figma",
    on: "file.comment_created",
    filters: &["file"],
    key: "file.key",
    events: "comments",
};

const CLOUDWATCH: Hooked = Hooked {
    source: "cloudwatch",
    vendor: "CloudWatch",
    on: "alarm.state_changed",
    filters: &["name", "state", "previous_state"],
    key: "alarm.arn",
    events: "alarm state changes",
};

const INTERCOM: Hooked = Hooked {
    source: "intercom",
    vendor: "Intercom",
    on: "conversation.created",
    filters: &[],
    key: "conversation.id",
    events: "conversations",
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
    let a = if source.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
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
                "{a} `{source}` trigger needs `[webhooks.{source}]` in factory.toml: {vendor} {} arrive only by webhook",
                hooked.events
            ),
        );
    }
    if value.on != *on {
        report(
            "on",
            format!("{a} `{source}` trigger handles `{on}`, not `{}`", value.on),
        );
    }
    if value.filter.as_ref().is_some_and(|filter| {
        filter
            .keys()
            .any(|key| !hooked.filters.contains(&key.as_str()))
    }) {
        let message = match hooked.filters {
            [] => format!("{a} `{source}` trigger takes no `filter`"),
            filters => format!(
                "{a} `{source}` trigger filters on `{}`",
                filters.join("` or `")
            ),
        };
        report("filter", message);
    }
    if value.repo.as_deref().is_some_and(|repo| !owner_name(repo)) {
        report(
            "repo",
            format!("{a} {vendor} trigger's `repo` is `owner/name`"),
        );
    }
    if value.key.as_deref().is_some_and(|key| key != hooked.key) {
        report(
            "key",
            format!("{a} {vendor} trigger's `key` is `{}`", hooked.key),
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

/// A `[pre]` script sits on any trigger but `cli` and is an executable file inside the factory
/// folder, with a timeout of at most 60 seconds and `env` names that are variable names.
fn check_pre(factory: &Factory, trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let Some(pre) = &trigger.value.pre else {
        return;
    };
    let mut report = |message: String| {
        problems.push(problem(trigger, trigger.lines.field("pre"), message));
    };
    if trigger.value.source == "cli" {
        report(
            "`[pre]` runs on every source except `triggers/cli.toml`: priory add and priory request never run it"
                .into(),
        );
        return;
    }
    match pre.argv.first() {
        None => report("`[pre] argv` needs a program".into()),
        Some(program) => {
            if let Some(refused) = inside(&factory.dir, program) {
                report(format!("`[pre] argv`: {refused}"));
            }
        }
    }
    if pre.timeout() > crate::factory::Pre::MAX_TIMEOUT {
        report(format!(
            "`[pre] timeout` is {}s: a pre script runs at most 60s",
            pre.timeout().as_secs()
        ));
    }
    for name in pre.env.iter().filter(|name| !variable(name)) {
        report(format!("`[pre] env`: `{name}` is not a variable name"));
    }
}

/// Why `program` is not an executable file inside `dir`, if it is not.
fn inside(dir: &Path, program: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let path = Path::new(program);
    let leaves = path.is_absolute()
        || path
            .components()
            .any(|part| part == std::path::Component::ParentDir);
    let resolved = dir.join(path).canonicalize();
    let folder = dir.canonicalize();
    match (resolved, folder) {
        _ if leaves => Some(format!("{program} leaves the factory folder")),
        (Ok(resolved), Ok(folder)) if !resolved.starts_with(&folder) => {
            Some(format!("{program} leaves the factory folder"))
        }
        (Ok(resolved), _) => match std::fs::metadata(&resolved) {
            Ok(file) if file.is_file() && file.permissions().mode() & 0o111 == 0 => {
                Some(format!("{program} is not executable: chmod +x it"))
            }
            Ok(file) if file.is_file() => None,
            _ => Some(format!("{program} is not a file in the factory folder")),
        },
        (Err(_), _) => Some(format!("{program} is not a file in the factory folder")),
    }
}

/// Whether `name` can name an environment variable.
fn variable(name: &str) -> bool {
    name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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
    if let Some(sandbox) = &workflow.value.sandbox {
        let declared = factory
            .settings
            .as_ref()
            .is_some_and(|settings| settings.value.sandboxes.contains_key(sandbox));
        if !declared {
            problems.push(problem(
                workflow,
                workflow.lines.field("sandbox"),
                format!("`sandbox` names `{sandbox}`, which `[sandboxes]` in factory.toml does not declare"),
            ));
        }
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
            (step.question.is_some(), Decider::Human, "a `question`"),
            (!step.approves.is_empty(), Decider::Human, "`approves`"),
            (step.command.is_some(), Decider::Human, "a `command`"),
            (step.budget.is_some(), Decider::Model, "a `budget`"),
            (step.record.is_some(), Decider::Model, "a `record` command"),
            (step.attach, Decider::Model, "`attach = true`"),
            (step.action.is_some(), Decider::Code, "an `action`"),
            (step.run.is_some(), Decider::Code, "a `run` command"),
        ];
        for (_, owner, field) in fields
            .iter()
            .filter(|(set, owner, _)| *set && *owner != step.decides)
        {
            report(format!("only a {} step has {field}", owner.name()));
        }
        if step.debate.is_some() && step.decides != Decider::Model {
            report("only a model step has a `debate`".into());
        }
        match step.decides {
            Decider::Model => {
                match (&step.agent, &step.debate) {
                    (Some(_), Some(_)) => {
                        report("a debate step has no `agent`: its `debate` names both".into())
                    }
                    (Some(agent), None) if !factory.agents.contains_key(agent) => {
                        report(format!("agent `{agent}` does not exist"))
                    }
                    (Some(agent), None)
                        if factory.agents[agent].value.investigator
                            && !built_in(factory, workflow) =>
                    {
                        report(format!(
                            "agent `{agent}` is an investigator, which only answers consults"
                        ))
                    }
                    (_, Some(debate)) => {
                        for message in debate_problems(factory, step, debate) {
                            report(message);
                        }
                        for agent in debate.agents.iter().filter(|agent| {
                            factory
                                .agents
                                .get(*agent)
                                .is_some_and(|agent| agent.value.investigator)
                        }) {
                            report(format!(
                                "agent `{agent}` is an investigator, which only answers consults"
                            ));
                        }
                    }
                    (Some(_), None) => {}
                    (None, None) => report("a model step needs an `agent`".into()),
                }
                if step.prompt.is_none() {
                    report("a model step needs a `prompt`".into());
                }
                if let Some(record) = &step.record
                    && let Err(refused) = recorder(&factory.dir, record)
                {
                    report(format!("`record`: {refused}"));
                }
                if step.attach && factory.attach().is_none() {
                    report(
                        "`attach = true` needs `[runs] attach` in the factory's settings".into(),
                    );
                }
                if let Some(budget) = &step.budget {
                    if step.budget().is_none() {
                        report(format!(
                            "`budget` {budget:?} is not an amount of dollars above zero, such as \"1.00\""
                        ));
                    }
                    if factory.prices.is_empty() {
                        report(
                            "`budget` needs prices.toml, which turns the step's tokens into dollars"
                                .into(),
                        );
                    }
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
                for message in unknown_fields("question", step.question.iter()) {
                    report(message);
                }
                if let Some(command) = &step.command {
                    for message in unknown_fields("command argv", command.argv.iter()) {
                        report(message);
                    }
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
                    && factory.settings.as_ref().is_none_or(|settings| {
                        settings.value.kit.is_none() && settings.value.memories.is_none()
                    }) =>
                {
                    report(format!(
                        "an `{OPEN_MEMORIES_PR}` action writes to the memories repository or the kit, so factory.toml needs a `[memories]` or a `[kit]`"
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
        if !step.gate.is_empty() && step.decides != Decider::Model {
            report("only a model step has a `gate`".into());
        }
        for (outcome, scorers) in &step.gate {
            if !step.next.contains_key(outcome) {
                report(format!(
                    "`gate` has outcome `{outcome}`, which is not in `next`"
                ));
            }
            for name in scorers {
                if let Some(why) = gate_refused(factory, name) {
                    report(format!("`gate` of outcome `{outcome}` names {why}"));
                }
            }
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

/// What is wrong with a debate step's `debate` and the keys beside it.
fn debate_problems(factory: &Factory, step: &Step, debate: &Debate) -> Vec<String> {
    let mut problems = Vec::new();
    match debate.agents.as_slice() {
        [first, second] if first == second => problems.push(format!(
            "a debate's two agents must differ: `{first}` is named twice"
        )),
        [_, _] => {}
        agents => problems.push(format!(
            "a debate needs exactly two agents, in speaking order, not {}",
            agents.len()
        )),
    }
    if !(1..=5).contains(&debate.rounds) {
        problems.push(format!(
            "`rounds` is {}; a debate takes 1 to 5 rounds",
            debate.rounds
        ));
    }
    if !matches!(step.effect, Effect::None | Effect::Read) {
        problems.push(format!(
            "a debate step's effect is `none` or `read`, not `{}`: debaters judge and change nothing",
            step.effect.name()
        ));
    }
    if !step.proof.is_empty() {
        problems.push("a debate step has no `proof`: debaters judge and change nothing".into());
    }
    if step.timeout.is_none() {
        problems.push("a debate step needs a `timeout`: its cap cannot bound one turn".into());
    }
    let mut named = BTreeSet::new();
    for name in &debate.agents {
        if !named.insert(name) {
            continue;
        }
        let Some(agent) = factory.agents.get(name) else {
            problems.push(format!("agent `{name}` does not exist"));
            continue;
        };
        if matches!(agent.value.session, Session::Shared) {
            problems.push(format!(
                "agent `{name}` keeps a shared session, and a debater needs its own"
            ));
        }
        match &agent.value.model {
            Some(model) if factory.prices.contains_key(model) => {}
            Some(model) => problems.push(format!(
                "agent `{name}`'s model `{model}` has no price in prices.toml, so the debate's cap cannot count it"
            )),
            None => problems.push(format!(
                "agent `{name}` names no model, so prices.toml cannot price it for the debate's cap"
            )),
        }
    }
    problems
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

/// Whether `workflow` is one Priory builds, such as a lead's or an investigator's: its own name
/// has a `/`, which no workflow file's has, and its problems are reported where it comes from.
fn built_in(factory: &Factory, workflow: &Loaded<Workflow>) -> bool {
    factory
        .workflows
        .iter()
        .any(|(name, other)| name.starts_with("priory/") && std::ptr::eq(other, workflow))
}

/// A problem for each workflow that turns a sandbox on, when this machine has no `bwrap`.
pub fn missing_bwrap(factory: &Factory, installed: bool) -> Vec<Problem> {
    if installed {
        return Vec::new();
    }
    factory
        .workflows
        .values()
        .filter_map(|workflow| {
            let sandbox = workflow.value.sandbox.as_ref()?;
            Some(problem(
                workflow,
                workflow.lines.field("sandbox"),
                format!(
                    "`sandbox` turns on sandbox `{sandbox}`, and `bwrap` is not on PATH: install bubblewrap, or remove `sandbox`"
                ),
            ))
        })
        .collect()
}

/// What is wrong with `[sandboxes.<name>]`: a path that is not absolute or under `~/`, or is the
/// home itself, and a variable that is no name, which the run's shell could not keep, or that
/// holds one of the factory's secrets.
fn sandbox_problems(
    name: &str,
    sandbox: &crate::factory::Sandbox,
    secrets: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    for (field, paths) in [("read", &sandbox.read), ("write", &sandbox.write)] {
        for path in paths {
            let rooted = path.starts_with('/') || path.starts_with("~/");
            let trimmed = path.trim_end_matches('/');
            let why = if trimmed == "~" {
                "the home itself, which would undo the jail"
            } else if rooted && trimmed.is_empty() {
                "the whole file system, which would undo the jail"
            } else if !rooted {
                "not absolute or under `~/`"
            } else {
                continue;
            };
            problems.push(format!(
                "sandbox `{name}` has `{field}` path `{path}`, which is {why}"
            ));
        }
    }
    for variable in &sandbox.env {
        let named = variable
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && variable
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '_');
        if !named {
            problems.push(format!(
                "sandbox `{name}` keeps `{variable}` in `env`, which is not a variable name"
            ));
        } else if secrets.contains(variable) {
            problems.push(format!(
                "sandbox `{name}` keeps `{variable}` in `env`, which holds a secret of the factory"
            ));
        }
    }
    problems
}

fn problem<T>(loaded: &Loaded<T>, line: Option<usize>, message: String) -> Problem {
    Problem {
        file: loaded.file.clone(),
        at: line.map(|line| line.to_string()),
        message,
    }
}

/// A problem for each `{name}` in `texts` that is not a field of a task's event, once per name.
fn unknown_fields<'a>(what: &str, texts: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut unknown: Vec<&str> = Vec::new();
    for name in texts.flat_map(|text| crate::factory::placeholders(text)) {
        if !crate::factory::EVENT_FIELDS.contains(&name) && !unknown.contains(&name) {
            unknown.push(name);
        }
    }
    unknown
        .into_iter()
        .map(|name| {
            format!(
                "{what} names `{{{name}}}`, which is not a field of the task's event: use {{repo}}, {{pull_request}}, {{head_sha}} or {{base_branch}}"
            )
        })
        .collect()
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
    let mut problems = repo_problems("kit", &kit.repo, kit.reference.as_deref());
    problems.extend(path_problems("kit", "a kit path", &kit.paths()));
    problems
}

/// What `priory check` refuses in `[memories]`, as in `[kit]`.
fn memories_problems(memories: &crate::factory::MemoriesRepo) -> Vec<String> {
    let mut problems = repo_problems("memories", &memories.repo, memories.reference.as_deref());
    problems.extend(path_problems(
        "memories",
        "a memories path",
        &[("path", memories.path())],
    ));
    problems
}

/// An empty `repo` or `ref` of `[table]`, or one git would read as an option.
fn repo_problems(table: &str, repo: &str, reference: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, value) in [("repo", Some(repo)), ("ref", reference)] {
        match value.map(str::trim) {
            Some("") => problems.push(format!("`[{table}]` `{key}` is empty")),
            Some(value) if value.starts_with('-') => problems.push(format!(
                "`[{table}]` `{key}` is `{value}`: it cannot start with `-`"
            )),
            _ => {}
        }
    }
    problems
}

/// Each path of `[table]` that is empty, absolute, or leaves the repository.
fn path_problems(table: &str, what: &str, paths: &[(&str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, path) in paths {
        let inside = !path.is_empty()
            && std::path::Path::new(path)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)));
        if !inside {
            problems.push(format!(
                "`[{table}]` `{key}` is `{path}`: {what} is relative and stays inside the repository"
            ));
        }
    }
    problems
}

/// Why the command `named`, under the factory folder `dir`, is not an executable file, if it is not.
fn not_executable(dir: &Path, named: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let shown = named.display();
    match std::fs::metadata(dir.join(named)) {
        Ok(file) if file.is_file() && file.permissions().mode() & 0o111 == 0 => {
            Some(format!("{shown} is not executable: chmod +x it"))
        }
        Ok(file) if file.is_file() => None,
        _ => Some(format!("{shown} is not a file in the factory folder")),
    }
}

/// Why the `record` command `record` is not an executable file inside the factory folder `dir`, as
/// through `..` or a symbolic link that leads out.
pub fn recorder(dir: &Path, record: &Path) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let shown = record.display();
    let folder = dir
        .canonicalize()
        .map_err(|why| format!("cannot read the factory folder: {why}"))?;
    let program = dir
        .join(record)
        .canonicalize()
        .map_err(|_| format!("{shown} is not a file in the factory folder"))?;
    if !program.starts_with(&folder) {
        return Err(format!("{shown} leads outside the factory folder"));
    }
    match std::fs::metadata(&program) {
        Ok(file) if file.is_file() && file.permissions().mode() & 0o111 == 0 => {
            Err(format!("{shown} is not executable: chmod +x it"))
        }
        Ok(file) if file.is_file() => Ok(program),
        _ => Err(format!("{shown} is not a file in the factory folder")),
    }
}

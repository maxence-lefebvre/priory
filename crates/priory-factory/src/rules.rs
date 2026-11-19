use crate::factory::{
    Agent, Decider, Effect, Factory, Harness, Loaded, PermissionMode, Problem, Step, Trigger,
    Until, Workflow,
};
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
        if let Some(name) = &settings.value.handler {
            let refused = match factory.agents.get(name) {
                None => Some(format!("the handler agent `{name}` does not exist")),
                Some(agent) if !matches!(agent.value.harness, Harness::ClaudeCode) => Some(
                    format!("the handler agent `{name}` needs harness = \"claude-code\""),
                ),
                Some(_) => None,
            };
            if let Some(message) = refused {
                problems.push(problem(settings, settings.lines.field("handler"), message));
            }
        }
        if !owner_name(settings.value.sweep.repo()) {
            problems.push(problem(
                settings,
                settings.lines.field("sweep"),
                "`[sweep] repo` needs to be `owner/name`".into(),
            ));
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
        match trigger.value.source.as_str() {
            "github" => check_github(trigger, &mut problems),
            "linear" | "linear.view" => check_linear(trigger, &mut problems),
            "slack" => check_slack(trigger, &mut problems),
            _ if trigger.value.backfill.is_some() => problems.push(problem(
                trigger,
                trigger.lines.field("backfill"),
                "only a GitHub, Linear or Slack trigger has `backfill`".into(),
            )),
            _ => {}
        }
        if trigger.value.source != "slack" && trigger.value.channel.is_some() {
            problems.push(problem(
                trigger,
                trigger.lines.field("channel"),
                "only a Slack trigger names a `channel`".into(),
            ));
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
        check_codex(agent, &mut problems);
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
    problems
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

/// Refuses the Claude Code permissions a Codex run cannot apply.
fn check_codex(agent: &Loaded<Agent>, problems: &mut Vec<Problem>) {
    let value = &agent.value;
    if !matches!(value.harness, Harness::Codex) {
        return;
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

/// Whether `repo` reads `owner/name`.
fn owner_name(repo: &str) -> bool {
    repo.split_once('/')
        .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
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
                "a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled` or `pull_request.opened`, not `{}`",
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

fn check_linear(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    let view = value.source == "linear.view";
    let (on, filters): (&[&str], &[&str]) = if view {
        (&["issue.entered", "issue.left"], &[])
    } else {
        (
            &["issue.created", "issue.state_changed", "comment.created"],
            &["team", "label", "state"],
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
            "a `linear` trigger filters on `team`, `label` or `state`".to_string()
        };
        report("filter", message);
    }
    if value.repo.is_some() {
        report("repo", "a Linear trigger names no `repo`".into());
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
    if value.repo.is_some() || value.view.is_some() {
        report("source", "a Slack trigger names no `repo` or `view`".into());
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
    let start = workflow.value.start.as_str();
    if !steps.contains_key(start) {
        problems.push(problem(
            workflow,
            workflow.lines.field("start"),
            format!("start `{start}` is not a step"),
        ));
        return;
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
            (!step.options.is_empty(), Decider::Human, "`options`"),
            (!step.approves.is_empty(), Decider::Human, "`approves`"),
            (step.action.is_some(), Decider::Code, "an `action`"),
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
            }
            Decider::Code if step.action.is_none() => {
                report("a code step needs an `action`".into())
            }
            Decider::Code => {}
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

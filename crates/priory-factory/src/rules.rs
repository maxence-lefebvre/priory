use crate::factory::{Decider, Effect, Factory, Loaded, Problem, Step, Trigger, Workflow};
use std::collections::{BTreeMap, BTreeSet};

const ENDS: [&str; 2] = ["done", "stop"];

pub fn check(factory: &Factory) -> Vec<Problem> {
    let mut problems = Vec::new();
    for trigger in factory.triggers.values() {
        if !factory.workflows.contains_key(&trigger.value.workflow) {
            problems.push(problem(
                trigger,
                trigger.lines.field("workflow"),
                format!("workflow `{}` does not exist", trigger.value.workflow),
            ));
        }
        if trigger.value.source == "github" {
            check_github(trigger, &mut problems);
        } else if trigger.value.backfill.is_some() {
            problems.push(problem(
                trigger,
                trigger.lines.field("backfill"),
                "only a GitHub trigger has `backfill`".into(),
            ));
        }
    }
    for workflow in factory.workflows.values() {
        check_workflow(factory, workflow, &mut problems);
    }
    problems
}

fn check_github(trigger: &Loaded<Trigger>, problems: &mut Vec<Problem>) {
    let value = &trigger.value;
    let repo = value.repo.as_deref().and_then(|repo| repo.split_once('/'));
    if !repo
        .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
    {
        problems.push(problem(
            trigger,
            trigger
                .lines
                .field("repo")
                .or(trigger.lines.field("source")),
            "a GitHub trigger needs `repo = \"owner/name\"`".into(),
        ));
    }
    const ON: [&str; 4] = [
        "issues.labeled",
        "issues.opened",
        "pull_request.labeled",
        "pull_request.opened",
    ];
    if !ON.contains(&value.on.as_str()) {
        problems.push(problem(
            trigger,
            trigger.lines.field("on"),
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
fn reachable<'a>(
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

use crate::factory::{By, Confidence, Decider, Effect, Factory, Harness, Proof, Step, Workflow};
use std::collections::BTreeSet;

/// A guarded call an agent can make with no human decision, in a workflow, and why.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reach {
    pub agent: String,
    pub guard: String,
    pub workflow: String,
    pub why: String,
}

/// Per agent, the guarded calls it can make with no human decision: those a workflow it runs a step
/// of allows, the guards an autonomous irreversible step it runs names, and, for a Codex agent with
/// MCP servers, MCP calls, which Codex may run without the guard's hook.
pub fn tools(factory: &Factory) -> Vec<Reach> {
    let guards = factory.guards();
    let mut reach = BTreeSet::new();
    let mut add = |agent: &str, guard: &str, workflow: &str, why: String| {
        reach.insert(Reach {
            agent: agent.to_owned(),
            guard: guard.to_owned(),
            workflow: workflow.to_owned(),
            why,
        });
    };
    for (name, workflow) in &factory.workflows {
        let model_steps = workflow
            .value
            .steps
            .iter()
            .filter(|step| step.decides == Decider::Model);
        for step in model_steps {
            let Some(agent) = &step.agent else {
                continue;
            };
            for guard in &workflow.value.guard.allow {
                add(agent, guard, name, "allowed by the workflow".to_owned());
            }
            if step.effect == Effect::Irreversible && step.autonomous.is_some() {
                for guard in &step.guard.allow {
                    add(agent, guard, name, format!("autonomous step {}", step.id));
                }
            }
        }
    }
    let codex = factory.agents.iter().filter(|(_, agent)| {
        matches!(agent.value.harness, Harness::Codex) && !agent.value.mcp_servers.is_empty()
    });
    for (agent, _) in codex {
        for guard in std::iter::once("mcp-write").chain(guards.tools.iter().map(String::as_str)) {
            add(
                agent,
                guard,
                "-",
                "codex: an MCP call may skip the guard's hook".to_owned(),
            );
        }
    }
    reach.into_iter().collect()
}

/// Each debate step of the workflow and its worst case, as (step, line): `2 agents, up to 8
/// turns, cap $3.00`, with a note when both agents run the same harness and model.
pub fn debates<'w>(factory: &Factory, workflow: &'w Workflow) -> Vec<(&'w str, String)> {
    workflow
        .steps
        .iter()
        .filter_map(|step| {
            let debate = step.debate.as_ref()?;
            let mut line = format!(
                "{} agents, up to {} turns, cap ${:.2}",
                debate.agents.len(),
                debate.turns(),
                debate.cap
            );
            let runs: Vec<_> = debate
                .agents
                .iter()
                .filter_map(|name| factory.agents.get(name))
                .map(|agent| (harness_name(&agent.value.harness), &agent.value.model))
                .collect();
            if let [first, second] = runs.as_slice()
                && first == second
            {
                line.push_str(&format!(
                    "; both agents run {} with model {}, so the debate mostly agrees with itself",
                    first.0,
                    first.1.as_deref().unwrap_or("default")
                ));
            }
            Some((step.id.as_str(), line))
        })
        .collect()
}

fn harness_name(harness: &Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "claude-code",
        Harness::Codex => "codex",
    }
}

/// An irreversible step and what lets it run: the human steps that approve it and lead to it, or
/// its `autonomous` reason.
#[derive(Debug, PartialEq, Eq)]
pub struct Gate {
    pub step: String,
    pub approved_by: Vec<String>,
    pub autonomous: Option<String>,
}

/// Each outcome of the workflow that the daemon verifies, as (step, outcome, proof).
pub fn proofs(workflow: &Workflow) -> Vec<(&str, &str, Proof)> {
    workflow
        .steps
        .iter()
        .flat_map(|step| {
            step.proof
                .iter()
                .map(|(outcome, proof)| (step.id.as_str(), outcome.as_str(), *proof))
        })
        .collect()
}

pub fn gates(workflow: &Workflow) -> Vec<Gate> {
    workflow
        .steps
        .iter()
        .filter(|step| step.effect == Effect::Irreversible)
        .map(|step| Gate {
            step: step.id.clone(),
            approved_by: workflow
                .steps
                .iter()
                .filter(|other| {
                    other.decides == Decider::Human
                        && other.approves.contains(&step.id)
                        && workflow.reaches(&other.id, &step.id)
                })
                .map(|other| other.id.clone())
                .collect(),
            autonomous: step.autonomous.clone(),
        })
        .collect()
}

/// Every path of steps from `start` to an irreversible step that follows no edge a human decides.
pub fn unattended_paths(workflow: &Workflow) -> Vec<Vec<String>> {
    let mut paths = BTreeSet::new();
    let mut path = vec![workflow.start.as_str()];
    walk(workflow, &mut path, &mut paths);
    paths.into_iter().collect()
}

fn walk<'a>(workflow: &'a Workflow, path: &mut Vec<&'a str>, paths: &mut BTreeSet<Vec<String>>) {
    let Some(step) = path.last().and_then(|id| workflow.step(id)) else {
        return;
    };
    if step.effect == Effect::Irreversible {
        paths.insert(path.iter().map(|id| id.to_string()).collect());
    }
    for target in unattended_targets(step) {
        if !path.contains(&target) {
            path.push(target);
            walk(workflow, path, paths);
            path.pop();
        }
    }
}

fn unattended_targets(step: &Step) -> BTreeSet<&str> {
    step.next
        .iter()
        .filter(|(outcome, _)| step.by(outcome) != By::Human)
        .map(|(_, target)| target.as_str())
        .collect()
}

/// Whether an ask made at step `at` may be answered by anyone but a person: a lead under
/// `hands-off`, or the daemon under `auto_answer`. Never an ask that runs a command, an ask at a
/// step that `approves`, or an ask whose answer can let an irreversible step run.
pub fn delegable(workflow: &Workflow, at: &str, command: bool) -> bool {
    undelegable(workflow, at, command).is_none()
}

/// Why `delegable` refuses an ask made at step `at`, in a clause such as `it runs a command`, or
/// `None` when anyone but a person may answer it.
pub fn undelegable(workflow: &Workflow, at: &str, command: bool) -> Option<String> {
    if command {
        return Some("it runs a command".to_owned());
    }
    if let Some(step) = workflow.step(at).filter(|step| !step.approves.is_empty()) {
        return Some(format!(
            "step {at} approves {}",
            step.approves
                .iter()
                .map(|approved| format!("irreversible step {approved}"))
                .collect::<Vec<_>>()
                .join(" and ")
        ));
    }
    irreversible_step_after(workflow, at).map(|step| {
        format!("answering it can let irreversible step {step} run before a person decides again")
    })
}

/// Whether the daemon answers an agent's ask at step `at`, stated at `confidence`, with its
/// recommendation, for a task whose `auto_answer` is `level`. The workflow's shape decides first,
/// through `delegable`, so the stated confidence never widens what a lead may answer.
pub fn auto_answerable(
    workflow: &Workflow,
    at: &str,
    command: bool,
    level: Option<Confidence>,
    confidence: Option<Confidence>,
) -> bool {
    let (Some(level), Some(confidence)) = (level, confidence) else {
        return false;
    };
    level != Confidence::Low && confidence >= level && delegable(workflow, at, command)
}

/// Whether answering the ask of human step `ask` can let an irreversible step run before a
/// person decides again.
pub fn answer_lets_irreversible_run(workflow: &Workflow, ask: &str) -> bool {
    irreversible_step_after(workflow, ask).is_some()
}

/// The first irreversible step, in workflow order, that answering the ask of human step `ask` can
/// let run before a person decides again.
pub fn irreversible_step_after<'a>(workflow: &'a Workflow, ask: &str) -> Option<&'a str> {
    let step = workflow.step(ask)?;
    let mut reached: BTreeSet<&str> = step.next.values().map(String::as_str).collect();
    let mut pending: Vec<&str> = reached.iter().copied().collect();
    while let Some(id) = pending.pop() {
        let Some(step) = workflow.step(id) else {
            continue;
        };
        for target in unattended_targets(step) {
            if reached.insert(target) {
                pending.push(target);
            }
        }
    }
    workflow
        .steps
        .iter()
        .find(|step| step.effect == Effect::Irreversible && reached.contains(step.id.as_str()))
        .map(|step| step.id.as_str())
}

/// How a step reads in a drawing: `ask` and `answer` for a human step, which `answer`s when it
/// approves a gate, then by effect: `irreversible`, `reversible`, `read`, and `verdict` for a step
/// that decides without touching anything.
pub fn kind(step: &Step) -> &'static str {
    match (&step.decides, &step.effect) {
        (Decider::Human, _) if step.approves.is_empty() => "ask",
        (Decider::Human, _) => "answer",
        (_, Effect::Irreversible) => "irreversible",
        (_, Effect::Reversible) => "reversible",
        (_, Effect::Read) => "read",
        (_, Effect::None) => "verdict",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMPLEMENT: &str = r#"start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan."
effect = "reversible"
next = { red = "approve-plan", green = "implement" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "implement", rejected = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

    #[test]
    fn answering_the_approval_of_an_irreversible_step_lets_it_run() {
        let workflow: Workflow = toml::from_str(IMPLEMENT).unwrap();

        assert!(answer_lets_irreversible_run(&workflow, "approve-pr"));
    }

    #[test]
    fn undelegable_names_why_a_person_answers() {
        let workflow: Workflow = toml::from_str(IMPLEMENT).unwrap();
        let autonomous: Workflow =
            toml::from_str(&IMPLEMENT.replace("passed = \"approve-pr\"", "passed = \"open-pr\""))
                .unwrap();

        assert_eq!(
            undelegable(&workflow, "approve-plan", true).as_deref(),
            Some("it runs a command")
        );
        assert_eq!(
            undelegable(&workflow, "approve-pr", false).as_deref(),
            Some("step approve-pr approves irreversible step open-pr")
        );
        assert_eq!(
            undelegable(&autonomous, "approve-plan", false).as_deref(),
            Some(
                "answering it can let irreversible step open-pr run before a person decides again"
            )
        );
        assert_eq!(undelegable(&workflow, "approve-plan", false), None);
    }

    #[test]
    fn answering_a_plan_question_waits_for_another_human_decision() {
        let workflow: Workflow = toml::from_str(IMPLEMENT).unwrap();

        assert!(!answer_lets_irreversible_run(&workflow, "approve-plan"));
    }

    #[test]
    fn answering_a_plan_question_before_an_autonomous_step_lets_it_run() {
        let workflow: Workflow =
            toml::from_str(&IMPLEMENT.replace("passed = \"approve-pr\"", "passed = \"open-pr\""))
                .unwrap();

        assert!(answer_lets_irreversible_run(&workflow, "approve-plan"));
    }

    mod generated {
        use super::*;
        use proptest::prelude::*;

        const DECIDERS: [&str; 3] = ["model", "human", "code"];
        const EFFECTS: [&str; 4] = ["none", "read", "reversible", "irreversible"];
        const BY: [&str; 4] = ["code", "model", "human", "asleep"];

        /// One step: its decider, effect, outcomes with their target and `by`, and the steps it
        /// approves, each an index into the workflow's steps or past its end for `stop`.
        type Drawn = (usize, usize, Vec<(usize, Option<usize>)>, Vec<usize>);

        fn workflow(steps: &[Drawn]) -> String {
            let id = |index: usize| match index < steps.len() {
                true => format!("s{index}"),
                false => "stop".to_owned(),
            };
            let mut toml = "start = \"s0\"\n".to_owned();
            for (index, (decides, effect, next_by, approves)) in steps.iter().enumerate() {
                let next: Vec<String> = next_by
                    .iter()
                    .enumerate()
                    .map(|(outcome, (target, _))| format!("o{outcome} = \"{}\"", id(*target)))
                    .collect();
                let by: Vec<String> = next_by
                    .iter()
                    .enumerate()
                    .filter_map(|(outcome, (_, by))| {
                        by.map(|by| format!("o{outcome} = \"{}\"", BY[by]))
                    })
                    .collect();
                let approves: Vec<String> = approves
                    .iter()
                    .filter(|step| **step < steps.len())
                    .map(|step| format!("\"s{step}\""))
                    .collect();
                toml += &format!(
                    "\n[[steps]]\nid = \"s{index}\"\ndecides = \"{}\"\neffect = \"{}\"\nnext = {{ {} }}\nby = {{ {} }}\napproves = [{}]\n",
                    DECIDERS[*decides],
                    EFFECTS[*effect],
                    next.join(", "),
                    by.join(", "),
                    approves.join(", ")
                );
            }
            toml
        }

        fn drawn() -> impl Strategy<Value = Vec<Drawn>> {
            proptest::collection::vec(
                (
                    0..DECIDERS.len(),
                    0..EFFECTS.len(),
                    proptest::collection::vec((0usize..8, proptest::option::of(0..BY.len())), 1..4),
                    proptest::collection::vec(0usize..8, 0..2),
                ),
                1..7,
            )
        }

        /// Whether a person decides before any irreversible step after `at`, read without
        /// `irreversible_step_after`.
        fn guarded(workflow: &Workflow, at: &str) -> bool {
            let Some(step) = workflow.step(at) else {
                return true;
            };
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut pending: Vec<String> = step.next.values().cloned().collect();
            while let Some(id) = pending.pop() {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Some(step) = workflow.step(&id) else {
                    continue;
                };
                if step.effect == Effect::Irreversible {
                    return false;
                }
                for (outcome, target) in &step.next {
                    if step.by(outcome) != By::Human {
                        pending.push(target.clone());
                    }
                }
            }
            true
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]

            #[test]
            fn every_auto_answerable_ask_is_one_a_lead_may_answer(
                steps in drawn(),
                command in any::<bool>(),
                level in proptest::option::of(0..3usize),
                confidence in proptest::option::of(0..3usize),
            ) {
                let workflow: Workflow = toml::from_str(&workflow(&steps)).unwrap();
                let words = [Confidence::Low, Confidence::Medium, Confidence::High];
                let level = level.map(|level| words[level]);
                let confidence = confidence.map(|confidence| words[confidence]);
                for step in &workflow.steps {
                    let at = step.id.as_str();
                    if auto_answerable(&workflow, at, command, level, confidence) {
                        prop_assert!(delegable(&workflow, at, command));
                        prop_assert!(!command && step.approves.is_empty());
                        prop_assert!(guarded(&workflow, at), "{at} can let an irreversible step run");
                        prop_assert!(level != Some(Confidence::Low));
                        prop_assert!(confidence >= level);
                    }
                }
            }
        }
    }
}

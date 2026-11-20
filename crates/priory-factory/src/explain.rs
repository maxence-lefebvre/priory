use crate::factory::{By, Decider, Effect, Step, Workflow};
use std::collections::BTreeSet;

/// An irreversible step and what lets it run: the human steps that approve it and lead to it, or
/// its `autonomous` reason.
#[derive(Debug, PartialEq, Eq)]
pub struct Gate {
    pub step: String,
    pub approved_by: Vec<String>,
    pub autonomous: Option<String>,
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

/// Whether answering the ask of human step `ask` can let an irreversible step run before a
/// person decides again.
pub fn answer_lets_irreversible_run(workflow: &Workflow, ask: &str) -> bool {
    let Some(step) = workflow.step(ask) else {
        return false;
    };
    let mut reached: BTreeSet<&str> = step.next.values().map(String::as_str).collect();
    let mut pending: Vec<&str> = reached.iter().copied().collect();
    while let Some(id) = pending.pop() {
        let Some(step) = workflow.step(id) else {
            continue;
        };
        if step.effect == Effect::Irreversible {
            return true;
        }
        for target in unattended_targets(step) {
            if reached.insert(target) {
                pending.push(target);
            }
        }
    }
    false
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
}

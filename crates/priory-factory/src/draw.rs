use crate::factory::{Decider, Effect, Step, Workflow};
use std::collections::BTreeSet;
use std::fmt::Write;

/// A Mermaid flowchart of `workflow`: shape by decider, a mark on irreversible steps, outcomes on
/// the edges and dotted approval edges.
pub fn mermaid(workflow: &Workflow) -> String {
    let node = |id: &str| match workflow.steps.iter().position(|step| step.id == id) {
        Some(index) => format!("n{index}"),
        None => id.to_owned(),
    };
    let mut out =
        String::from("flowchart TD\n    classDef irreversible stroke:#c00,stroke-width:3px\n");
    for (index, step) in workflow.steps.iter().enumerate() {
        let _ = writeln!(out, "    {}", declaration(index, step));
    }
    let terminals: BTreeSet<&str> = workflow
        .steps
        .iter()
        .flat_map(|step| step.next.values())
        .map(String::as_str)
        .filter(|target| matches!(*target, "done" | "stop"))
        .collect();
    for terminal in terminals {
        let _ = writeln!(out, "    {terminal}(({terminal}))");
    }
    for (index, step) in workflow.steps.iter().enumerate() {
        for (outcome, target) in &step.next {
            let _ = writeln!(out, "    n{index} -->|{}| {}", label(outcome), node(target));
        }
        for approved in &step.approves {
            let _ = writeln!(out, "    n{index} -.->|approves| {}", node(approved));
        }
    }
    out
}

fn declaration(index: usize, step: &Step) -> String {
    let irreversible = step.effect == Effect::Irreversible;
    let text = format!(
        "{}<br/>{}, {}{}",
        label(&step.id),
        step.decides.name(),
        step.effect.name(),
        if irreversible { " ⚠" } else { "" },
    );
    let shape = match step.decides {
        Decider::Model => format!("([\"{text}\"])"),
        Decider::Human => format!("{{{{\"{text}\"}}}}"),
        Decider::Code => format!("[\"{text}\"]"),
    };
    let class = if irreversible { ":::irreversible" } else { "" };
    format!("n{index}{shape}{class}")
}

/// Mermaid reads a bare `"` or `|` inside a label as syntax.
fn label(text: &str) -> String {
    text.replace('"', "#quot;").replace('|', "#124;")
}

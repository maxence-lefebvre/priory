use priory_factory::case::{Answers, Case, GhAnswer};
use priory_factory::factory::{
    Agent, Benchmark, MemoryFrontmatter, Runner, Scorer, Settings, Trigger, View, Workflow,
};
use priory_factory::labels::{Labels, TriageCases};
use priory_factory::lanes::Lanes;
use priory_intake::events::*;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const BASE: &str = "https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/";

fn schema<T: JsonSchema>(settings: SchemaSettings, path: &str) -> (String, Value) {
    let mut schema = settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value();
    schema["$id"] = json!(format!("{BASE}{path}"));
    (path.to_owned(), schema)
}

fn config<T: JsonSchema>(name: &str) -> (String, Value) {
    schema::<T>(SchemaSettings::draft07(), &format!("config/{name}.json"))
}

/// The envelope around `Data`, titled and described by the message type.
fn event<Data: JsonSchema>(kind: &str) -> (String, Value) {
    let settings = SchemaSettings::draft07().for_serialize();
    let (path, mut schema) = schema::<Envelope<Data>>(settings, &format!("events/{kind}.json"));
    let data = schema["properties"]["data"]["$ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let name = data.rsplit('/').next().unwrap();
    schema["description"] = schema["definitions"][name]["description"].clone();
    schema["title"] = json!(kind);
    schema["properties"]["type"]["const"] = json!(kind);
    (path, schema)
}

fn generated() -> Vec<(String, Value)> {
    vec![
        config::<Settings>("factory"),
        config::<Workflow>("workflow"),
        config::<Agent>("agent"),
        config::<Trigger>("trigger"),
        config::<Runner>("runner"),
        config::<Lanes>("lanes"),
        config::<View>("view"),
        config::<Scorer>("scorer"),
        config::<Labels>("judge-cases"),
        config::<TriageCases>("triage-cases"),
        config::<Benchmark>("benchmark"),
        config::<MemoryFrontmatter>("memory"),
        config::<Case>("case"),
        config::<GhAnswer>("case-gh"),
        config::<Answers>("case-answers"),
        schema::<priory_core::sections::Report>(SchemaSettings::draft07(), "report.json"),
        schema::<priory_factory::readiness::Report>(
            SchemaSettings::draft07().for_serialize(),
            "readiness.json",
        ),
        event::<IssueLabeled>("github.issues.labeled"),
        event::<IssueOpened>("github.issues.opened"),
        event::<IssueClosed>("github.issues.closed"),
        event::<IssueComment>("github.issues.comment"),
        event::<PullRequestOpened>("github.pull_request.opened"),
        event::<PullRequestLabeled>("github.pull_request.labeled"),
        event::<PullRequestClosed>("github.pull_request.closed"),
        event::<ReviewSubmitted>("github.pull_request_review.submitted"),
        event::<PullRequestComment>("github.pull_request.comment"),
        event::<ReviewThreadComment>("github.pull_request_review_comment"),
        event::<CheckRunCompleted>("github.check_run.completed"),
        event::<CheckRuns>("github.pull_request.check_runs"),
        event::<ReviewRequested>("github.pull_request.review_requested"),
        event::<ReviewRequestRemoved>("github.pull_request.review_request_removed"),
        event::<ViewChanged>("linear.issue.entered"),
        event::<ViewChanged>("linear.issue.left"),
        event::<ViewFilterChanged>("linear.view.changed"),
        event::<LinearIssueCreated>("linear.issue.created"),
        event::<LinearStateChanged>("linear.issue.state_changed"),
        event::<LinearBlockers>("linear.issue.blockers"),
        event::<LinearIssueUpdated>("linear.issue.updated"),
        event::<LinearCommentCreated>("linear.comment.created"),
        event::<SentryIssueCreated>("sentry.issue.created"),
        event::<FigmaFileCommentCreated>("figma.file.comment_created"),
        event::<CloudwatchAlarmStateChanged>("cloudwatch.alarm.state_changed"),
        event::<HoneycombTriggerFired>("honeycomb.trigger.fired"),
        event::<IntercomConversationCreated>("intercom.conversation.created"),
        event::<SlackPosted>("slack.message.posted"),
        event::<SlackReplied>("slack.thread.replied"),
        event::<ScheduleFired>("schedule.fired"),
        event::<CliRequested>("priory.task.requested.cli"),
        event::<TaskRequested>("factory.task.requested"),
        event::<Baseline>("factory.intake.baseline"),
        event::<IntakeDropped>("factory.intake.dropped"),
        event::<TaskClaimed>("priory.task.claimed"),
        event::<TaskHeld>("priory.task.held"),
        event::<RunnerAlive>("priory.runner.alive"),
        event::<RunGranted>("priory.run.granted"),
        event::<RunRevoked>("priory.run.revoked"),
        event::<DioceseAlive>("priory.diocese.alive"),
        event::<AskAnswered>("priory.ask.answered"),
        event::<AskDismissed>("priory.ask.dismissed"),
        event::<AskPosted>("priory.ask.posted"),
        event::<AskApplied>("priory.ask.applied"),
        event::<TaskActed>("priory.task.acted"),
        event::<TaskApplied>("priory.task.applied"),
        event::<TaskDropped>("priory.task.dropped"),
        event::<MemoryWritten>("priory.memory.written"),
        event::<MemoryAccepted>("priory.memory.accepted"),
        event::<MemoryDropped>("priory.memory.dropped"),
        event::<AutopilotSet>("priory.autopilot.set"),
    ]
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas")
}

#[test]
fn the_committed_schemas_match_the_code() {
    let update = std::env::var_os("PRIORY_UPDATE_SCHEMAS").is_some();
    let stale: Vec<String> = generated()
        .into_iter()
        .filter_map(|(path, schema)| {
            let file = root().join(&path);
            let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
            if fs::read_to_string(&file).ok() == Some(text.clone()) {
                return None;
            }
            if !update {
                return Some(path);
            }
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, text).unwrap();
            None
        })
        .collect();
    assert!(
        stale.is_empty(),
        "docs/schemas differs from the code for {stale:?}: run \
         `PRIORY_UPDATE_SCHEMAS=1 cargo test -p priory --test schemas` and commit the result"
    );
}

#[test]
fn the_intakes_role_may_publish_every_source_type_and_cli_requests_but_no_runner_message() {
    let mut expected: Vec<String> = generated()
        .into_iter()
        .filter_map(|(path, _)| {
            Some(
                path.strip_prefix("events/")?
                    .strip_suffix(".json")?
                    .to_owned(),
            )
        })
        .filter(|kind| !kind.starts_with("priory.") || kind == "priory.task.requested.cli")
        .collect();
    expected.sort();
    let mut allowed: Vec<&str> = priory_intake::log::sql::INTAKE_TYPES.to_vec();
    allowed.sort_unstable();
    assert_eq!(allowed, expected);
}

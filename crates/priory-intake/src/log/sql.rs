//! The Postgres log's SQL that needs no connection: its tables, and the intake's role on them.

/// Creates the tables once per database, whichever process connects first.
pub const SCHEMA: &str = "
    SELECT pg_advisory_xact_lock(hashtext('priory_log_schema'));
    CREATE TABLE IF NOT EXISTS priory_log_messages (
        seq BIGSERIAL PRIMARY KEY,
        output TEXT NOT NULL,
        id TEXT NOT NULL,
        published TIMESTAMPTZ NOT NULL,
        message TEXT NOT NULL,
        UNIQUE (output, id)
    );
    CREATE TABLE IF NOT EXISTS priory_log_cursors (
        consumer TEXT NOT NULL,
        output TEXT NOT NULL,
        seq BIGINT NOT NULL,
        PRIMARY KEY (consumer, output)
    );
";

/// The message types the intake publishes: the source events, the requests its triggers and the
/// Slack shortcut make, and its markers. Never an answer, claim, give, hold or alive message.
pub const INTAKE_TYPES: &[&str] = &[
    "cloudwatch.alarm.state_changed",
    "factory.intake.baseline",
    "factory.intake.dropped",
    "factory.task.requested",
    "figma.file.comment_created",
    "github.check_run.completed",
    "github.issues.closed",
    "github.issues.comment",
    "github.issues.labeled",
    "github.issues.opened",
    "github.pull_request.check_runs",
    "github.pull_request.closed",
    "github.pull_request.comment",
    "github.pull_request.labeled",
    "github.pull_request.opened",
    "github.pull_request_review_comment",
    "github.pull_request.review_requested",
    "github.pull_request.review_request_removed",
    "github.pull_request_review.submitted",
    "honeycomb.trigger.fired",
    "intercom.conversation.created",
    "linear.comment.created",
    "linear.issue.blockers",
    "linear.issue.created",
    "linear.issue.entered",
    "linear.issue.left",
    "linear.issue.state_changed",
    "linear.issue.updated",
    "linear.view.changed",
    "priory.task.requested.cli",
    "schedule.fired",
    "sentry.issue.created",
    "slack.message.posted",
    "slack.thread.replied",
];

/// The consumer whose cursor the intake's role may move.
pub const INTAKE_CONSUMER: &str = "priory.intake";

/// The one message type the diocese's gateway publishes: a cloud run's tool call.
pub const GATEWAY_TYPES: &[&str] = &["priory.run.called"];

/// The consumer whose cursor the gateway's role may move.
pub const GATEWAY_CONSUMER: &str = "priory.gateway";

/// The SQL that creates `role` for the intake, as `restricted_role` says.
pub fn intake_role(role: &str) -> Result<String, String> {
    restricted_role(role, INTAKE_TYPES, INTAKE_CONSUMER)
}

/// The SQL that creates `role` for the diocese's gateway, as `restricted_role` says.
pub fn gateway_role(role: &str) -> Result<String, String> {
    restricted_role(role, GATEWAY_TYPES, GATEWAY_CONSUMER)
}

/// The SQL an admin runs once on the log's database, in the schema the log's tables live in, to
/// create `role`: it reads every message, publishes only `types`, and moves only `consumer`'s
/// cursor. Every other role keeps what it had. Running it again changes nothing. It sets no
/// password.
fn restricted_role(role: &str, types: &[&str], consumer: &str) -> Result<String, String> {
    let valid = role.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && role
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!(
            "{role:?} is no role name: use lowercase letters, digits and underscores"
        ));
    }
    let types = types
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "BEGIN;
{SCHEMA}
DO $$ BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{role}') THEN
        CREATE ROLE {role} LOGIN;
    END IF;
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO {role}', current_schema());
END $$;
GRANT SELECT, INSERT ON priory_log_messages TO {role};
GRANT USAGE ON SEQUENCE priory_log_messages_seq_seq TO {role};
GRANT SELECT, INSERT, UPDATE ON priory_log_cursors TO {role};
ALTER TABLE priory_log_messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE priory_log_cursors ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS priory_log_anyone ON priory_log_messages;
CREATE POLICY priory_log_anyone ON priory_log_messages USING (true) WITH CHECK (true);
DROP POLICY IF EXISTS priory_log_anyone ON priory_log_cursors;
CREATE POLICY priory_log_anyone ON priory_log_cursors USING (true) WITH CHECK (true);
DROP POLICY IF EXISTS {role}_publishes ON priory_log_messages;
CREATE POLICY {role}_publishes ON priory_log_messages AS RESTRICTIVE FOR INSERT TO {role}
    WITH CHECK ((message::jsonb ->> 'type') = ANY (ARRAY[{types}]));
DROP POLICY IF EXISTS {role}_adds_cursor ON priory_log_cursors;
CREATE POLICY {role}_adds_cursor ON priory_log_cursors AS RESTRICTIVE FOR INSERT TO {role}
    WITH CHECK (consumer = '{consumer}');
DROP POLICY IF EXISTS {role}_moves_cursor ON priory_log_cursors;
CREATE POLICY {role}_moves_cursor ON priory_log_cursors AS RESTRICTIVE FOR UPDATE TO {role}
    USING (consumer = '{consumer}') WITH CHECK (consumer = '{consumer}');
COMMIT;
"
    ))
}

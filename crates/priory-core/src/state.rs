use crate::room::{self, Share};
use crate::route::{self, Decision};
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::explain;
use priory_factory::factory::{Factory, Fair, Loaded, Session, Workflow};
use priory_factory::lanes::{Lanes, Subject};
use priory_intake::intake::{CHECKED, ISSUE_CLOSED, PR_CLOSED as CLOSED};
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

const VERSION: i64 = 113;

const SCHEMA: &str = "
CREATE TABLE tasks (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  key      TEXT NOT NULL,
  workflow TEXT NOT NULL,
  step     TEXT NOT NULL,
  state    TEXT NOT NULL,
  band     TEXT,
  lane     TEXT,
  trigger  TEXT NOT NULL,
  source   TEXT NOT NULL
);
CREATE UNIQUE INDEX one_active_task_per_key ON tasks (key)
  WHERE state NOT IN ('done', 'failed', 'dropped');
CREATE TABLE inbox (
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  id      TEXT NOT NULL UNIQUE,
  type    TEXT NOT NULL,
  subject TEXT,
  time    TEXT NOT NULL,
  data    TEXT NOT NULL,
  links   TEXT NOT NULL,
  task    INTEGER REFERENCES tasks (id),
  outcome TEXT NOT NULL
);
PRAGMA user_version = 1;
";

const SCHEMA_2: &str = "
CREATE TABLE runs (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  step     TEXT NOT NULL,
  tmux     TEXT NOT NULL,
  session  TEXT NOT NULL,
  idle     INTEGER NOT NULL DEFAULT 0,
  ready    INTEGER NOT NULL DEFAULT 0,
  started  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended    TEXT,
  ended_as TEXT
);
CREATE UNIQUE INDEX one_live_run_per_task ON runs (task) WHERE ended IS NULL;
CREATE TABLE outbox (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  task      INTEGER NOT NULL REFERENCES tasks (id),
  run       INTEGER REFERENCES runs (id),
  text      TEXT NOT NULL,
  state     TEXT NOT NULL DEFAULT 'queued',
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  attempted TEXT,
  confirmed TEXT
);
ALTER TABLE tasks ADD COLUMN branch TEXT;
ALTER TABLE tasks ADD COLUMN worktree TEXT;
ALTER TABLE tasks ADD COLUMN reason TEXT;
ALTER TABLE inbox ADD COLUMN outbox INTEGER REFERENCES outbox (id);
PRAGMA user_version = 2;
";

const SCHEMA_3: &str = "
CREATE TABLE results (
  run        INTEGER PRIMARY KEY REFERENCES runs (id),
  task       INTEGER NOT NULL REFERENCES tasks (id),
  step       TEXT NOT NULL,
  outcome    TEXT NOT NULL,
  summary    TEXT NOT NULL,
  evidence   TEXT,
  commit_sha TEXT,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TRIGGER results_are_immutable BEFORE UPDATE ON results
  BEGIN SELECT RAISE(ABORT, 'results are immutable'); END;
CREATE TRIGGER results_stay BEFORE DELETE ON results
  BEGIN SELECT RAISE(ABORT, 'results are immutable'); END;
CREATE TABLE asks (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  task           INTEGER NOT NULL REFERENCES tasks (id),
  run            INTEGER REFERENCES runs (id),
  step           TEXT NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('step', 'agent')),
  revision       INTEGER NOT NULL DEFAULT 1,
  question       TEXT NOT NULL,
  options        TEXT NOT NULL,
  recommendation TEXT,
  tradeoff       TEXT,
  answer         TEXT,
  answered       TEXT,
  created        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 3;
";

const SCHEMA_4: &str = "
CREATE TABLE owns (
  task     INTEGER NOT NULL REFERENCES tasks (id),
  resource TEXT NOT NULL,
  PRIMARY KEY (task, resource)
);
CREATE INDEX owns_by_resource ON owns (resource);
INSERT INTO owns (task, resource) SELECT id, key FROM tasks;
CREATE TABLE waits (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  run      INTEGER NOT NULL REFERENCES runs (id),
  kind     TEXT NOT NULL CHECK (kind IN ('pull_request', 'checks', 'review', 'time', 'task')),
  resource TEXT,
  until    TEXT,
  target   INTEGER REFERENCES tasks (id),
  head_sha TEXT,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  fired    TEXT,
  inbox    INTEGER REFERENCES inbox (seq)
);
PRAGMA user_version = 4;
";

const SCHEMA_5: &str = "
ALTER TABLE runs ADD COLUMN agent TEXT;
PRAGMA user_version = 5;
";

const SCHEMA_6: &str = "
ALTER TABLE waits ADD COLUMN daemon INTEGER NOT NULL DEFAULT 0;
ALTER TABLE results ADD COLUMN inbox INTEGER REFERENCES inbox (seq);
PRAGMA user_version = 6;
";

const SCHEMA_7: &str = "
ALTER TABLE runs ADD COLUMN turn TEXT;
ALTER TABLE runs ADD COLUMN prompted TEXT;
ALTER TABLE tasks ADD COLUMN changed TEXT;
UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
CREATE TRIGGER task_changed AFTER UPDATE OF state ON tasks WHEN NEW.state IS NOT OLD.state
  BEGIN UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = NEW.id; END;
PRAGMA user_version = 7;
";

const SCHEMA_8: &str = "
CREATE TABLE approvals (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task       INTEGER NOT NULL REFERENCES tasks (id),
  step       TEXT NOT NULL,
  ask        INTEGER NOT NULL REFERENCES asks (id),
  revision   INTEGER NOT NULL,
  commit_sha TEXT,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE actions (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  task         INTEGER NOT NULL REFERENCES tasks (id),
  step         TEXT NOT NULL,
  action       TEXT NOT NULL,
  status       TEXT NOT NULL,
  pull_request TEXT,
  commit_sha   TEXT,
  error        TEXT,
  result       INTEGER REFERENCES results (run),
  inbox        INTEGER REFERENCES inbox (seq),
  created      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 8;
";

const SCHEMA_9: &str = "
CREATE TABLE shared_sessions (
  agent   TEXT PRIMARY KEY,
  session TEXT NOT NULL
);
CREATE UNIQUE INDEX one_live_run_per_session ON runs (session) WHERE ended IS NULL;
PRAGMA user_version = 9;
";

const SCHEMA_10: &str = "
ALTER TABLE tasks ADD COLUMN ending TEXT;
PRAGMA user_version = 10;
";

const SCHEMA_11: &str = "
CREATE TABLE trace (
  seq   INTEGER PRIMARY KEY AUTOINCREMENT,
  id    TEXT NOT NULL UNIQUE,
  type  TEXT NOT NULL,
  time  TEXT NOT NULL,
  event TEXT NOT NULL
);
ALTER TABLE tasks ADD COLUMN entered TEXT;
UPDATE tasks SET entered = changed;
ALTER TABLE tasks ADD COLUMN tokens TEXT;
ALTER TABLE tasks ADD COLUMN model TEXT;
ALTER TABLE runs ADD COLUMN transcript TEXT;
ALTER TABLE runs ADD COLUMN read INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 11;
";

const SCHEMA_12: &str = "
ALTER TABLE tasks ADD COLUMN position INTEGER;
CREATE TABLE bands (
  name TEXT PRIMARY KEY,
  rank INTEGER NOT NULL
);
PRAGMA user_version = 12;
";

const SCHEMA_13: &str = "
ALTER TABLE tasks ADD COLUMN team TEXT;
ALTER TABLE tasks ADD COLUMN project TEXT;
ALTER TABLE tasks ADD COLUMN lane_derived INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 13;
";

const SCHEMA_14: &str = "
ALTER TABLE runs ADD COLUMN thread TEXT;
PRAGMA user_version = 14;
";

const SCHEMA_15: &str = "
ALTER TABLE outbox ADD COLUMN nudge INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN nudgeable INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 15;
";

const SCHEMA_16: &str = "
ALTER TABLE tasks ADD COLUMN generation INTEGER NOT NULL DEFAULT 1;
ALTER TABLE asks ADD COLUMN withdrawn INTEGER;
ALTER TABLE waits ADD COLUMN withdrawn INTEGER;
PRAGMA user_version = 16;
";

const SCHEMA_17: &str = "
CREATE TABLE holds (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('person', 'time', 'task', 'merge', 'blockers')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  released    TEXT,
  released_as TEXT
);
CREATE UNIQUE INDEX one_open_hold_per_task ON holds (task) WHERE released IS NULL;
PRAGMA user_version = 17;
";

const SCHEMA_18: &str = "
CREATE TABLE grants (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  bytes   INTEGER NOT NULL,
  pid     INTEGER NOT NULL,
  started INTEGER NOT NULL,
  task    INTEGER REFERENCES tasks (id),
  command TEXT NOT NULL,
  state   TEXT NOT NULL DEFAULT 'waiting' CHECK (state IN ('waiting', 'held')),
  created TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  granted TEXT
);
PRAGMA user_version = 18;
";

const SCHEMA_19: &str = "
CREATE TABLE stalls (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  resource TEXT NOT NULL,
  missing  TEXT NOT NULL,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  since    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  scripts  TEXT,
  swept    TEXT,
  sweeper  INTEGER REFERENCES tasks (id),
  ended    TEXT
);
CREATE UNIQUE INDEX one_open_stall ON stalls ((ended IS NULL)) WHERE ended IS NULL;
CREATE TABLE sweeps (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  stall   INTEGER NOT NULL REFERENCES stalls (id),
  script  TEXT NOT NULL,
  started TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended   TEXT,
  status  TEXT,
  output  TEXT,
  pid     INTEGER,
  pid_started INTEGER
);
PRAGMA user_version = 19;
";

const SCHEMA_20: &str = "
CREATE TABLE waits_20 (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  task      INTEGER NOT NULL REFERENCES tasks (id),
  run       INTEGER NOT NULL REFERENCES runs (id),
  kind      TEXT NOT NULL CHECK (kind IN ('pull_request', 'checks', 'review', 'time', 'task',
              'linear_comment', 'linear_state', 'slack_reply')),
  resource  TEXT,
  until     TEXT,
  target    INTEGER REFERENCES tasks (id),
  head_sha  TEXT,
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  fired     TEXT,
  inbox     INTEGER REFERENCES inbox (seq),
  daemon    INTEGER NOT NULL DEFAULT 0,
  withdrawn INTEGER,
  state     TEXT,
  since     TEXT
);
INSERT INTO waits_20 (id, task, run, kind, resource, until, target, head_sha, created, fired, inbox,
  daemon, withdrawn)
  SELECT id, task, run, kind, resource, until, target, head_sha, created, fired, inbox, daemon,
  withdrawn FROM waits;
DROP TABLE waits;
ALTER TABLE waits_20 RENAME TO waits;
PRAGMA user_version = 20;
";

const SCHEMA_21: &str = "
ALTER TABLE tasks ADD COLUMN retired TEXT;
CREATE TABLE waits_21 (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  task      INTEGER NOT NULL REFERENCES tasks (id),
  run       INTEGER NOT NULL REFERENCES runs (id),
  kind      TEXT NOT NULL CHECK (kind IN ('pull_request', 'checks', 'review', 'time', 'task',
              'linear_comment', 'linear_state', 'slack_reply', 'disk')),
  resource  TEXT,
  until     TEXT,
  target    INTEGER REFERENCES tasks (id),
  head_sha  TEXT,
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  fired     TEXT,
  inbox     INTEGER REFERENCES inbox (seq),
  daemon    INTEGER NOT NULL DEFAULT 0,
  withdrawn INTEGER,
  state     TEXT,
  since     TEXT,
  bytes     INTEGER
);
INSERT INTO waits_21 (id, task, run, kind, resource, until, target, head_sha, created, fired, inbox,
  daemon, withdrawn, state, since)
  SELECT id, task, run, kind, resource, until, target, head_sha, created, fired, inbox, daemon,
  withdrawn, state, since FROM waits;
DROP TABLE waits;
ALTER TABLE waits_21 RENAME TO waits;
PRAGMA user_version = 21;
";

const SCHEMA_22: &str = "
ALTER TABLE runs ADD COLUMN signaled TEXT;
CREATE TABLE handlers (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  run         INTEGER NOT NULL REFERENCES runs (id),
  called      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  screen      TEXT NOT NULL,
  pid         INTEGER,
  pid_started INTEGER,
  answered    TEXT,
  action      TEXT CHECK (action IN ('nudge', 'restart', 'leave')),
  text        TEXT
);
PRAGMA user_version = 22;
";

// A run carried into the next step now takes over its predecessor's unconfirmed prompt.
const SCHEMA_23: &str = "
UPDATE outbox SET run = (
  SELECT heir.id FROM runs AS heir JOIN runs AS typed ON typed.id = outbox.run
  WHERE heir.task = typed.task AND heir.ended IS NULL AND heir.tmux = typed.tmux
  AND heir.session = typed.session AND NOT EXISTS (
    SELECT 1 FROM runs AS mid WHERE mid.task = typed.task
    AND mid.id > typed.id AND mid.id < heir.id AND mid.ended_as IS NOT 'moved'))
WHERE state IN ('attempted', 'uncertain')
AND (SELECT ended_as FROM runs WHERE id = outbox.run) = 'moved'
AND EXISTS (
  SELECT 1 FROM runs AS heir JOIN runs AS typed ON typed.id = outbox.run
  WHERE heir.task = typed.task AND heir.ended IS NULL AND heir.tmux = typed.tmux
  AND heir.session = typed.session AND NOT EXISTS (
    SELECT 1 FROM runs AS mid WHERE mid.task = typed.task
    AND mid.id > typed.id AND mid.id < heir.id AND mid.ended_as IS NOT 'moved'));
PRAGMA user_version = 23;
";

const SCHEMA_24: &str = "
ALTER TABLE tasks ADD COLUMN repo TEXT;
UPDATE tasks SET repo = (SELECT json_extract(inbox.data, '$.repo') FROM inbox
  WHERE inbox.id = tasks.source AND inbox.type = 'priory.task.requested.cli')
  WHERE trigger = 'cli';
PRAGMA user_version = 24;
";

// A task queued on schema 10 had no `changed`, so migration 11 left its `entered` NULL.
const SCHEMA_25: &str = "
UPDATE tasks SET entered = coalesce(changed, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) WHERE entered IS NULL;
PRAGMA user_version = 25;
";

// A `/compact` the compact tool queued, which the next compaction confirms.
const SCHEMA_26: &str = "
ALTER TABLE outbox ADD COLUMN compact INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 26;
";

// How far each OTLP endpoint has received the trace, by the last `trace.seq` it accepted.
const SCHEMA_27: &str = "
CREATE TABLE IF NOT EXISTS exports (
  endpoint TEXT PRIMARY KEY,
  seq      INTEGER NOT NULL
);
PRAGMA user_version = 27;
";

// Search: the TL;DR, its keyword index, summarizer calls and embeddings.
const SCHEMA_28: &str = "
ALTER TABLE tasks ADD COLUMN tldr TEXT;
CREATE VIEW task_text AS SELECT tasks.id AS task,
  coalesce(json_extract(inbox.data, '$.title'), '') AS title,
  coalesce(json_extract(inbox.data, '$.text'), json_extract(inbox.data, '$.body'),
    json_extract(inbox.data, '$.description'), json_extract(inbox.data, '$.prompt'), '') AS request,
  coalesce(tasks.tldr, '') AS tldr,
  coalesce((SELECT group_concat(summary || ' ' || coalesce(evidence, ''), ' ') FROM results
    WHERE results.task = tasks.id), '') AS results
  FROM tasks LEFT JOIN inbox ON inbox.id = tasks.source;
CREATE VIRTUAL TABLE search USING fts5 (title, request, tldr, results, tokenize = 'porter unicode61');
INSERT INTO search (rowid, title, request, tldr, results) SELECT * FROM task_text;
CREATE TRIGGER search_task_created AFTER INSERT ON tasks BEGIN
  INSERT INTO search (rowid, title, request, tldr, results) SELECT * FROM task_text WHERE task = NEW.id;
END;
CREATE TRIGGER search_tldr_changed AFTER UPDATE OF tldr ON tasks BEGIN
  DELETE FROM search WHERE rowid = NEW.id;
  INSERT INTO search (rowid, title, request, tldr, results) SELECT * FROM task_text WHERE task = NEW.id;
END;
CREATE TRIGGER search_result_stored AFTER INSERT ON results BEGIN
  DELETE FROM search WHERE rowid = NEW.task;
  INSERT INTO search (rowid, title, request, tldr, results) SELECT * FROM task_text WHERE task = NEW.task;
END;
CREATE TABLE summaries (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  called      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  pid         INTEGER,
  pid_started INTEGER,
  answered    TEXT,
  status      TEXT
);
INSERT INTO summaries (task, answered, status)
  SELECT id, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'created before search' FROM tasks;
CREATE TABLE embedder (
  id    INTEGER PRIMARY KEY CHECK (id = 1),
  name  TEXT NOT NULL,
  error TEXT
);
CREATE TABLE vectors (
  task     INTEGER PRIMARY KEY REFERENCES tasks (id),
  embedder TEXT NOT NULL,
  text     TEXT NOT NULL,
  vector   BLOB NOT NULL
);
PRAGMA user_version = 28;
";

// A task's grooming grade, set by the latest model step that ended red, yellow or green.
const SCHEMA_29: &str = "
ALTER TABLE results ADD COLUMN reason TEXT;
ALTER TABLE tasks ADD COLUMN grade TEXT;
ALTER TABLE tasks ADD COLUMN grade_reason TEXT;
PRAGMA user_version = 29;
";

const SCHEMA_30: &str = "
CREATE TABLE snoozes (
  kind        TEXT NOT NULL CHECK (kind IN ('ask', 'task', 'prompt')),
  item        INTEGER NOT NULL,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  snoozed     TEXT NOT NULL,
  until       TEXT NOT NULL,
  back        TEXT,
  back_reason TEXT,
  PRIMARY KEY (kind, item)
);
CREATE TRIGGER snooze_ends_on_a_new_revision AFTER UPDATE OF revision ON asks
  WHEN NEW.revision IS NOT OLD.revision BEGIN
  UPDATE snoozes SET back = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), back_reason = 'new revision'
  WHERE kind = 'ask' AND item = NEW.id AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
END;
CREATE TRIGGER snooze_ends_on_a_task_state_change AFTER UPDATE OF state ON tasks
  WHEN NEW.state IS NOT OLD.state BEGIN
  UPDATE snoozes SET back = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    back_reason = CASE WHEN NEW.state IN ('blocked', 'failed') THEN 'new failure'
      ELSE 'task state changed' END
  WHERE task = NEW.id AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    AND (kind != 'ask' OR back IS NULL AND item IN (SELECT id FROM asks WHERE task = NEW.id
      AND answer IS NULL AND withdrawn IS NULL));
END;
PRAGMA user_version = 30;
";

// When a person read a posted report; a read report leaves the desk.
const SCHEMA_31: &str = "
ALTER TABLE inbox ADD COLUMN read TEXT;
PRAGMA user_version = 31;
";

// A person's pause: while its one row exists, no queued task starts a run. A prompt that `say`
// queued keeps its run from sleeping, so it is never stranded behind an ask or a wait.
const SCHEMA_32: &str = "
CREATE TABLE pause (
  id     INTEGER PRIMARY KEY CHECK (id = 1),
  reason TEXT,
  since  TEXT NOT NULL
);
ALTER TABLE outbox ADD COLUMN wakes INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 32;
";

// An outside ask belongs to no task: the rebuild lets `task` and `step` be NULL. `open` runs it
// with foreign keys off, so the approvals and holds that name an ask keep pointing at `asks`. The
// legacy rename skips checking the snooze trigger on `tasks`, which names `asks` while it is gone.
const SCHEMA_33: &str = "
CREATE TABLE asks_33 (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  task           INTEGER REFERENCES tasks (id),
  run            INTEGER REFERENCES runs (id),
  step           TEXT,
  kind           TEXT NOT NULL CHECK (kind IN ('step', 'agent', 'outside')),
  revision       INTEGER NOT NULL DEFAULT 1,
  question       TEXT NOT NULL,
  options        TEXT NOT NULL,
  recommendation TEXT,
  tradeoff       TEXT,
  answer         TEXT,
  answered       TEXT,
  created        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  withdrawn      INTEGER,
  name           TEXT UNIQUE,
  title          TEXT,
  text           TEXT,
  what_waits     TEXT,
  next_action    TEXT,
  command        TEXT,
  coordinator    TEXT,
  key            TEXT,
  run_id         TEXT,
  session        TEXT,
  CHECK ((kind = 'outside') = (task IS NULL))
);
INSERT INTO asks_33 (id, task, run, step, kind, revision, question, options, recommendation,
  tradeoff, answer, answered, created, withdrawn)
  SELECT id, task, run, step, kind, revision, question, options, recommendation, tradeoff, answer,
  answered, created, withdrawn FROM asks;
DROP TABLE asks;
PRAGMA legacy_alter_table = ON;
ALTER TABLE asks_33 RENAME TO asks;
PRAGMA legacy_alter_table = OFF;
CREATE TRIGGER snooze_ends_on_a_new_revision AFTER UPDATE OF revision ON asks
  WHEN NEW.revision IS NOT OLD.revision BEGIN
  UPDATE snoozes SET back = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), back_reason = 'new revision'
  WHERE kind = 'ask' AND item = NEW.id AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
END;
PRAGMA user_version = 33;
";

const SCHEMA_34: &str = "
CREATE TABLE reports (
  task     INTEGER PRIMARY KEY REFERENCES tasks (id),
  revision INTEGER NOT NULL,
  report   TEXT NOT NULL,
  put      TEXT NOT NULL
);
PRAGMA user_version = 34;
";

/// Three triggers per table that bump the revision in the same transaction as each row change.
macro_rules! bumps {
    ($($table:literal),*) => {
        concat!($(
            "CREATE TRIGGER IF NOT EXISTS revision_", $table, "_insert AFTER INSERT ON ", $table,
            " BEGIN UPDATE revision SET value = value + 1; END;\n",
            "CREATE TRIGGER IF NOT EXISTS revision_", $table, "_update AFTER UPDATE ON ", $table,
            " BEGIN UPDATE revision SET value = value + 1; END;\n",
            "CREATE TRIGGER IF NOT EXISTS revision_", $table, "_delete AFTER DELETE ON ", $table,
            " BEGIN UPDATE revision SET value = value + 1; END;\n",
        )*)
    };
}

// The state's revision: every committed change to a table a listing reads bumps it, so a reader
// can wait for a change. A migration that rebuilds one of these tables recreates its triggers.
// It is idempotent, so a file set back to an older version, as the upgrade tests do, migrates again.
const SCHEMA_35: &str = concat!(
    "CREATE TABLE IF NOT EXISTS revision (
  id    INTEGER PRIMARY KEY CHECK (id = 1),
  value INTEGER NOT NULL
);
INSERT OR IGNORE INTO revision VALUES (1, 0);
",
    bumps!(
        "tasks", "asks", "runs", "waits", "holds", "inbox", "outbox", "results", "actions",
        "snoozes", "sweeps", "stalls", "grants", "pause", "bands", "reports"
    ),
    "PRAGMA user_version = 35;
"
);

const SCHEMA_36: &str = concat!(
    "CREATE TABLE IF NOT EXISTS deliveries (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ask         INTEGER NOT NULL REFERENCES asks (id),
  revision    INTEGER NOT NULL,
  session     TEXT NOT NULL,
  answer      TEXT,
  state       TEXT NOT NULL DEFAULT 'queued'
                CHECK (state IN ('queued', 'running', 'delivered', 'failed')),
  tries       INTEGER NOT NULL DEFAULT 0,
  due         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  error       TEXT,
  attempted   TEXT,
  pid         INTEGER,
  pid_started INTEGER,
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended       TEXT
);
",
    bumps!("deliveries"),
    "PRAGMA user_version = 36;
"
);

// A task's progress as its agent last published it: a JSON object, merged field by field.
const SCHEMA_37: &str = "
ALTER TABLE tasks ADD COLUMN progress TEXT;
PRAGMA user_version = 37;
";

// Each person's last visit to the desk, which its Since you looked counts from.
const SCHEMA_38: &str = concat!(
    "CREATE TABLE IF NOT EXISTS visits (
  person TEXT PRIMARY KEY,
  seen   TEXT NOT NULL
);
",
    bumps!("visits", "trace"),
    "PRAGMA user_version = 38;
"
);

// A `run` action is stored before its command starts, with its process group, and `ended` once it
// ends, so a restarted daemon finds the unended ones; it keeps the tail of the command's output.
const SCHEMA_39: &str = "ALTER TABLE actions ADD COLUMN stdout TEXT;
ALTER TABLE actions ADD COLUMN stderr TEXT;
ALTER TABLE actions ADD COLUMN pid INTEGER;
ALTER TABLE actions ADD COLUMN pid_started INTEGER;
ALTER TABLE actions ADD COLUMN ended TEXT;
PRAGMA user_version = 39;
";

// A hold takes several conditions, one row each, and an issue condition. Each row is met on its
// own; the hold's rows are released together once all of them are met.
const SCHEMA_40: &str = concat!(
    "CREATE TABLE holds_40 (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('person', 'time', 'task', 'merge', 'blockers', 'issue')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  met         TEXT,
  met_as      TEXT,
  released    TEXT,
  released_as TEXT
);
INSERT INTO holds_40 (id, task, reason, kind, until, target, resource, if_closed, resume, ask,
  created, met, met_as, released, released_as)
  SELECT id, task, reason, kind, until, target, resource, if_closed, resume, ask, created,
  released, released_as, released, released_as FROM holds;
DROP TABLE holds;
ALTER TABLE holds_40 RENAME TO holds;
CREATE INDEX open_holds ON holds (task) WHERE released IS NULL;
",
    bumps!("holds"),
    "PRAGMA user_version = 40;
"
);

// An answer that runs its ask's command stores it as an `answer` action, with the ask, the revision
// it answered, the command it ran and the task's generation, before the command starts.
const SCHEMA_41: &str = "ALTER TABLE actions ADD COLUMN ask INTEGER REFERENCES asks (id);
ALTER TABLE actions ADD COLUMN revision INTEGER;
ALTER TABLE actions ADD COLUMN command TEXT;
ALTER TABLE actions ADD COLUMN generation INTEGER;
PRAGMA user_version = 41;
";

// A report pick's request, stored with the choice and deleted once it is published.
const SCHEMA_42: &str = "CREATE TABLE dispatches (
  pick TEXT PRIMARY KEY,
  request TEXT NOT NULL
);
PRAGMA user_version = 42;
";

// Every revision of an ask as it was asked and answered, and every answer the daemon refused.
// Triggers record each revision and its answer, so no writer of `asks` can skip one. An ask stored
// before keeps only its current revision, dated when the ask was first asked.
const SCHEMA_43: &str = concat!(
    "CREATE TABLE IF NOT EXISTS ask_revisions (
  ask            INTEGER NOT NULL REFERENCES asks (id),
  revision       INTEGER NOT NULL,
  question       TEXT NOT NULL,
  options        TEXT NOT NULL,
  recommendation TEXT,
  text           TEXT,
  created        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  answer         TEXT,
  answered       TEXT,
  PRIMARY KEY (ask, revision)
);
INSERT OR IGNORE INTO ask_revisions (ask, revision, question, options, recommendation, text, created,
  answer, answered)
  SELECT id, revision, question, options, recommendation, text, created, answer, answered FROM asks;
CREATE TRIGGER IF NOT EXISTS ask_revision_asked AFTER INSERT ON asks BEGIN
  INSERT INTO ask_revisions (ask, revision, question, options, recommendation, text)
  VALUES (NEW.id, NEW.revision, NEW.question, NEW.options, NEW.recommendation, NEW.text);
END;
CREATE TRIGGER IF NOT EXISTS ask_revision_revised AFTER UPDATE OF revision ON asks
  WHEN NEW.revision IS NOT OLD.revision BEGIN
  INSERT OR REPLACE INTO ask_revisions (ask, revision, question, options, recommendation, text)
  VALUES (NEW.id, NEW.revision, NEW.question, NEW.options, NEW.recommendation, NEW.text);
END;
CREATE TRIGGER IF NOT EXISTS ask_revision_answered AFTER UPDATE OF answer ON asks
  WHEN NEW.answer IS NOT NULL AND NEW.answer IS NOT OLD.answer BEGIN
  UPDATE ask_revisions SET answer = NEW.answer, answered = NEW.answered
  WHERE ask = NEW.id AND revision = NEW.revision;
END;
CREATE TABLE IF NOT EXISTS ask_refusals (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  ask      INTEGER NOT NULL REFERENCES asks (id),
  revision INTEGER NOT NULL,
  answer   TEXT NOT NULL,
  by       TEXT NOT NULL,
  why      TEXT NOT NULL,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
",
    bumps!("ask_refusals"),
    "PRAGMA user_version = 43;
"
);

// The human steps whose answer lets an irreversible step run, as the daemon last recorded them.
const SCHEMA_44: &str = concat!(
    "CREATE TABLE IF NOT EXISTS irreversible_asks (
  workflow TEXT NOT NULL,
  step     TEXT NOT NULL,
  PRIMARY KEY (workflow, step)
);
",
    bumps!("irreversible_asks"),
    "PRAGMA user_version = 44;
"
);

// Each intake source's last poll and, while it fails, its error and the report that tells it. A
// poll writes `polled` every minute, so only a new source or a changed error bumps the revision.
const SCHEMA_45: &str = "CREATE TABLE IF NOT EXISTS intake_sources (
  source        TEXT PRIMARY KEY,
  polled        TEXT NOT NULL,
  error         TEXT,
  failing_since TEXT,
  report        INTEGER
);
CREATE TRIGGER IF NOT EXISTS revision_intake_sources_insert AFTER INSERT ON intake_sources
  BEGIN UPDATE revision SET value = value + 1; END;
CREATE TRIGGER IF NOT EXISTS revision_intake_sources_error AFTER UPDATE OF error ON intake_sources
  WHEN NEW.error IS NOT OLD.error BEGIN UPDATE revision SET value = value + 1; END;
CREATE TRIGGER IF NOT EXISTS revision_intake_sources_delete AFTER DELETE ON intake_sources
  BEGIN UPDATE revision SET value = value + 1; END;
PRAGMA user_version = 45;
";

// Comments on a report's rows, by the row's id, each a thread or a reply in one.
const SCHEMA_46: &str = "CREATE TABLE IF NOT EXISTS row_comments (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  row      TEXT NOT NULL,
  reply_to INTEGER REFERENCES row_comments (id),
  author   TEXT NOT NULL,
  text     TEXT NOT NULL,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  resolved INTEGER NOT NULL DEFAULT 0
);
PRAGMA user_version = 46;
";

// The resources a task subscribes to, beside its ownership in `owns`: polled and woken on until
// the resource closes, the task retires or unsubscribes. `owned` is what a task owns either way.
const SCHEMA_47: &str = concat!(
    "CREATE TABLE IF NOT EXISTS subscriptions (
  task     INTEGER NOT NULL REFERENCES tasks (id),
  resource TEXT NOT NULL,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  PRIMARY KEY (task, resource)
);
CREATE INDEX IF NOT EXISTS subscriptions_by_resource ON subscriptions (resource);
CREATE VIEW IF NOT EXISTS owned AS
  SELECT task, resource FROM owns UNION SELECT task, resource FROM subscriptions;
",
    bumps!("subscriptions"),
    "PRAGMA user_version = 47;
"
);

// The browsers that get Web Push notifications, and the notifications the daemon has yet to send.
// Triggers queue a new ask, an ask asked again, and a task that turns blocked or failed, only while
// a browser subscribes.
const SCHEMA_48: &str = "CREATE TABLE IF NOT EXISTS push_subscriptions (
  endpoint TEXT PRIMARY KEY,
  p256dh   TEXT NOT NULL,
  auth     TEXT NOT NULL,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE IF NOT EXISTS push_queue (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  kind    TEXT NOT NULL CHECK (kind IN ('ask', 'alarm')),
  item    INTEGER NOT NULL,
  revision INTEGER,
  state   TEXT,
  created TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS push_a_new_ask AFTER INSERT ON asks
  WHEN NEW.answer IS NULL AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, revision) VALUES ('ask', NEW.id, NEW.revision);
END;
CREATE TRIGGER IF NOT EXISTS push_an_ask_asked_again AFTER UPDATE OF revision ON asks
  WHEN NEW.revision IS NOT OLD.revision AND NEW.answer IS NULL AND NEW.withdrawn IS NULL
  AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, revision) VALUES ('ask', NEW.id, NEW.revision);
END;
CREATE TRIGGER IF NOT EXISTS push_an_alarm AFTER UPDATE OF state ON tasks
  WHEN NEW.state IN ('blocked', 'failed') AND NEW.state IS NOT OLD.state
  AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, state) VALUES ('alarm', NEW.id, NEW.state);
END;
PRAGMA user_version = 48;
";

// When a task began and each time it ended, for the OTLP export's root span per task. `created`
// starts again when a revive or reopen starts the next generation. Its seconds match `runs.started`,
// so no run starts before its task. `task_ends` is a log, so its
// `seq` is a cursor the export keeps in `exports.ended`.
const SCHEMA_49: &str = "
ALTER TABLE tasks ADD COLUMN created TEXT;
UPDATE tasks SET created = coalesce((SELECT time FROM inbox WHERE inbox.id = tasks.source), entered, changed);
CREATE TABLE task_ends (
  seq        INTEGER PRIMARY KEY AUTOINCREMENT,
  task       INTEGER NOT NULL REFERENCES tasks (id),
  generation INTEGER NOT NULL,
  state      TEXT NOT NULL,
  created    TEXT NOT NULL,
  ended      TEXT NOT NULL,
  run        INTEGER NOT NULL
);
ALTER TABLE exports ADD COLUMN ended INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER task_created AFTER INSERT ON tasks BEGIN
  UPDATE tasks SET created = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = NEW.id;
END;
CREATE TRIGGER task_ended AFTER UPDATE OF state ON tasks
  WHEN NEW.state IN ('done', 'failed', 'dropped') AND OLD.state NOT IN ('done', 'failed', 'dropped') BEGIN
  INSERT INTO task_ends (task, generation, state, created, ended, run)
    VALUES (NEW.id, NEW.generation, NEW.state, coalesce(NEW.created, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
      strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), (SELECT coalesce(max(id), 0) FROM runs WHERE task = NEW.id));
END;
CREATE TRIGGER task_restarted AFTER UPDATE OF state ON tasks
  WHEN OLD.state IN ('done', 'failed', 'dropped') AND NEW.state NOT IN ('done', 'failed', 'dropped') BEGIN
  UPDATE tasks SET created = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = NEW.id;
END;
PRAGMA user_version = 49;
";

// A task's stack: every pull request it opened or claimed, each based on the one below, and the
// branches a result asks `open-pr` to open, bottom first.
const SCHEMA_50: &str = concat!(
    "CREATE TABLE IF NOT EXISTS pull_requests (
  task     INTEGER NOT NULL REFERENCES tasks (id),
  resource TEXT NOT NULL,
  branch   TEXT NOT NULL,
  base     TEXT NOT NULL,
  state    TEXT NOT NULL DEFAULT 'open',
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  PRIMARY KEY (task, resource)
);
CREATE INDEX IF NOT EXISTS pull_requests_by_resource ON pull_requests (resource);
CREATE TABLE IF NOT EXISTS result_branches (
  run      INTEGER PRIMARY KEY REFERENCES results (run),
  branches TEXT NOT NULL
);
",
    bumps!("pull_requests"),
    "PRAGMA user_version = 50;
"
);

// The outbox id of the compact the daemon queued since the session's context last read below
// `compactAt`, or 0.
const SCHEMA_51: &str = "ALTER TABLE runs ADD COLUMN crossed INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 51;
";

// Every withdrawal of an ask's revision, such as a person's dismissal, with its time, for the
// attention report. A log, so an ask reopened later keeps it. Earlier withdrawals have none.
const SCHEMA_52: &str = "CREATE TABLE IF NOT EXISTS ask_withdrawals (
  ask       INTEGER NOT NULL REFERENCES asks (id),
  revision  INTEGER NOT NULL,
  withdrawn INTEGER NOT NULL,
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS ask_withdrawn AFTER UPDATE OF withdrawn ON asks
  WHEN NEW.withdrawn IS NOT NULL AND NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  INSERT INTO ask_withdrawals (ask, revision, withdrawn) VALUES (NEW.id, NEW.revision, NEW.withdrawn);
END;
PRAGMA user_version = 52;
";

// A task's intervention level and lead, the lead an ask is routed to, and who answered an ask. An
// ask routed to a lead pushes no notification.
const SCHEMA_53: &str = "ALTER TABLE tasks ADD COLUMN intervention TEXT;
ALTER TABLE tasks ADD COLUMN lead TEXT;
ALTER TABLE asks ADD COLUMN routed TEXT;
ALTER TABLE asks ADD COLUMN answered_by TEXT;
DROP TRIGGER IF EXISTS push_a_new_ask;
CREATE TRIGGER push_a_new_ask AFTER INSERT ON asks
  WHEN NEW.answer IS NULL AND NEW.routed IS NULL AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, revision) VALUES ('ask', NEW.id, NEW.revision);
END;
PRAGMA user_version = 53;
";

// How two tasks relate: `task` builds on, fixes, takes up or relates to `other`, as `by` recorded.
const SCHEMA_54: &str = concat!(
    "CREATE TABLE IF NOT EXISTS relations (
  task    INTEGER NOT NULL REFERENCES tasks (id),
  kind    TEXT NOT NULL CHECK (kind IN ('builds-on', 'fixes', 'takes-up', 'relates')),
  other   INTEGER NOT NULL REFERENCES tasks (id),
  by      TEXT NOT NULL,
  created TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  PRIMARY KEY (task, kind, other),
  CHECK (task != other)
);
CREATE INDEX IF NOT EXISTS relations_by_other ON relations (other);
",
    bumps!("relations"),
    "PRAGMA user_version = 54;
"
);

// A person's to-dos: desk items with no task or agent attached.
const SCHEMA_55: &str = concat!(
    "CREATE TABLE IF NOT EXISTS todos (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  text    TEXT NOT NULL,
  band    TEXT,
  due     TEXT,
  created TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  done    TEXT
);
",
    bumps!("todos"),
    "PRAGMA user_version = 55;
"
);

// The irreversible step each of those answers lets run, for the desk's why.
const SCHEMA_56: &str = concat!(
    "ALTER TABLE irreversible_asks ADD COLUMN lets TEXT NOT NULL DEFAULT '';
",
    bumps!("irreversible_asks"),
    "PRAGMA user_version = 56;
"
);

// The desk finds an ask by `coalesce(name, id)` and lists the newest trace and inbox entries by
// `julianday(time)`; the attention report looks up snooze records by trace type, withdrawals by ask and
// asks by task. Without these each read scans every ask, trace record and inbox entry.
const SCHEMA_57: &str = "
CREATE INDEX IF NOT EXISTS asks_by_ref ON asks (coalesce(name, id));
CREATE INDEX IF NOT EXISTS trace_by_day ON trace (julianday(time));
CREATE INDEX IF NOT EXISTS inbox_by_day ON inbox (julianday(time));
CREATE INDEX IF NOT EXISTS trace_by_type ON trace (type);
CREATE INDEX IF NOT EXISTS ask_withdrawals_by_ask ON ask_withdrawals (ask);
CREATE INDEX IF NOT EXISTS asks_by_task ON asks (task);
PRAGMA user_version = 57;
";

// Why an ask or wait was withdrawn, named from the sentinel `withdrawn` held, now a plain 1.
// A wait's -2 also meant an answer, dismiss or stack ask, which the old value cannot tell apart.
const SCHEMA_58: &str = "
DROP TRIGGER IF EXISTS ask_withdrawn;
ALTER TABLE asks ADD COLUMN withdrawn_reason TEXT CHECK (withdrawn_reason IN
  ('revived', 'moved-on', 'sibling-fired', 'answered-elsewhere', 'dismissed', 'asked'));
ALTER TABLE waits ADD COLUMN withdrawn_reason TEXT CHECK (withdrawn_reason IN
  ('revived', 'moved-on', 'sibling-fired', 'answered-elsewhere', 'dismissed', 'asked'));
ALTER TABLE ask_withdrawals ADD COLUMN reason TEXT;
UPDATE asks SET withdrawn_reason = CASE withdrawn WHEN -1 THEN 'dismissed' WHEN -2 THEN 'sibling-fired'
  WHEN 0 THEN 'moved-on' ELSE 'revived' END, withdrawn = 1 WHERE withdrawn IS NOT NULL;
UPDATE waits SET withdrawn_reason = CASE withdrawn WHEN -1 THEN 'dismissed' WHEN -2 THEN 'sibling-fired'
  WHEN 0 THEN 'moved-on' ELSE 'revived' END, withdrawn = 1 WHERE withdrawn IS NOT NULL;
UPDATE ask_withdrawals SET reason = CASE withdrawn WHEN -1 THEN 'dismissed' WHEN -2 THEN 'sibling-fired'
  WHEN 0 THEN 'moved-on' ELSE 'revived' END, withdrawn = 1;
CREATE TRIGGER ask_withdrawn AFTER UPDATE OF withdrawn ON asks
  WHEN NEW.withdrawn IS NOT NULL AND NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  INSERT INTO ask_withdrawals (ask, revision, withdrawn, reason)
    VALUES (NEW.id, NEW.revision, NEW.withdrawn, NEW.withdrawn_reason);
END;
PRAGMA user_version = 58;
";

// A routed ask whose lead task ends or blocks without answering returns to the person: it loses
// its lead and is asked again as its next revision, which pushes it. Every alarm the desk ranks
// pushes too: an intake failure, an uncertain prompt and an uncertain command, with the desk kind
// in `push_queue.state`. A stack's gate records whether its checks are green, so the daemon tells
// once when the whole stack is.
const SCHEMA_59: &str = "
ALTER TABLE waits ADD COLUMN green INTEGER;
CREATE TRIGGER IF NOT EXISTS ask_returns_from_its_lead AFTER UPDATE OF state ON tasks
  WHEN NEW.trigger = 'priory/lead' AND NEW.state IN ('done', 'failed', 'dropped', 'blocked')
  AND NEW.state IS NOT OLD.state BEGIN
  UPDATE asks SET routed = NULL, revision = revision + 1,
    question = question || ' (' || routed || ' did not answer: its task is ' || NEW.state || ')'
  WHERE routed IS NOT NULL AND answer IS NULL AND withdrawn IS NULL
  AND (id, revision) = (SELECT json_extract(data, '$.ask'), json_extract(data, '$.revision')
    FROM inbox WHERE inbox.id = NEW.source);
END;
CREATE TRIGGER IF NOT EXISTS push_an_intake_failure AFTER INSERT ON inbox
  WHEN NEW.type = 'priory.report' AND NEW.subject = 'intake'
  AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, state) VALUES ('alarm', NEW.seq, 'report');
END;
CREATE TRIGGER IF NOT EXISTS push_an_uncertain_prompt AFTER UPDATE OF state ON outbox
  WHEN NEW.state = 'uncertain' AND OLD.state IS NOT 'uncertain'
  AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, state) VALUES ('alarm', NEW.id, 'prompt');
END;
CREATE TRIGGER IF NOT EXISTS push_an_uncertain_command AFTER UPDATE OF status ON actions
  WHEN NEW.status = 'uncertain' AND OLD.status IS NOT 'uncertain' AND NEW.action = 'answer'
  AND EXISTS (SELECT 1 FROM push_subscriptions) BEGIN
  INSERT INTO push_queue (kind, item, state) VALUES ('alarm', NEW.id, 'command');
END;
PRAGMA user_version = 59;
";

// The files a task's branch changed at the end of its latest run, a JSON array, and that run.
const SCHEMA_60: &str = "
ALTER TABLE tasks ADD COLUMN files TEXT;
ALTER TABLE tasks ADD COLUMN files_run INTEGER;
PRAGMA user_version = 60;
";

// Findings: the problems a scheduled run handed to fix tasks, kept as the run memory of its next run.
const SCHEMA_61: &str = "
CREATE TABLE IF NOT EXISTS findings (
  task       INTEGER NOT NULL REFERENCES tasks (id),
  key        TEXT NOT NULL,
  title      TEXT NOT NULL,
  generation INTEGER NOT NULL,
  streak     INTEGER NOT NULL,
  run        INTEGER NOT NULL,
  escalated  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (task, key)
);
PRAGMA user_version = 61;
";

// A risk step's verdict on the finished change: the result keeps it, and the task its latest.
const SCHEMA_62: &str = "
ALTER TABLE results ADD COLUMN merge_risk TEXT;
ALTER TABLE tasks ADD COLUMN merge_risk TEXT;
PRAGMA user_version = 62;
";

// What a plan result proposed: its goal, how it is tested, the change to make and why, and how it will be proved.
const SCHEMA_63: &str = "
ALTER TABLE tasks ADD COLUMN plan_goal TEXT;
ALTER TABLE tasks ADD COLUMN plan_testing TEXT;
ALTER TABLE tasks ADD COLUMN plan_brief TEXT;
ALTER TABLE tasks ADD COLUMN verification TEXT;
PRAGMA user_version = 63;
";

// Migration 58 named every old wait withdrawn as -2 `sibling-fired`, which also meant an answer, a
// dismiss or a stack ask. A wait whose run had no fired wait was withdrawn by its run's dismissed or
// answered ask; a gate of a task none of whose gates fired, by a stack ask. Since 58 every
// `sibling-fired` wait has a fired sibling, so these rows are old ones only.
// A lead task that blocks sent its ask back to the person, so it is dropped: no resume can answer.
// Already blocked lead tasks drop too.
const SCHEMA_64: &str = "
CREATE TRIGGER IF NOT EXISTS a_blocked_lead_drops AFTER UPDATE OF state ON tasks
  WHEN NEW.trigger = 'priory/lead' AND NEW.state = 'blocked' BEGIN
  UPDATE tasks SET state = 'dropped' WHERE id = NEW.id;
END;
UPDATE tasks SET state = 'dropped' WHERE trigger = 'priory/lead' AND state = 'blocked';
UPDATE waits SET withdrawn_reason = CASE
    WHEN EXISTS (SELECT 1 FROM asks WHERE asks.run = waits.run AND asks.withdrawn_reason = 'dismissed')
      THEN 'dismissed'
    WHEN EXISTS (SELECT 1 FROM asks WHERE asks.run = waits.run AND asks.answer IS NOT NULL)
      THEN 'answered-elsewhere'
    ELSE withdrawn_reason END
  WHERE withdrawn_reason = 'sibling-fired' AND NOT daemon AND NOT EXISTS (SELECT 1 FROM waits AS
    sibling WHERE sibling.run = waits.run AND NOT sibling.daemon AND sibling.fired IS NOT NULL);
UPDATE waits SET withdrawn_reason = 'asked'
  WHERE withdrawn_reason = 'sibling-fired' AND daemon AND NOT EXISTS (SELECT 1 FROM waits AS
    sibling WHERE sibling.task = waits.task AND sibling.daemon AND sibling.fired IS NOT NULL);
PRAGMA user_version = 64;
";

// `chosen`: a `subscribe` the task asked for, which carries every event even on its own key. An
// approval's `generation`, so a revive or reopen does not inherit it. An older approval keeps none,
// since which generation it was given in is unknown, so it is asked again.
const SCHEMA_65: &str = "
ALTER TABLE subscriptions ADD COLUMN chosen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE approvals ADD COLUMN generation INTEGER;
PRAGMA user_version = 65;
";

// The step a daemon's checks gate enters once every pull request of its task merged.
const SCHEMA_66: &str = "
ALTER TABLE waits ADD COLUMN after TEXT;
PRAGMA user_version = 66;
";

const SCHEMA_67: &str = "
CREATE TABLE IF NOT EXISTS claims (
  key       TEXT PRIMARY KEY,
  runner    TEXT NOT NULL,
  request   TEXT NOT NULL,
  hold      TEXT NOT NULL DEFAULT 'waiting',
  published TEXT,
  won       INTEGER
);
CREATE TABLE IF NOT EXISTS leases (
  runner TEXT PRIMARY KEY,
  alive  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sends (
  id      INTEGER PRIMARY KEY,
  message TEXT NOT NULL
);
PRAGMA user_version = 67;
";

// The connector a to-do asks a person to sign in to: one open to-do per connector.
const SCHEMA_68: &str = "
ALTER TABLE todos ADD COLUMN connector TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS todos_open_connector ON todos (connector)
  WHERE connector IS NOT NULL AND done IS NULL;
PRAGMA user_version = 68;
";

// The class a result gave each review feedback item of a task, named by its URL.
const SCHEMA_69: &str = concat!(
    "CREATE TABLE IF NOT EXISTS feedback_triage (
  task    INTEGER NOT NULL REFERENCES tasks (id),
  item    TEXT NOT NULL,
  class   TEXT NOT NULL CHECK (class IN ('trivial', 'nothing-to-do', 'persons-call')),
  run     INTEGER NOT NULL REFERENCES runs (id),
  created TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  PRIMARY KEY (task, item)
);
",
    bumps!("feedback_triage"),
    "PRAGMA user_version = 69;
"
);

const SCHEMA_70: &str = "
CREATE TABLE idle_compactions (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  agent    TEXT NOT NULL,
  session  TEXT NOT NULL,
  tmux     TEXT NOT NULL,
  started  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ready    INTEGER NOT NULL DEFAULT 0,
  typed    TEXT,
  ended    TEXT,
  ended_as TEXT
);
CREATE UNIQUE INDEX one_live_idle_compaction ON idle_compactions (session) WHERE ended IS NULL;
ALTER TABLE shared_sessions ADD COLUMN definition TEXT;
PRAGMA user_version = 70;
";

// When a person cleared a failed delivery from the desk with `priory deliver --clear`, and when
// the daemon saw a snooze end, so it traces each end once. Snoozes already over count as seen.
// When an ask or wait was withdrawn, stamped by trigger at every write that withdraws one, and
// backfilled from the withdrawal log. `withdrawn` stays: about 60 queries still test it for null.
const SCHEMA_71: &str = "
ALTER TABLE deliveries ADD COLUMN cleared TEXT;
ALTER TABLE snoozes ADD COLUMN ended TEXT;
UPDATE snoozes SET ended = coalesce(back, until)
  WHERE back IS NOT NULL OR until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
ALTER TABLE asks ADD COLUMN withdrawn_at TEXT;
ALTER TABLE waits ADD COLUMN withdrawn_at TEXT;
UPDATE asks SET withdrawn_at = (SELECT max(created) FROM ask_withdrawals WHERE ask = asks.id)
  WHERE withdrawn IS NOT NULL;
CREATE TRIGGER ask_withdrawn_at AFTER UPDATE OF withdrawn ON asks
  WHEN NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  UPDATE asks SET withdrawn_at = CASE WHEN NEW.withdrawn IS NOT NULL
    THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END WHERE id = NEW.id;
END;
CREATE TRIGGER wait_withdrawn_at AFTER UPDATE OF withdrawn ON waits
  WHEN NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  UPDATE waits SET withdrawn_at = CASE WHEN NEW.withdrawn IS NOT NULL
    THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END WHERE id = NEW.id;
END;
PRAGMA user_version = 71;
";

// The desk items a desktop notification already told a person, so a restarted daemon tells none twice.
const SCHEMA_72: &str = "
CREATE TABLE IF NOT EXISTS notified (
  kind TEXT NOT NULL,
  item TEXT NOT NULL,
  PRIMARY KEY (kind, item)
);
PRAGMA user_version = 72;
";

// A batch: its planning task, the issues a person handed over and the plan its result sent. Each
// entry's task names its batch and entry, and a `merged` hold condition waits on another task's
// stack to merge.
const SCHEMA_73: &str = concat!(
    "CREATE TABLE IF NOT EXISTS batches (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  key              TEXT NOT NULL,
  task             INTEGER NOT NULL UNIQUE REFERENCES tasks (id),
  title            TEXT,
  repo             TEXT,
  band             TEXT,
  lane             TEXT,
  issues           TEXT NOT NULL,
  entries_workflow TEXT NOT NULL,
  state            TEXT NOT NULL DEFAULT 'planning' CHECK (state IN ('planning', 'asked', 'going')),
  plan             TEXT,
  plan_revision    INTEGER NOT NULL DEFAULT 0,
  asked_revision   INTEGER,
  went             TEXT,
  went_by          TEXT,
  created          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
ALTER TABLE tasks ADD COLUMN batch INTEGER;
ALTER TABLE tasks ADD COLUMN batch_entry TEXT;
CREATE TABLE holds_73 (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL
                CHECK (kind IN ('person', 'time', 'task', 'merge', 'blockers', 'issue', 'merged')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  met         TEXT,
  met_as      TEXT,
  released    TEXT,
  released_as TEXT
);
INSERT INTO holds_73 SELECT id, task, reason, kind, until, target, resource, if_closed, resume, ask,
  created, met, met_as, released, released_as FROM holds;
DROP TABLE holds;
ALTER TABLE holds_73 RENAME TO holds;
CREATE INDEX open_holds ON holds (task) WHERE released IS NULL;
",
    bumps!("holds", "batches"),
    "PRAGMA user_version = 73;
"
);

// An event a subscription delivered to a task other than the one routing gave it, with how it
// reached that task. The prompt can be merged or deleted later, so it is not a foreign key.
const SCHEMA_74: &str = concat!(
    "CREATE TABLE IF NOT EXISTS subscription_deliveries (
  seq     INTEGER NOT NULL REFERENCES inbox (seq),
  task    INTEGER NOT NULL REFERENCES tasks (id),
  outcome TEXT NOT NULL,
  outbox  INTEGER,
  PRIMARY KEY (seq, task)
);
",
    bumps!("subscription_deliveries"),
    "PRAGMA user_version = 74;
"
);

// While the daemon cannot reach the log: since when, and the last error. No row while it can.
const SCHEMA_75: &str = concat!(
    "CREATE TABLE IF NOT EXISTS log_link (
  id    INTEGER PRIMARY KEY CHECK (id = 1),
  since TEXT NOT NULL,
  error TEXT NOT NULL
);
",
    bumps!("log_link"),
    "PRAGMA user_version = 75;
"
);

// A batch entry's ask on a predecessor that failed is withdrawn when that predecessor is revived.
// SQLite cannot widen a CHECK, so the column is swapped for one that takes the new reason.
const SCHEMA_76: &str = "
ALTER TABLE asks ADD COLUMN withdrawn_reason_76 TEXT CHECK (withdrawn_reason_76 IN
  ('revived', 'moved-on', 'sibling-fired', 'answered-elsewhere', 'dismissed', 'asked',
   'predecessor-revived'));
UPDATE asks SET withdrawn_reason_76 = withdrawn_reason;
DROP TRIGGER ask_withdrawn;
ALTER TABLE asks DROP COLUMN withdrawn_reason;
ALTER TABLE asks RENAME COLUMN withdrawn_reason_76 TO withdrawn_reason;
CREATE TRIGGER ask_withdrawn AFTER UPDATE OF withdrawn ON asks
  WHEN NEW.withdrawn IS NOT NULL AND NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  INSERT INTO ask_withdrawals (ask, revision, withdrawn, reason)
    VALUES (NEW.id, NEW.revision, NEW.withdrawn, NEW.withdrawn_reason);
END;
PRAGMA user_version = 76;
";

// Stacked batch entries: a hold condition met once another task waits on its open pull request,
// and the task whose branch an entry starts from and bases its pull request on.
const SCHEMA_77: &str = concat!(
    "ALTER TABLE tasks ADD COLUMN base_task INTEGER;
CREATE TABLE holds_77 (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN
                ('person', 'time', 'task', 'merge', 'blockers', 'issue', 'merged', 'waiting')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  met         TEXT,
  met_as      TEXT,
  released    TEXT,
  released_as TEXT
);
INSERT INTO holds_77 SELECT id, task, reason, kind, until, target, resource, if_closed, resume, ask,
  created, met, met_as, released, released_as FROM holds;
DROP TABLE holds;
ALTER TABLE holds_77 RENAME TO holds;
CREATE INDEX open_holds ON holds (task) WHERE released IS NULL;
",
    bumps!("holds"),
    "PRAGMA user_version = 77;
"
);

// A hash of the factory's skills and instructions that a run got, so two runners whose factory
// checkouts differ show it (R11.1).
const SCHEMA_78: &str = "ALTER TABLE runs ADD COLUMN kit_hash TEXT;
PRAGMA user_version = 78;
";

// Every runner's asks on every desk: each ask's `version`, raised by every change its post shows,
// and the version it last posted; the asks other runners posted; the answers and dismisses sent
// to another runner's ask, with what became of each; and the runner this file belongs to, which
// the listings name on its own rows. Asks closed before count as posted.
const SCHEMA_79: &str = concat!(
    "CREATE TABLE IF NOT EXISTS ask_versions (
  ask     INTEGER PRIMARY KEY,
  version INTEGER NOT NULL,
  posted  INTEGER
);
INSERT OR IGNORE INTO ask_versions (ask, version, posted)
  SELECT id, 1, CASE WHEN answer IS NOT NULL OR withdrawn IS NOT NULL THEN 1 END FROM asks;
CREATE TRIGGER IF NOT EXISTS ask_version_asked AFTER INSERT ON asks BEGIN
  INSERT OR IGNORE INTO ask_versions (ask, version) VALUES (NEW.id, 1);
END;
CREATE TRIGGER IF NOT EXISTS ask_version_revised AFTER UPDATE OF revision ON asks
  WHEN NEW.revision IS NOT OLD.revision BEGIN
  INSERT INTO ask_versions (ask, version) VALUES (NEW.id, 2)
    ON CONFLICT (ask) DO UPDATE SET version = version + 1;
END;
CREATE TRIGGER IF NOT EXISTS ask_version_answered AFTER UPDATE OF answer ON asks
  WHEN NEW.answer IS NOT OLD.answer BEGIN
  INSERT INTO ask_versions (ask, version) VALUES (NEW.id, 2)
    ON CONFLICT (ask) DO UPDATE SET version = version + 1;
END;
CREATE TRIGGER IF NOT EXISTS ask_version_withdrawn AFTER UPDATE OF withdrawn ON asks
  WHEN NEW.withdrawn IS NOT OLD.withdrawn BEGIN
  INSERT INTO ask_versions (ask, version) VALUES (NEW.id, 2)
    ON CONFLICT (ask) DO UPDATE SET version = version + 1;
END;
CREATE TRIGGER IF NOT EXISTS ask_version_refused AFTER INSERT ON ask_refusals BEGIN
  INSERT INTO ask_versions (ask, version) VALUES (NEW.ask, 2)
    ON CONFLICT (ask) DO UPDATE SET version = version + 1;
END;
CREATE TABLE IF NOT EXISTS remote_asks (
  runner  TEXT NOT NULL,
  ask     INTEGER NOT NULL,
  version INTEGER NOT NULL,
  data    TEXT NOT NULL,
  PRIMARY KEY (runner, ask)
);
CREATE TABLE IF NOT EXISTS remote_submissions (
  submission TEXT PRIMARY KEY,
  runner     TEXT NOT NULL,
  ask        INTEGER NOT NULL,
  revision   INTEGER,
  verb       TEXT NOT NULL CHECK (verb IN ('answer', 'dismiss')),
  text       TEXT,
  sent       TEXT NOT NULL,
  outcome    TEXT CHECK (outcome IN ('answered', 'dismissed', 'refused')),
  why        TEXT,
  applied    TEXT
);
CREATE INDEX IF NOT EXISTS remote_submissions_by_ask ON remote_submissions (runner, ask);
CREATE TABLE IF NOT EXISTS runner (
  id   INTEGER PRIMARY KEY CHECK (id = 1),
  name TEXT NOT NULL
);
",
    bumps!("remote_asks", "remote_submissions"),
    "PRAGMA user_version = 79;
"
);

// The commit a task's worktree was first created from, so a later reader knows where it started.
const SCHEMA_80: &str = "
ALTER TABLE tasks ADD COLUMN base_sha TEXT;
PRAGMA user_version = 80;
";

// A stacked entry's parent as the `waiting` moment met it: its top pull request, branch and head,
// which the entry's start revalidates and the parent's later pushes move.
const SCHEMA_81: &str = "
ALTER TABLE tasks ADD COLUMN parent_pull_request TEXT;
ALTER TABLE tasks ADD COLUMN parent_branch TEXT;
ALTER TABLE tasks ADD COLUMN parent_head TEXT;
PRAGMA user_version = 81;
";

// A person's hold on a run's keyboard, by the hash of its secret, and each input sent under one: a
// row with no outcome is one whose send a crash may have cut, so it is never sent again.
const SCHEMA_82: &str = concat!(
    "CREATE TABLE IF NOT EXISTS keyboard_grants (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  run     INTEGER NOT NULL REFERENCES runs (id),
  hash    TEXT NOT NULL UNIQUE,
  label   TEXT NOT NULL,
  started TEXT NOT NULL,
  ends    TEXT NOT NULL,
  ended   TEXT,
  why     TEXT
);
CREATE INDEX IF NOT EXISTS keyboard_grants_open ON keyboard_grants (run) WHERE ended IS NULL;
CREATE TABLE IF NOT EXISTS keyboard_keys (
  grant_id INTEGER NOT NULL REFERENCES keyboard_grants (id),
  request  TEXT NOT NULL,
  input    TEXT NOT NULL,
  length   INTEGER,
  outcome  TEXT,
  error    TEXT,
  PRIMARY KEY (grant_id, request)
);
",
    bumps!("keyboard_grants"),
    "PRAGMA user_version = 82;
"
);

// The kit commit this runner gives its runs, why its last fetch failed, and the commit each run got.
const SCHEMA_83: &str = "
CREATE TABLE kit (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  kit_commit TEXT,
  failing_since TEXT,
  error TEXT
);
ALTER TABLE runs ADD COLUMN kit_commit TEXT;
PRAGMA user_version = 83;
";

// The subscriptions a close of their resource ended, which a later event of it open again restores.
const SCHEMA_84: &str = "
CREATE TABLE IF NOT EXISTS closed_subscriptions (
  task     INTEGER NOT NULL REFERENCES tasks (id),
  resource TEXT NOT NULL,
  created  TEXT NOT NULL,
  chosen   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (task, resource)
);
PRAGMA user_version = 84;
";

// Writes to another runner's task: each key's generation as its owner last published it, and the
// one this runner last published for a key of its own; and the resumes, revives, cancels, releases
// and says sent to a task of any runner, with what became of each.
const SCHEMA_85: &str = concat!(
    "ALTER TABLE claims ADD COLUMN generation INTEGER;
ALTER TABLE claims ADD COLUMN published_generation INTEGER;
CREATE TABLE IF NOT EXISTS remote_task_submissions (
  submission TEXT PRIMARY KEY,
  runner     TEXT NOT NULL,
  key        TEXT NOT NULL,
  generation INTEGER NOT NULL,
  act        TEXT NOT NULL CHECK (act IN ('resume', 'revive', 'cancel', 'release', 'say')),
  text       TEXT,
  step       TEXT,
  sent       TEXT NOT NULL,
  outcome    TEXT CHECK (outcome IN ('applied', 'refused')),
  why        TEXT,
  applied    TEXT
);
CREATE INDEX IF NOT EXISTS remote_task_submissions_by_key ON remote_task_submissions (runner, key);
",
    bumps!("remote_task_submissions"),
    "PRAGMA user_version = 85;
"
);

// Memory candidates that agents write and people accept, in log order (R11.1).
const SCHEMA_86: &str = concat!(
    "
CREATE TABLE IF NOT EXISTS memory_candidates (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT,
  text        TEXT,
  key         TEXT NOT NULL,
  runner      TEXT,
  task        INTEGER REFERENCES tasks (id),
  written     TEXT NOT NULL,
  logged      INTEGER,
  accepted    INTEGER,
  dropped     INTEGER,
  decided     TEXT
);
",
    bumps!("memory_candidates"),
    "PRAGMA user_version = 86;
"
);

// A replay's scratch state: its daemon ends a task before an irreversible step, never running it.
const SCHEMA_87: &str = "
CREATE TABLE IF NOT EXISTS replay (
  id      INTEGER PRIMARY KEY CHECK (id = 1),
  started TEXT NOT NULL
);
PRAGMA user_version = 87;
";

// Each subscription a queued notification was handed to, recorded before its `curl` starts, so a
// daemon stopped mid-send does not send it there again, as the outbox does for a prompt.
const SCHEMA_88: &str = "
CREATE TABLE IF NOT EXISTS push_sent (
  row      INTEGER NOT NULL,
  endpoint TEXT NOT NULL,
  PRIMARY KEY (row, endpoint)
);
CREATE TRIGGER IF NOT EXISTS push_sent_leaves_with_its_row AFTER DELETE ON push_queue BEGIN
  DELETE FROM push_sent WHERE row = OLD.id;
END;
PRAGMA user_version = 88;
";

// A task held at a spend cap, the cap it reached and, once a person raises it, the limit it may
// spend to; and each window cap reached now, which the daemon writes for the desk.
const SCHEMA_89: &str = "
CREATE TABLE IF NOT EXISTS spend_holds (
  hold      INTEGER PRIMARY KEY REFERENCES holds (id),
  task      INTEGER NOT NULL REFERENCES tasks (id),
  cap       TEXT NOT NULL,
  cap_usd   REAL NOT NULL,
  spent_usd REAL NOT NULL,
  raised_to REAL
);
CREATE TABLE IF NOT EXISTS spend_reached (
  cap       TEXT PRIMARY KEY,
  line      TEXT NOT NULL
);
PRAGMA user_version = 89;
";

// Each repository recipe the daemon owes or ran in a task's worktree: `setup`, `resume` or
// `teardown`, with its command, its process group while it runs, and its status and output after.
const SCHEMA_90: &str = concat!(
    "
CREATE TABLE IF NOT EXISTS recipes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  moment      TEXT NOT NULL CHECK (moment IN ('setup', 'resume', 'teardown')),
  worktree    TEXT NOT NULL,
  argv        TEXT NOT NULL,
  timeout     INTEGER NOT NULL,
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  started     TEXT,
  pid         INTEGER,
  pid_started INTEGER,
  ended       TEXT,
  status      TEXT,
  output      TEXT,
  shown       TEXT
);
CREATE INDEX IF NOT EXISTS recipes_by_task ON recipes (task, id);
",
    bumps!("recipes"),
    "PRAGMA user_version = 90;
"
);

// Each skill or memory file a run's `PreToolUse` hook added to its context for a path under its
// `paths`, so the run gets each file once.
const SCHEMA_91: &str = "
CREATE TABLE IF NOT EXISTS scoped_sent (
  run  INTEGER NOT NULL REFERENCES runs (id),
  file TEXT NOT NULL,
  PRIMARY KEY (run, file)
);
PRAGMA user_version = 91;
";

// The memories repository's commit this runner gives its runs, why its last fetch failed, and the
// commit each run got.
const SCHEMA_92: &str = "
CREATE TABLE memories_repo (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  memories_commit TEXT,
  failing_since TEXT,
  error TEXT
);
ALTER TABLE runs ADD COLUMN memories_commit TEXT;
PRAGMA user_version = 92;
";

// A `pre` script's title, text and context for a task, and the requests a server stores until it
// runs their `pre` script.
const SCHEMA_93: &str = "
ALTER TABLE tasks ADD COLUMN pre_title TEXT;
ALTER TABLE tasks ADD COLUMN pre_text TEXT;
ALTER TABLE tasks ADD COLUMN pre_context TEXT;
ALTER TABLE tasks ADD COLUMN pre_script TEXT;
DROP VIEW task_text;
CREATE VIEW task_text AS SELECT tasks.id AS task,
  coalesce(tasks.pre_title, json_extract(inbox.data, '$.title'), '') AS title,
  coalesce(tasks.pre_text, json_extract(inbox.data, '$.text'), json_extract(inbox.data, '$.body'),
    json_extract(inbox.data, '$.description'), json_extract(inbox.data, '$.prompt'), '')
    || coalesce(char(10) || tasks.pre_context, '') AS request,
  coalesce(tasks.tldr, '') AS tldr,
  coalesce((SELECT group_concat(summary || ' ' || coalesce(evidence, ''), ' ') FROM results
    WHERE results.task = tasks.id), '') AS results
  FROM tasks LEFT JOIN inbox ON inbox.id = tasks.source;
CREATE TABLE IF NOT EXISTS pending_requests (
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  id      TEXT NOT NULL UNIQUE,
  pending TEXT NOT NULL,
  stored  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 93;
";

// Urgent wakes: the head task a live run stops for at its turn end and since when, the head a task
// was stopped for until its next result, ask or wait, and the runs that protect a head's room.
const SCHEMA_94: &str = "
ALTER TABLE runs ADD COLUMN preempted_for INTEGER;
ALTER TABLE runs ADD COLUMN marked TEXT;
ALTER TABLE tasks ADD COLUMN preempted INTEGER;
ALTER TABLE tasks ADD COLUMN protected TEXT;
PRAGMA user_version = 94;
";

// A debate at a step: the commit its agents read and how it ended; each position a debater took,
// in order; and the debate each run took a turn of.
const SCHEMA_95: &str = "
CREATE TABLE IF NOT EXISTS debates (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task       INTEGER NOT NULL REFERENCES tasks (id),
  step       TEXT NOT NULL,
  commit_sha TEXT,
  started    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended      TEXT,
  ended_as   TEXT,
  reason     TEXT
);
CREATE INDEX IF NOT EXISTS debates_of_task ON debates (task);
CREATE TABLE IF NOT EXISTS positions (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  debate   INTEGER NOT NULL REFERENCES debates (id),
  run      INTEGER NOT NULL UNIQUE REFERENCES runs (id),
  round    INTEGER NOT NULL,
  agent    TEXT NOT NULL,
  harness  TEXT NOT NULL,
  model    TEXT,
  verdict  TEXT NOT NULL,
  argument TEXT NOT NULL,
  concedes TEXT,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS positions_are_immutable BEFORE UPDATE ON positions
  BEGIN SELECT RAISE(ABORT, 'positions are immutable'); END;
ALTER TABLE runs ADD COLUMN debate INTEGER REFERENCES debates (id);
PRAGMA user_version = 95;
";

// A result a step's gate holds until its scorers grade it: the result call's arguments, the scorers,
// and whether the run's turn ended while they graded. A refused result is never stored.
const SCHEMA_96: &str = "
CREATE TABLE IF NOT EXISTS gates (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  run       INTEGER NOT NULL REFERENCES runs (id),
  task      INTEGER NOT NULL REFERENCES tasks (id),
  step      TEXT NOT NULL,
  outcome   TEXT NOT NULL,
  arguments TEXT NOT NULL,
  result    TEXT NOT NULL,
  scorers   TEXT NOT NULL,
  state     TEXT NOT NULL DEFAULT 'grading'
            CHECK (state IN ('grading', 'passed', 'refused', 'dropped')),
  reason    TEXT,
  stopped   INTEGER NOT NULL DEFAULT 0,
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended     TEXT
);
CREATE INDEX IF NOT EXISTS gates_grading ON gates (run) WHERE state = 'grading';
PRAGMA user_version = 96;
";

// The outside session that added a task through `priory mcp --outside`.
const SCHEMA_97: &str = "ALTER TABLE tasks ADD COLUMN added_by TEXT;
PRAGMA user_version = 97;
";

// The confidence and rationale an agent states on its ask, and the lowest confidence at which the
// daemon answers a task's asks with their recommendation.
const SCHEMA_98: &str = "
ALTER TABLE asks ADD COLUMN confidence TEXT CHECK (confidence IN ('high', 'medium', 'low'));
ALTER TABLE asks ADD COLUMN rationale TEXT;
ALTER TABLE tasks ADD COLUMN auto_answer TEXT CHECK (auto_answer IN ('high', 'medium'));
PRAGMA user_version = 98;
";

// Evidence: each file a run's result carries, never changed; each step's `record` command the
// daemon owes or ran after a result; and the last item each approval covers.
const SCHEMA_99: &str = concat!(
    "CREATE TABLE IF NOT EXISTS evidence (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task       INTEGER NOT NULL REFERENCES tasks (id),
  run        INTEGER NOT NULL REFERENCES runs (id),
  kind       TEXT NOT NULL CHECK (kind IN ('image', 'cast', 'video')),
  media      TEXT NOT NULL,
  caption    TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  bytes      INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS evidence_is_never_changed BEFORE UPDATE ON evidence
  BEGIN SELECT RAISE(ABORT, 'evidence is never changed'); END;
CREATE TABLE IF NOT EXISTS evidence_records (
  run        INTEGER PRIMARY KEY REFERENCES runs (id),
  task       INTEGER NOT NULL REFERENCES tasks (id),
  command    TEXT NOT NULL,
  commit_sha TEXT,
  state      TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'recorded', 'refused', 'failed')),
  reason     TEXT,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended      TEXT
);
CREATE TABLE IF NOT EXISTS approval_evidence (
  approval INTEGER PRIMARY KEY REFERENCES approvals (id),
  evidence INTEGER NOT NULL
);
",
    bumps!("evidence", "evidence_records"),
    "PRAGMA user_version = 99;
"
);

// Consults: one question a run asked an investigator, the investigator's task that answers it, and
// the report or why none came. A task holds one open consult at a time. A done, failed or dropped
// asking task, or one revived as its next generation, withdraws its open consult and drops the
// investigator's task; the daemon ends a run that task still has.
const SCHEMA_100: &str = "
CREATE TABLE IF NOT EXISTS consults (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  task          INTEGER NOT NULL REFERENCES tasks (id),
  run           INTEGER NOT NULL REFERENCES runs (id),
  generation    INTEGER NOT NULL,
  step          TEXT NOT NULL,
  agent         TEXT NOT NULL,
  investigator  TEXT NOT NULL,
  question      TEXT NOT NULL,
  asked         TEXT NOT NULL,
  deadline      TEXT NOT NULL,
  cap           REAL NOT NULL,
  investigation INTEGER REFERENCES tasks (id),
  sign_in       TEXT,
  outcome       TEXT,
  report        TEXT,
  reported      TEXT,
  cost          REAL
);
CREATE UNIQUE INDEX IF NOT EXISTS one_open_consult_per_task ON consults (task)
  WHERE outcome IS NULL;
CREATE INDEX IF NOT EXISTS consults_by_investigation ON consults (investigation);
CREATE TRIGGER IF NOT EXISTS a_consult_ends_with_its_asker AFTER UPDATE OF state, generation ON tasks
  WHEN NEW.state IN ('done', 'failed', 'dropped') OR NEW.generation IS NOT OLD.generation BEGIN
  UPDATE tasks SET state = 'dropped', reason = 'its consult was withdrawn: task ' || NEW.id || ' is '
      || iif(NEW.generation IS NOT OLD.generation, 'generation ' || NEW.generation, NEW.state)
    WHERE id IN (SELECT investigation FROM consults WHERE task = NEW.id AND outcome IS NULL)
    AND state NOT IN ('done', 'failed', 'dropped');
  UPDATE consults SET outcome = 'withdrawn', reported = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE task = NEW.id AND outcome IS NULL;
END;
PRAGMA user_version = 100;
";

/// What the queue takes turns between inside a band, from `[queue] fair`: a row only when set.
const SCHEMA_101: &str = "
CREATE TABLE IF NOT EXISTS fairness (
  id  INTEGER PRIMARY KEY CHECK (id = 1),
  key TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS tasks_by_lane ON tasks (band, lane);
CREATE INDEX IF NOT EXISTS runs_of_task ON runs (task);
PRAGMA user_version = 101;
";

// A trigger's intent and no-op cases, as the task's trigger had them when it created the task.
const SCHEMA_102: &str = "
ALTER TABLE tasks ADD COLUMN intent TEXT;
ALTER TABLE tasks ADD COLUMN noop TEXT;
PRAGMA user_version = 102;
";

// A lesson: a memory candidate scoped to the approval step a person rejected, and the lessons a
// gate's pre-review reads, as JSON.
const SCHEMA_103: &str = "
ALTER TABLE memory_candidates ADD COLUMN step TEXT;
ALTER TABLE gates ADD COLUMN lessons TEXT;
PRAGMA user_version = 103;
";

// `[queue] age`: how many seconds a task waits in the queue for each band it moves up in start
// order, and the rank of the highest band it can reach. No row when the factory sets no age.
const SCHEMA_104: &str = "
CREATE TABLE IF NOT EXISTS aging (
  id    INTEGER PRIMARY KEY CHECK (id = 1),
  every INTEGER NOT NULL CHECK (every > 0),
  top   INTEGER NOT NULL
);
PRAGMA user_version = 104;
";

// The ask a gate's held result waits on while a judge it names is not calibrated, shared by every
// gate whose uncalibrated judges are the same, and those judges' names as JSON.
const SCHEMA_105: &str = "
ALTER TABLE gates ADD COLUMN ask INTEGER REFERENCES asks (id);
ALTER TABLE gates ADD COLUMN judges TEXT;
UPDATE gates SET ask = (SELECT id FROM asks WHERE asks.name LIKE 'gate-' || gates.id || '-%'
  ORDER BY id DESC LIMIT 1) WHERE state = 'grading';
PRAGMA user_version = 105;
";

// When a run's `SessionStart` arrived, to the millisecond, its harness, and whether it resumed a
// session, for the `runs` metrics group.
const SCHEMA_106: &str = "
ALTER TABLE runs ADD COLUMN session_started TEXT;
ALTER TABLE runs ADD COLUMN harness TEXT;
ALTER TABLE runs ADD COLUMN resumes INTEGER;
PRAGMA user_version = 106;
";

// The sandbox a run's agent started in, for `priory runs` and its step's trace record.
const SCHEMA_107: &str = "
ALTER TABLE runs ADD COLUMN sandbox TEXT;
PRAGMA user_version = 107;
";

// An outside ask has no task, and a person snoozes it too.
const SCHEMA_108: &str = concat!(
    "
CREATE TABLE snoozes_108 (
  kind        TEXT NOT NULL CHECK (kind IN ('ask', 'task', 'prompt')),
  item        INTEGER NOT NULL,
  task        INTEGER REFERENCES tasks (id),
  snoozed     TEXT NOT NULL,
  until       TEXT NOT NULL,
  back        TEXT,
  back_reason TEXT,
  ended       TEXT,
  PRIMARY KEY (kind, item)
);
INSERT INTO snoozes_108 SELECT kind, item, task, snoozed, until, back, back_reason, ended FROM snoozes;
DROP TABLE snoozes;
PRAGMA legacy_alter_table = ON;
ALTER TABLE snoozes_108 RENAME TO snoozes;
PRAGMA legacy_alter_table = OFF;
",
    bumps!("snoozes"),
    "PRAGMA user_version = 108;
"
);

// A parked hold: a task adopted onto a pull request, which a trigger's request releases.
const SCHEMA_109: &str = concat!(
    "CREATE TABLE holds_109 (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN
                ('person', 'time', 'task', 'merge', 'blockers', 'issue', 'merged', 'waiting', 'parked')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  met         TEXT,
  met_as      TEXT,
  released    TEXT,
  released_as TEXT
);
INSERT INTO holds_109 SELECT id, task, reason, kind, until, target, resource, if_closed, resume, ask,
  created, met, met_as, released, released_as FROM holds;
DROP TABLE holds;
ALTER TABLE holds_109 RENAME TO holds;
CREATE INDEX open_holds ON holds (task) WHERE released IS NULL;
",
    bumps!("holds"),
    "PRAGMA user_version = 109;
"
);

// Whether a run started under `direnv exec`: 1, then 2 once the daemon saw direnv fail and the
// harness start without the `.envrc`.
const SCHEMA_110: &str = "
ALTER TABLE runs ADD COLUMN direnv INTEGER;
PRAGMA user_version = 110;
";

// A memory grant that timed out, whose request time a retry of its task's command keeps.
const SCHEMA_111: &str = "
CREATE TABLE lapses (
  task    INTEGER NOT NULL REFERENCES tasks (id),
  command TEXT NOT NULL,
  bytes   INTEGER NOT NULL,
  created TEXT NOT NULL,
  lapsed  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 111;
";

// A scheduled sweep script runs outside any stall.
const SCHEMA_112: &str = concat!(
    "CREATE TABLE sweeps_112 (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  stall   INTEGER REFERENCES stalls (id),
  script  TEXT NOT NULL,
  started TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended   TEXT,
  status  TEXT,
  output  TEXT,
  pid     INTEGER,
  pid_started INTEGER
);
INSERT INTO sweeps_112 SELECT id, stall, script, started, ended, status, output, pid, pid_started
  FROM sweeps;
DROP TABLE sweeps;
ALTER TABLE sweeps_112 RENAME TO sweeps;
CREATE INDEX sweeps_by_script ON sweeps (script, started);
",
    bumps!("sweeps"),
    "PRAGMA user_version = 112;
"
);

// A task's labels, as a JSON object, which runner filters match, and why a request no runner's
// filters accept waits.
const SCHEMA_113: &str = "
ALTER TABLE tasks ADD COLUMN labels TEXT;
ALTER TABLE inbox ADD COLUMN reason TEXT;
CREATE INDEX inbox_unaccepted ON inbox (subject, seq) WHERE outcome = 'unaccepted';
PRAGMA user_version = 113;
";

/// Each schema change in order: a file at version `n` applies those from index `n` on.
const MIGRATIONS: [&str; VERSION as usize] = [
    SCHEMA, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8, SCHEMA_9,
    SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15, SCHEMA_16, SCHEMA_17,
    SCHEMA_18, SCHEMA_19, SCHEMA_20, SCHEMA_21, SCHEMA_22, SCHEMA_23, SCHEMA_24, SCHEMA_25,
    SCHEMA_26, SCHEMA_27, SCHEMA_28, SCHEMA_29, SCHEMA_30, SCHEMA_31, SCHEMA_32, SCHEMA_33,
    SCHEMA_34, SCHEMA_35, SCHEMA_36, SCHEMA_37, SCHEMA_38, SCHEMA_39, SCHEMA_40, SCHEMA_41,
    SCHEMA_42, SCHEMA_43, SCHEMA_44, SCHEMA_45, SCHEMA_46, SCHEMA_47, SCHEMA_48, SCHEMA_49,
    SCHEMA_50, SCHEMA_51, SCHEMA_52, SCHEMA_53, SCHEMA_54, SCHEMA_55, SCHEMA_56, SCHEMA_57,
    SCHEMA_58, SCHEMA_59, SCHEMA_60, SCHEMA_61, SCHEMA_62, SCHEMA_63, SCHEMA_64, SCHEMA_65,
    SCHEMA_66, SCHEMA_67, SCHEMA_68, SCHEMA_69, SCHEMA_70, SCHEMA_71, SCHEMA_72, SCHEMA_73,
    SCHEMA_74, SCHEMA_75, SCHEMA_76, SCHEMA_77, SCHEMA_78, SCHEMA_79, SCHEMA_80, SCHEMA_81,
    SCHEMA_82, SCHEMA_83, SCHEMA_84, SCHEMA_85, SCHEMA_86, SCHEMA_87, SCHEMA_88, SCHEMA_89,
    SCHEMA_90, SCHEMA_91, SCHEMA_92, SCHEMA_93, SCHEMA_94, SCHEMA_95, SCHEMA_96, SCHEMA_97,
    SCHEMA_98, SCHEMA_99, SCHEMA_100, SCHEMA_101, SCHEMA_102, SCHEMA_103, SCHEMA_104, SCHEMA_105,
    SCHEMA_106, SCHEMA_107, SCHEMA_108, SCHEMA_109, SCHEMA_110, SCHEMA_111, SCHEMA_112, SCHEMA_113,
    SCHEMA_113,
];

/// A task's band rank, null for a band the factory no longer has.
macro_rules! rank {
    () => {
        "(SELECT rank FROM bands WHERE bands.name = tasks.band)"
    };
}

/// The rank a queued or ready task starts in under `[queue] age`: one band higher for each `every`
/// it has waited since it entered the queue, down to rank `top`. Null without an age.
macro_rules! aged {
    () => {
        "(SELECT CASE WHEN tasks.state IN ('queued', 'ready') AND bands.rank > aging.top
           THEN max(aging.top, bands.rank - max(0, CAST((julianday('now')
             - julianday(coalesce(tasks.changed, tasks.created))) * 86400 / aging.every AS INTEGER)))
           ELSE bands.rank END
         FROM bands JOIN aging WHERE bands.name = tasks.band)"
    };
}

/// Whether a task starts in a higher band than its own, by `[queue] age`.
macro_rules! moved_up {
    () => {
        concat!("coalesce(", aged!(), " < ", rank!(), ", 0)")
    };
}

/// Whether a task is a lead's or an investigator's that waits to start, which finishes the work of
/// a task that already ran. The triggers are `tools::LEAD_TRIGGER` and `consult::TRIGGER`.
macro_rules! helper {
    () => {
        "(tasks.trigger IN ('priory/lead', 'priory/consult') AND tasks.state IN ('queued', 'ready'))"
    };
}

/// The order the queue starts tasks in: band, then a waiting lead's or investigator's task, then
/// a person's order, then age. A band the factory no longer has comes after the last one. A task that `[queue] age` moved up starts in the higher
/// band after the tasks a person ordered there. With `[queue] fair = "lane"`, the tasks a person has
/// not ordered take turns between lanes before age: the oldest of each lane first, and among those
/// the lane whose last run started longest ago, so a lane that never ran comes first.
pub const START_ORDER: &str = concat!(
    "coalesce(",
    aged!(),
    ", ",
    rank!(),
    ", (SELECT count(*) FROM bands)), NOT ",
    helper!(),
    ", ",
    moved_up!(),
    " OR tasks.position IS NULL, iif(",
    moved_up!(),
    ", NULL, tasks.position),
  CASE WHEN (tasks.position IS NULL OR ",
    moved_up!(),
    ") AND tasks.state IN ('queued', 'ready')
    AND EXISTS (SELECT 1 FROM fairness WHERE fairness.key = 'lane')
  THEN (SELECT count(*) FROM tasks AS ahead WHERE ahead.band IS tasks.band
    AND ahead.lane IS tasks.lane AND ahead.state IN ('queued', 'ready')
    AND ahead.position IS NULL AND ahead.id < tasks.id) END,
  CASE WHEN (tasks.position IS NULL OR ",
    moved_up!(),
    ") AND tasks.state IN ('queued', 'ready')
    AND EXISTS (SELECT 1 FROM fairness WHERE fairness.key = 'lane')
  THEN (SELECT max(runs.id) FROM tasks AS served JOIN runs ON runs.task = served.id
    WHERE served.band IS tasks.band AND served.lane IS tasks.lane) END,
  tasks.id"
);

/// The band `[queue] age` moved a queued or ready task up to, null when it starts in its own.
pub const AGED_BAND: &str = concat!(
    "(SELECT name FROM bands WHERE rank = ",
    aged!(),
    " AND ",
    moved_up!(),
    ")"
);

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// To the millisecond, so a wait's catch-up never takes an event of the prompt's second as later.
pub(crate) const CONFIRMED: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Marks the state as a replay's, whose daemon never runs an irreversible step.
pub fn mark_replay(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        &format!("INSERT OR IGNORE INTO replay (id, started) VALUES (1, {NOW})"),
        [],
    )
    .map(drop)
}

/// Whether the state is a replay's.
pub(crate) fn replaying(db: &Connection) -> rusqlite::Result<bool> {
    db.query_row("SELECT EXISTS (SELECT 1 FROM replay)", [], |row| row.get(0))
}

/// Stores `base` as the commit the task started from, unless it already has one.
pub(crate) fn set_base(db: &Connection, task: i64, base: Option<&str>) -> rusqlite::Result<()> {
    if let Some(base) = base {
        db.execute(
            "UPDATE tasks SET base_sha = ?2 WHERE id = ?1 AND base_sha IS NULL",
            rusqlite::params![task, base],
        )?;
    }
    Ok(())
}

/// Bumps the revision when a task ended over a day ago left the queue's `ended` list since the
/// last call at `since`, which no write marks. The first call only notes the time.
pub(crate) fn age_queue(db: &Connection, since: &mut Option<String>) -> rusqlite::Result<()> {
    let now: String = db.query_row(&format!("SELECT {NOW}"), [], |row| row.get(0))?;
    if let Some(since) = since {
        db.execute(
            "UPDATE revision SET value = value + 1 WHERE EXISTS (SELECT 1 FROM tasks
               WHERE state IN ('done', 'failed', 'dropped')
                 AND changed < strftime('%Y-%m-%dT%H:%M:%SZ', ?2, '-1 day')
                 AND changed >= strftime('%Y-%m-%dT%H:%M:%SZ', ?1, '-1 day'))",
            params![*since, now],
        )?;
    }
    *since = Some(now);
    Ok(())
}

pub(crate) struct Task {
    pub id: i64,
}

/// Opens the state file `PRIORY_DB`, creating its schema on first use.
pub fn open() -> Result<Connection, String> {
    open_at(&path()?)
}

/// Opens the state file `path`, creating its schema on first use.
pub fn open_at(path: &std::path::Path) -> Result<Connection, String> {
    let at = |error: rusqlite::Error| format!("{}: {error}", path.display());
    let mut db = Connection::open(path).map_err(at)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(at)?;
    // Setting the mode takes a lock that a crashed daemon's WAL recovery can hold past the timeout.
    let mode: String = db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(at)?;
    if mode != "wal" {
        db.pragma_update(None, "journal_mode", "WAL").map_err(at)?;
    }
    let version = |db: &Connection| -> rusqlite::Result<i64> {
        db.pragma_query_value(None, "user_version", |row| row.get(0))
    };
    if version(&db).map_err(at)? == VERSION {
        db.pragma_update(None, "foreign_keys", true).map_err(at)?;
        return Ok(db);
    }
    // A table rebuild needs foreign keys off, or its rename rewrites the references to it.
    db.pragma_update(None, "foreign_keys", false).map_err(at)?;
    migrate(&mut db).map_err(|error| format!("{}: {error}", path.display()))?;
    db.pragma_update(None, "foreign_keys", true).map_err(at)?;
    Ok(db)
}

/// A state in memory with every migration applied, for unit tests.
#[cfg(test)]
pub(crate) fn memory() -> Connection {
    let mut db = Connection::open_in_memory().expect("an in-memory state opens");
    migrate(&mut db).expect("the migrations apply");
    db
}

/// Applies the migrations the file lacks in one transaction, which a broken reference rolls back.
fn migrate(db: &mut Connection) -> Result<(), String> {
    let at = |error: rusqlite::Error| error.to_string();
    let version = |db: &Connection| -> rusqlite::Result<i64> {
        db.pragma_query_value(None, "user_version", |row| row.get(0))
    };
    // Immediate, so two processes opening a new file do not both create its schema.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(at)?;
    let version = version(&tx).map_err(at)?;
    let Some(pending) = usize::try_from(version)
        .ok()
        .and_then(|done| MIGRATIONS.get(done..))
    else {
        return Err(format!(
            "state schema {version} is newer than this priory knows ({VERSION})"
        ));
    };
    tx.execute_batch(&pending.concat()).map_err(at)?;
    let broken: i64 = tx
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(at)?;
    if broken > 0 {
        return Err(format!(
            "the migration to schema {VERSION} left {broken} broken references"
        ));
    }
    tx.commit().map_err(at)
}

fn path() -> Result<std::path::PathBuf, String> {
    std::env::var_os("PRIORY_DB")
        // SQLite keeps nothing on disk for "", ":memory:" or a URI such as "file::memory:".
        .filter(|path| {
            let name = path.to_string_lossy();
            !name.is_empty() && name != ":memory:" && !name.starts_with("file:")
        })
        .map(Into::into)
        .ok_or_else(|| "PRIORY_DB is not set to a file: name the state file".to_owned())
}

/// Opens `PRIORY_DB` for reading only. It refuses a file whose schema this priory does not use,
/// such as one no `open` has created yet.
pub fn read_only() -> Result<Connection, String> {
    let path = path()?;
    let at = |error: rusqlite::Error| format!("{}: {error}", path.display());
    let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(at)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(at)?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(at)?;
    if version != VERSION {
        return Err(format!(
            "{}: state schema {version}, but this priory reads {VERSION}: start `priory daemon` once to create or migrate it",
            path.display()
        ));
    }
    Ok(db)
}

/// The desk line of a Linear view whose filter changed.
fn refiltered(data: &serde_json::Value) -> String {
    let count = |field: &str| data[field].as_array().map_or(0, Vec::len);
    format!(
        "view {} changed its filter: {} entered and {} left, with no task requested",
        data["view"].as_str().unwrap_or_default(),
        priory_factory::plural(count("entered"), "issue", "issues"),
        count("left")
    )
}

/// Records `message` in the inbox and applies its route, in one transaction. A message
/// already in the inbox changes nothing. Returns the tmux sessions of the runs it ended, to kill.
pub(crate) fn handle(
    db: &mut Connection,
    message: &Message,
    trace: &crate::trace::Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let kill = handle_in(&tx, message, trace, counter)?;
    tx.commit()?;
    Ok(kill)
}

/// Records `message` in `tx` and routes it, returning the tmux sessions to kill once it commits.
pub(crate) fn handle_in(
    tx: &Transaction,
    message: &Message,
    trace: &crate::trace::Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let seen = tx
        .query_row(
            "SELECT 1 FROM inbox WHERE id = ?1",
            [message.id.to_string()],
            |_| Ok(()),
        )
        .optional()?;
    if seen.is_some() {
        return Ok(Vec::new());
    }
    route_in(tx, message, trace, counter)
}

/// Moves what names a Linear issue moved to another team onto its new identifier `to`: the keys
/// of its tasks and their batch entries, what they own and subscribe to, and open waits and holds.
fn renamed(tx: &Transaction, message: &Message, to: Option<&String>) -> rusqlite::Result<()> {
    let (Some(to), Some(previous)) = (to, message.data["previous_issue"].as_str()) else {
        return Ok(());
    };
    let moved = message.kind == "linear.issue.updated"
        && message.data["changed"]
            .as_array()
            .is_some_and(|changed| changed.iter().any(|field| field == "identifier"));
    let from = format!("linear:{previous}");
    if !moved || from == *to {
        return Ok(());
    }
    // Every runner and view reads the move, so each renames its own copy of who owns the key.
    for table in ["tasks", "claims", "remote_task_submissions"] {
        tx.execute(
            &format!(
                "UPDATE OR IGNORE {table} SET key = ?2 || substr(key, length(?1) + 1)
                 WHERE key = ?1 OR substr(key, 1, length(?1) + 1) = ?1 || '.'"
            ),
            params![from, to],
        )?;
    }
    for table in ["owns", "subscriptions", "closed_subscriptions"] {
        tx.execute(
            &format!("UPDATE OR IGNORE {table} SET resource = ?2 WHERE resource = ?1"),
            params![from, to],
        )?;
        // What is left already names the new identifier too.
        tx.execute(&format!("DELETE FROM {table} WHERE resource = ?1"), [&from])?;
    }
    tx.execute(
        "UPDATE waits SET resource = ?2 WHERE resource = ?1 AND fired IS NULL",
        params![from, to],
    )?;
    tx.execute(
        "UPDATE holds SET resource = ?2 WHERE resource = ?1 AND released IS NULL",
        params![from, to],
    )
    .map(drop)
}

/// Routes `message` in `tx` and records it, or records its new route when the inbox holds it.
/// Returns the tmux session of each run it ended, to kill once it commits.
pub(crate) fn route_in(
    tx: &Transaction,
    message: &Message,
    trace: &crate::trace::Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let factory = trace.factory;
    if route::outsider(message) {
        record(tx, message, None, "outsider", None)?;
        return Ok(Vec::new());
    }
    if crate::memories::handle(tx, message)?.is_some() {
        return Ok(Vec::new());
    }
    if let Some(me) = trace.runner.as_deref()
        && let Some(kill) = crate::runners::handle(tx, message, me, trace, counter)?
    {
        return Ok(kill);
    }
    let resources = route::resources(message);
    renamed(tx, message, resources.first())?;
    claim_pull_request(tx, message)?;
    // The intake publishes no reopen, so a later event of an open pull request or issue tells it,
    // and gives back the subscriptions its close ended.
    if ![CLOSED, ISSUE_CLOSED].contains(&message.kind.as_str()) && message.data["open"] == true {
        tx.execute(
            "UPDATE pull_requests SET state = 'open' WHERE resource = ?1 AND state = 'closed'",
            [resources.first()],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO subscriptions (task, resource, created, chosen)
             SELECT task, resource, created, chosen FROM closed_subscriptions WHERE resource = ?1",
            [resources.first()],
        )?;
        tx.execute(
            "DELETE FROM closed_subscriptions WHERE resource = ?1",
            [resources.first()],
        )?;
    }
    if message.kind == CLOSED {
        let state = if message.data["merged"] == true {
            "merged"
        } else {
            "closed"
        };
        tx.execute(
            "UPDATE pull_requests SET state = ?2 WHERE resource = ?1",
            params![resources.first(), state],
        )?;
    }
    // A review event goes to its review task first, even when an older task is keyed by its PR.
    // A comment's task request joins the task its comment reached, or one that owns its pull request.
    let feedback = route::feedback(message, factory);
    let (keyed, owners) = match (route::requests(message), route::review(message)) {
        (true, _) if feedback => (
            keyed(tx, resources.first())?,
            joined(tx, message, &resources)?,
        ),
        (true, _) => (keyed(tx, resources.first())?, Vec::new()),
        (false, Some(review)) => (keyed(tx, Some(&review))?, owners(tx, &resources)?),
        (false, None) => (None, owners(tx, &resources)?),
    };
    let owner = keyed.as_ref().or(owners.first());
    // A failed or dropped task keeps its pull request: `priory revive` brings it back.
    let ended = feedback && owner.is_none() && key_ended(tx, resources.first())?;
    let reopening = match owner {
        None => crate::tools::reopenable(tx, message)?,
        Some(_) => None,
    };
    let (task, outcome) = match route::route(message, owner, factory) {
        _ if reopening.is_some() => (reopening, "reopened"),
        _ if ended => (None, "ignored"),
        _ if keyed.is_some() && route::withdraws(message) => {
            (keyed.as_ref().map(|task| task.id), "withdrew")
        }
        Decision::Create(new) => {
            tx.execute(
                "INSERT INTO tasks
                   (key, workflow, step, state, band, lane, team, project, lane_derived, trigger, source, repo, entered, tldr, intervention, lead,
                    pre_title, pre_text, pre_context, pre_script, added_by, auto_answer, intent, noop, labels)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?12, ?13, ?14,
                   ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                params![
                    new.key,
                    new.workflow,
                    new.step,
                    new.band,
                    new.lane,
                    new.team,
                    new.project,
                    new.lane_derived,
                    new.trigger,
                    new.source,
                    new.repo,
                    new.tldr
                        .as_deref()
                        .map(str::trim)
                        .filter(|tldr| !tldr.is_empty())
                        .map(|tldr| crate::search::fit(tldr, counter)),
                    new.intervention.map(|intervention| intervention.as_str()),
                    new.lead,
                    new.title,
                    new.text,
                    new.context,
                    new.pre.as_ref().map(|pre| &pre.script),
                    new.session,
                    new.auto_answer.map(|level| level.as_str()),
                    new.intent,
                    (!new.noop.is_empty()).then(|| serde_json::json!(new.noop).to_string()),
                    (!new.labels.is_empty()).then(|| serde_json::json!(new.labels).to_string())
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
                params![id, new.key],
            )?;
            // Its own issue, pull request or thread wakes it and reopens it once done. It reads from
            // the request's time, so a reply posted before the task existed still reaches it.
            if let Some(resource) = crate::subscribe::resource(&new.key) {
                tx.execute(
                    "INSERT INTO subscriptions (task, resource, created) VALUES (?1, ?2,
                       min(strftime('%Y-%m-%dT%H:%M:%SZ', ?3, 'unixepoch'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))",
                    params![id, resource, message.time.as_second()],
                )?;
                tx.execute(
                    "DELETE FROM subscriptions WHERE resource = ?2 AND task != ?1
                     AND task IN (SELECT id FROM tasks WHERE state = 'done')",
                    params![id, resource],
                )?;
            }
            if let Some(hold) = &new.hold {
                crate::hold::filed(tx, id, hold, factory)?;
            }
            crate::relate::created(tx, id, &new.key, &new.relates)?;
            if let Some(batch) = &new.batch {
                crate::batch::created(tx, id, &new.key, batch)?;
            }
            (Some(id), "created")
        }
        // A parked task's release starts its first run, which reads the events it kept.
        Decision::Keep(id) if feedback => (
            Some(id),
            match crate::hold::unpark(tx, id, message.data["trigger"].as_str().unwrap_or("?"))? {
                true => "released",
                false => "coalesced",
            },
        ),
        Decision::Keep(id) if route::tick(message, factory) => (
            Some(id),
            if wakes(tx, id, factory)? {
                "woke"
            } else {
                "coalesced"
            },
        ),
        Decision::Keep(id) if message.kind == priory_intake::request::REQUESTED => {
            (Some(id), "said")
        }
        Decision::Keep(id) => (Some(id), "kept"),
        Decision::Ignore => (None, "ignored"),
    };
    if let Some(id) = task.filter(|_| ["created", "kept"].contains(&outcome)) {
        merged(tx, id, message)?;
    }
    let running = match task {
        Some(id) => tx
            .query_row(
                "SELECT 1 FROM tasks WHERE id = ?1 AND state = 'running'",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some(),
        None => false,
    };
    // A debater never reads what the task keeps: the step after the debate reads it unread.
    let debating = match task {
        Some(id) if running => crate::debate::open(tx, id)?,
        _ => false,
    };
    let outbox = if let Some(task) = task.filter(|_| running && !debating && outcome == "kept") {
        let text = line(
            &message.kind,
            message.subject.as_deref(),
            &message.data.to_string(),
        );
        let mut text = placed(tx, task, &resources, &text)?;
        if crate::triage::item(&message.kind, &message.data).is_some() {
            text = format!("{text}\n{}", crate::triage::RULE);
        }
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![task, text],
        )?;
        Some(tx.last_insert_rowid())
    } else {
        None
    };
    let seq = record(tx, message, task, outcome, outbox)?;
    if outcome == "ignored"
        && let Some(line) = crate::tools::unheard(tx, message)?
    {
        crate::post::report(tx, "priory", &line)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
    }
    crate::grades::label(tx, trace, message)?;
    if message.kind == "linear.view.changed" {
        crate::post::report(tx, "linear", &refiltered(&message.data))?;
    }
    crate::hold::event(tx, &resources)?;
    if message.kind == "linear.issue.left"
        && let Some(resource) = resources.first()
    {
        crate::hold::left(tx, resource, message.data["view"].as_str())?;
    }
    if message.kind == "linear.issue.entered"
        && let Some(resource) = resources.first()
    {
        crate::hold::entered(tx, resource, message.data["view"].as_str())?;
    }
    if message.kind == priory_intake::intake::PR_CLOSED && message.data["merged"] == true {
        crate::tools::restack_stacked(tx, trace, seq, &message.data)?;
    }
    // The keyed task keeps the event; another task that shares the resource hears it through its
    // wait or its subscription. A merge that fires a gate may end a run a person's `say` started.
    let mut kill = Vec::new();
    for other in owners.iter().filter(|other| Some(other.id) != task) {
        let (fired, ended) = crate::tools::event(tx, other.id, seq, factory)?;
        kill.extend(ended);
        if fired {
            continue;
        }
        let heard = match delivered(tx, other.id, message, &resources, None, factory)? {
            Some(heard) => Some(heard),
            // A task that cannot hear it now keeps it for its next run, as the first owner does.
            None if subscribes(tx, other.id, &resources, &message.kind)? => {
                told(tx, other.id, message)?;
                Some(("kept", None))
            }
            None => None,
        };
        if let Some((outcome, prompt)) = heard {
            tx.execute(
                "INSERT OR IGNORE INTO subscription_deliveries (seq, task, outcome, outbox)
                 VALUES (?1, ?2, ?3, ?4)",
                params![seq, other.id, outcome, prompt],
            )?;
        }
    }
    match (task, outcome) {
        (Some(task), "kept") => {
            let (fired, ended) = crate::tools::event(tx, task, seq, factory)?;
            kill.extend(ended);
            if !fired {
                if delivered(tx, task, message, &resources, Some(seq), factory)?
                    .is_some_and(|(outcome, _)| outcome == "woke")
                {
                    tx.execute("UPDATE inbox SET outcome = 'woke' WHERE seq = ?1", [seq])?;
                } else {
                    told(tx, task, message)?;
                }
            }
        }
        (Some(task), "woke") => {
            let line = line(
                &message.kind,
                message.subject.as_deref(),
                &message.data.to_string(),
            );
            let text = message.data["prompt"].as_str().unwrap_or(&line);
            let text = match crate::findings::memory(tx, task)? {
                Some(memory) => format!("{text}\n{memory}"),
                None => text.to_owned(),
            };
            crate::tools::wake_on(tx, task, &text, Some(seq))?;
        }
        (Some(task), "said") => {
            let state: String =
                tx.query_row("SELECT state FROM tasks WHERE id = ?1", [task], |row| {
                    row.get(0)
                })?;
            let text = message.data["text"].as_str().unwrap_or_default();
            let text = format!("{}{text}", crate::tools::SAYS);
            // An agent that cannot hear it yet reads the request with its next prompt instead.
            if crate::tools::say_or_hold(tx, task, &text, &state, &factory.workflows)?.is_err() {
                tx.execute("UPDATE inbox SET outcome = 'kept' WHERE seq = ?1", [seq])?;
                kill.extend(crate::tools::event(tx, task, seq, factory)?.1);
            } else {
                tx.execute(
                    "UPDATE inbox SET outbox = (SELECT max(id) FROM outbox WHERE task = ?2 AND state IN ('queued', 'held'))
                     WHERE seq = ?1",
                    [seq, task],
                )?;
            }
        }
        (Some(task), "reopened") => crate::tools::reopen(tx, trace, task, seq)?,
        (Some(task), "withdrew") => {
            kill.extend(crate::tools::cancel(tx, task, "review request removed")?);
            return Ok(kill);
        }
        _ => {}
    }
    // After the delivery above, so the close reaches its subscribers.
    if [CLOSED, ISSUE_CLOSED].contains(&message.kind.as_str()) {
        tx.execute(
            "INSERT OR REPLACE INTO closed_subscriptions (task, resource, created, chosen)
             SELECT task, resource, created, chosen FROM subscriptions WHERE resource = ?1",
            [resources.first()],
        )?;
        tx.execute(
            "DELETE FROM subscriptions WHERE resource = ?1",
            [resources.first()],
        )?;
    }
    Ok(kill)
}

/// Stores `message` in the inbox with the task it went to and its outcome, and returns its seq.
pub(crate) fn record(
    tx: &Transaction,
    message: &Message,
    task: Option<i64>,
    outcome: &str,
    outbox: Option<i64>,
) -> rusqlite::Result<i64> {
    tx.query_row(
        "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome, outbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET task = excluded.task, outcome = excluded.outcome,
           outbox = excluded.outbox
         RETURNING seq",
        params![
            message.id.to_string(),
            message.kind,
            message.subject,
            message.time.to_string(),
            message.data.to_string(),
            serde_json::to_string(&message.links).expect("links serialize"),
            task,
            outcome,
            outbox
        ],
        |row| row.get(0),
    )
}

/// Subscribes the task whose branch is the head of the open pull request `message` names, on the
/// first event of that pull request, when no other active task owns it, and adds it to its stack.
/// A head named after the task's branch, `priory/7-api`, counts too when its base is the trunk (a
/// branch outside `priory/`) or another branch of the task. So a pull request or a stack a worker
/// opened itself is its own, and a later `unsubscribe` holds.
fn claim_pull_request(tx: &Transaction, message: &Message) -> rusqlite::Result<()> {
    let data = &message.data;
    let (Some(repo), Some(number), Some(head)) = (
        data["repo"].as_str(),
        data["pull_request"].as_u64(),
        data["head_branch"].as_str(),
    ) else {
        return Ok(());
    };
    if !message.kind.starts_with("github.") || message.kind == CLOSED || data["open"] == false {
        return Ok(());
    }
    if data.get("head_repo").is_some_and(|head| head != repo) {
        return Ok(());
    }
    let resource = route::pull_request(repo, number);
    let base = data["base_branch"].as_str().unwrap_or_default();
    let claimer = tx
        .query_row(
            "SELECT owns.task, tasks.worktree FROM owns JOIN tasks ON tasks.id = owns.task
             WHERE owns.resource GLOB 'github:*@*' AND tasks.state NOT IN ('failed', 'dropped')
             AND (owns.resource = ?2 OR (?2 GLOB owns.resource || '-?*'
               AND (?5 NOT GLOB 'priory/*' OR ('github:' || ?3 || '@' || ?5) = owns.resource
                 OR ('github:' || ?3 || '@' || ?5) GLOB owns.resource || '-?*')))
             AND NOT EXISTS (SELECT 1 FROM owned JOIN tasks AS other ON other.id = owned.task
               WHERE owned.resource = ?1 AND owned.task != owns.task
               AND other.state NOT IN ('done', 'failed', 'dropped'))
             AND NOT EXISTS (SELECT 1 FROM inbox WHERE json_extract(data, '$.repo') = ?3
               AND json_extract(data, '$.pull_request') = ?4 AND outcome != 'outsider')
             ORDER BY owns.task DESC LIMIT 1",
            params![
                resource,
                route::branch(repo, head),
                repo,
                number as i64,
                base
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    // Only a branch the task made in its worktree is its own: a same-named one is someone else's.
    let made = |worktree: &str| {
        priory_runtime::runtime::head(std::path::Path::new(worktree), head).is_some()
    };
    let Some((task, Some(_))) =
        claimer.filter(|(_, worktree)| worktree.as_deref().is_some_and(made))
    else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO subscriptions (task, resource) VALUES (?1, ?2)",
        params![task, resource],
    )?;
    // A pull request `open-pr` already recorded keeps the base it was opened on.
    let known = tx
        .query_row(
            "SELECT 1 FROM pull_requests WHERE task = ?1 AND resource = ?2",
            params![task, resource],
            |_| Ok(()),
        )
        .optional()?;
    if known.is_some() {
        return Ok(());
    }
    crate::stack::record(tx, task, &resource, head, base)
}

/// Whether `task` sleeps where an event can wake it: asleep at a model step, with no daemon's
/// checks gate, no `plan` approval open and no result a gate holds, since a human step or a gate
/// has no agent turn to wake.
fn wakes(tx: &Transaction, task: i64, factory: &Factory) -> rusqlite::Result<bool> {
    let asleep_at: Option<(String, String)> = tx
        .query_row(
            &format!(
                "SELECT workflow, step FROM tasks WHERE id = ?1 AND state = 'asleep'
                 AND NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL AND daemon)
                 AND NOT EXISTS (SELECT 1 FROM asks WHERE task = ?1 AND kind = 'step'
                   AND answer IS NULL AND withdrawn IS NULL)
                 AND NOT EXISTS ({})",
                crate::tools::HELD
            ),
            [task],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(asleep_at.is_some_and(|(workflow, step)| {
        factory
            .workflows
            .get(&workflow)
            .and_then(|workflow| workflow.value.step(&step))
            .is_some_and(|step| step.decides == priory_factory::factory::Decider::Model)
    }))
}

/// Delivers `message` to `task` when it subscribes to a resource of `resources` and no wait took
/// it: a wake while it sleeps where an event can wake it, else a queued prompt while it runs. `kept`
/// is the inbox entry when `task` kept the message, whose own prompt is already queued. Returns
/// `woke` or `kept`, with the prompt that carries the message, when it reached the task.
fn delivered(
    tx: &Transaction,
    task: i64,
    message: &Message,
    resources: &[String],
    kept: Option<i64>,
    factory: &Factory,
) -> rusqlite::Result<Option<(&'static str, Option<i64>)>> {
    if !subscribes(tx, task, resources, &message.kind)? {
        return Ok(None);
    }
    let queued = |tx: &Transaction| -> rusqlite::Result<Option<i64>> {
        tx.query_row(
            "SELECT max(id) FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0",
            [task],
            |row| row.get(0),
        )
    };
    let line = line(
        &message.kind,
        message.subject.as_deref(),
        &message.data.to_string(),
    );
    // An event the task keeps that was written while it ran, such as the agent's own comment,
    // wakes nothing: its next wake carries it. To the second, as waits: the last second is after.
    // A comment the intake knows no agent wrote is a person's, whenever it was written.
    // An item a decision model settled wakes no one; the task's next prompt carries it.
    let fresh: bool = !crate::decision::settled(&message.data)
        && (kept.is_none() || crate::tools::not_agents(&message.data)
        || tx.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM runs WHERE id = (SELECT max(id) FROM runs WHERE task = ?1)
               AND julianday(?2) >= (SELECT julianday(min(started)) FROM runs WHERE task = ?1)
               AND julianday(?2) < julianday(ended))",
            params![task, message.time.to_string()],
            |row| row.get(0),
        )?);
    if fresh && wakes(tx, task, factory)? {
        crate::tools::wake_on(tx, task, &placed(tx, task, resources, &line)?, kept)?;
        return Ok(Some(("woke", queued(tx)?)));
    }
    if fresh && crate::tools::gate_feedback(tx, task, message, kept)? {
        return Ok(Some(("woke", queued(tx)?)));
    }
    if kept.is_none()
        && tx.execute(
            "INSERT INTO outbox (task, text) SELECT id, ?2 FROM tasks WHERE id = ?1 AND state = 'running'
             AND NOT EXISTS (SELECT 1 FROM debates WHERE task = ?1 AND ended IS NULL)",
            params![task, placed(tx, task, resources, &line)?],
        )? == 1
    {
        return Ok(Some(("kept", Some(tx.last_insert_rowid()))));
    }
    Ok(None)
}

/// Adds what a person said in `message` to the open step ask of `task`, since no agent runs while
/// a person decides and the answer may lead to `done` with no run at all.
fn told(tx: &Transaction, task: i64, message: &Message) -> rusqlite::Result<bool> {
    if !OWN_KEY.contains(&message.kind.as_str()) || message.data["own"] == true {
        return Ok(false);
    }
    let line = crate::tools::heard(&message.kind, message.subject.as_deref(), &message.data);
    crate::tools::tell_ask(tx, task, &line)
}

/// The events a task's subscription to its own key carries: what people say, edit, close and take
/// out of a view. Checks,
/// labels and pushes stay with the checks gate and waits, since each would start a run.
pub(crate) const OWN_KEY: [&str; 11] = [
    "github.pull_request_review.submitted",
    "github.pull_request.comment",
    "github.pull_request_review_comment",
    CLOSED,
    "github.issues.comment",
    ISSUE_CLOSED,
    "linear.comment.created",
    "linear.issue.state_changed",
    "linear.issue.updated",
    "linear.issue.left",
    "slack.thread.replied",
];

/// `line`, after the place in the stack of `task` of the pull request `resources` names first, as
/// `#58, 2 of 3: <line>`.
fn placed(
    tx: &Transaction,
    task: i64,
    resources: &[String],
    line: &str,
) -> rusqlite::Result<String> {
    let place = match resources.first() {
        Some(resource) => crate::stack::place(tx, task, resource)?,
        None => None,
    };
    Ok(place.map_or_else(|| line.to_owned(), |place| format!("{place}: {line}")))
}

/// Whether `task` subscribes to any of `resources` with no open wait on them: a wait on the
/// resource decides which of its events wake the task. Its own key's subscription takes only
/// events of `OWN_KEY` kinds.
fn subscribes(
    tx: &Transaction,
    task: i64,
    resources: &[String],
    kind: &str,
) -> rusqlite::Result<bool> {
    let resources = serde_json::to_string(resources).expect("resources serialize");
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM subscriptions WHERE task = ?1
           AND resource IN (SELECT value FROM json_each(?2))
           AND (?3 OR chosen OR resource IS NOT (SELECT key FROM tasks WHERE id = ?1)))
         AND NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL
           AND resource IN (SELECT value FROM json_each(?2)))",
        params![task, resources, OWN_KEY.contains(&kind)],
        |row| row.get(0),
    )
}

/// The factory's bands, highest first, as the daemon last recorded them.
pub fn band_names(db: &Connection) -> rusqlite::Result<Vec<String>> {
    db.prepare("SELECT name FROM bands ORDER BY rank")?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// Refuses `band` with the factory's list when the factory has no band of that name.
pub fn check_band(db: &Connection, band: &str) -> Result<(), String> {
    let bands = band_names(db).map_err(|error| error.to_string())?;
    if bands.iter().any(|known| known == band) {
        return Ok(());
    }
    Err(format!("band `{band}` is not one of {}", bands.join(", ")))
}

/// Records the factory's bands, highest first, and its `[queue] age`, so listings and moves read
/// them without the factory.
pub(crate) fn bands(
    db: &mut Connection,
    bands: &[&str],
    age: Option<(Duration, usize)>,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM bands", [])?;
    tx.execute("DELETE FROM aging", [])?;
    if let Some((every, top)) = age {
        tx.execute(
            "INSERT INTO aging (id, every, top) VALUES (1, ?1, ?2)",
            params![
                i64::try_from(every.as_secs()).unwrap_or(i64::MAX),
                top as i64
            ],
        )?;
    }
    for (rank, band) in bands.iter().enumerate() {
        tx.execute(
            "INSERT INTO bands (name, rank) VALUES (?1, ?2)",
            params![band, rank as i64],
        )?;
    }
    // Tasks from before bands have none: the last band is what a request without one gets.
    tx.execute(
        "UPDATE tasks SET band = ?1 WHERE band IS NULL",
        [bands.last()],
    )?;
    tx.commit()
}

/// Records `[queue] fair`, so listings show the order the daemon starts tasks in without the
/// factory.
pub(crate) fn fairness(db: &Connection, fair: Option<Fair>) -> rusqlite::Result<()> {
    match fair {
        Some(Fair::Lane) => db.execute(
            "INSERT INTO fairness (id, key) VALUES (1, 'lane')
             ON CONFLICT (id) DO UPDATE SET key = 'lane'",
            [],
        ),
        None => db.execute("DELETE FROM fairness", []),
    }
    .map(drop)
}

/// Records each workflow's human steps whose answer lets an irreversible step run, so listings
/// read an ask's risk without the factory.
pub(crate) fn irreversible_asks(
    db: &mut Connection,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM irreversible_asks", [])?;
    for (name, workflow) in workflows {
        for step in &workflow.value.steps {
            if let Some(lets) = explain::irreversible_step_after(&workflow.value, &step.id) {
                tx.execute(
                    "INSERT INTO irreversible_asks (workflow, step, lets) VALUES (?1, ?2, ?3)",
                    params![name, step.id, lets],
                )?;
            }
        }
    }
    tx.commit()
}

/// Fires every time wait whose instant passed and task wait whose task ended. `head` reads the
/// commit a branch points to in a worktree.
pub(crate) fn due(
    db: &mut Connection,
    factory: &Factory,
    head: &dyn Fn(&str, &str) -> Option<String>,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::tools::due(&tx)?;
    crate::hold::due(&tx)?;
    crate::hold::merged(&tx, factory, head)?;
    tx.commit()
}

/// How many tasks with `key` ended: done, failed or dropped.
pub fn ended(db: &Connection, key: &str) -> rusqlite::Result<u64> {
    db.query_row(
        "SELECT count(*) FROM tasks WHERE key = ?1 AND state IN ('done', 'failed', 'dropped')",
        [key],
        |row| row.get::<_, i64>(0).map(i64::unsigned_abs),
    )
}

/// The resource and the agent's last prompt of each open wait that the intake polls for, then the
/// pull request or Linear issue and creation time of each unmet merge condition and each issue
/// condition of an open hold, met or not, then each subscription of a task that has not failed or
/// been dropped, and the own key of the newest task on a key that did, then the branch of each
/// active task, or of a task done in the last 10 minutes whose branch no event named yet, so the
/// intake sees a pull request its worker opened itself. Each comes with its rank: 0 for an open wait
/// or hold, 1 for an active task, 2 for one that ended.
pub fn watched(db: &Connection) -> rusqlite::Result<Vec<(String, String, u8)>> {
    db.prepare(
        "SELECT resource, since, rank FROM (
           SELECT 0 AS source, waits.id, waits.resource, coalesce(waits.since, waits.created) AS since,
             0 AS rank
           FROM waits
           JOIN tasks ON tasks.id = waits.task AND tasks.state NOT IN ('done', 'failed', 'dropped')
           WHERE waits.fired IS NULL AND waits.withdrawn IS NULL
           AND waits.kind IN ('linear_comment', 'linear_state', 'slack_reply', 'pull_request', 'checks', 'review')
           UNION ALL
           SELECT 1, holds.id, holds.resource, holds.created, 0 FROM holds
           JOIN tasks ON tasks.id = holds.task AND tasks.state NOT IN ('done', 'failed', 'dropped')
           WHERE holds.kind IN ('merge', 'issue') AND holds.released IS NULL
           AND (holds.met IS NULL OR holds.kind = 'issue')
           UNION ALL
           SELECT 2, subscriptions.rowid, subscriptions.resource, subscriptions.created,
             CASE WHEN tasks.state IN ('done', 'failed', 'dropped') THEN 2 ELSE 1 END
           FROM subscriptions
           JOIN tasks ON tasks.id = subscriptions.task AND (tasks.state NOT IN ('failed', 'dropped')
             -- A comment on the own key of an ended task's newest generation asks for a revive.
             OR (subscriptions.resource = tasks.key
               AND tasks.id = (SELECT max(id) FROM tasks AS newest WHERE newest.key = tasks.key)))
           -- Slack reads one thread per call, so an ended task's own thread is read for a week only.
           WHERE NOT (tasks.state IN ('done', 'failed', 'dropped') AND subscriptions.resource = tasks.key
             AND tasks.key GLOB 'slack:*' AND julianday('now') - julianday(tasks.changed) >= 7)
           UNION ALL
           SELECT 3, owns.rowid, owns.resource, tasks.changed,
             CASE WHEN tasks.state IN ('done', 'failed', 'dropped') THEN 2 ELSE 1 END FROM owns
           JOIN tasks ON tasks.id = owns.task
           WHERE owns.resource GLOB 'github:*@*' AND (tasks.state NOT IN ('done', 'failed', 'dropped')
             OR (tasks.state = 'done' AND julianday('now') - julianday(tasks.changed) < 10.0 / 1440
               AND NOT EXISTS (SELECT 1 FROM inbox WHERE inbox.outcome != 'outsider' AND owns.resource =
                 'github:' || json_extract(inbox.data, '$.repo') || '@' || json_extract(inbox.data, '$.head_branch'))))
         ) ORDER BY source, id",
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
    .collect()
}

/// The merged pull requests of a task, the pull request's repository, number, head and base branch,
/// merge commit and merge time, merged since `since` and with no check run event at the merge
/// commit that saw every check complete: the commits the intake still reads check runs at.
pub fn merges(db: &Connection, since: &str) -> rusqlite::Result<Vec<priory_intake::intake::Merge>> {
    db.prepare(
        "SELECT DISTINCT json_extract(data, '$.repo'), json_extract(data, '$.pull_request'),
           json_extract(data, '$.head_branch'), json_extract(data, '$.base_branch'),
           json_extract(data, '$.merge_commit'), time
         FROM inbox closed
         WHERE type = ?1 AND json_extract(data, '$.merged') = 1 AND time >= ?2
         AND json_extract(data, '$.merge_commit') IS NOT NULL
         AND (EXISTS (SELECT 1 FROM pull_requests WHERE resource = 'github:'
               || json_extract(closed.data, '$.repo') || '#' || json_extract(closed.data, '$.pull_request'))
           OR EXISTS (SELECT 1 FROM owns WHERE resource = 'github:'
               || json_extract(closed.data, '$.repo') || '@' || json_extract(closed.data, '$.head_branch')))
         AND NOT EXISTS (SELECT 1 FROM inbox WHERE type = ?3
           AND json_extract(data, '$.repo') = json_extract(closed.data, '$.repo')
           AND json_extract(data, '$.head_sha') = json_extract(closed.data, '$.merge_commit')
           AND json_extract(data, '$.pending') = 0
           -- A crash may have stored the passing events of the batch and not the failing ones.
           AND json_extract(data, '$.failing') <= (SELECT count(*) FROM inbox AS red
             WHERE red.type = ?3 AND json_extract(red.data, '$.repo') = json_extract(closed.data, '$.repo')
             AND json_extract(red.data, '$.head_sha') = json_extract(closed.data, '$.merge_commit')
             AND json_extract(red.data, '$.conclusion') NOT IN ('success', 'neutral', 'skipped')
             AND json_extract(red.data, '$.superseded_by') IS NULL))
         ORDER BY time",
    )?
    .query_map(params![CLOSED, since, CHECKED], |row| {
        Ok(priory_intake::intake::Merge {
            repo: row.get(0)?,
            pull_request: row.get::<_, i64>(1)? as u64,
            head_branch: row.get(2)?,
            base_branch: row.get(3)?,
            commit: row.get(4)?,
            time: row.get(5)?,
        })
    })?
    .collect()
}

/// Fires the disk waits the free disk that `free` measures satisfies; measures only while one is open.
pub(crate) fn disk_due(
    db: &mut Connection,
    free: impl FnOnce() -> Option<u64>,
) -> rusqlite::Result<()> {
    let open = db
        .query_row(
            "SELECT 1 FROM waits WHERE kind = 'disk' AND fired IS NULL AND withdrawn IS NULL",
            [],
            |_| Ok(()),
        )
        .optional()?;
    let Some(free) = open.and_then(|()| free()) else {
        return Ok(());
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::tools::disk_due(&tx, free)?;
    tx.commit()
}

/// The active task whose own key is `key`, or the oldest batch part of it, keyed `<key>.<part>`:
/// owning the key as a resource is not enough.
pub(crate) fn keyed(tx: &Transaction, key: Option<&String>) -> rusqlite::Result<Option<Task>> {
    tx.query_row(
        "SELECT id FROM tasks WHERE (key = ?1 OR batch IS NOT NULL
           AND substr(key, 1, length(?1) + 1) = ?1 || '.' AND batch_entry IS NOT NULL)
         AND state NOT IN ('done', 'failed', 'dropped') ORDER BY key != ?1, id LIMIT 1",
        [key],
        |row| Ok(Task { id: row.get(0)? }),
    )
    .optional()
}

/// The tasks a comment trigger's task request joins: the one its source event reached, kept or
/// reopened, then every active task that owns its pull request.
fn joined(
    tx: &Transaction,
    message: &Message,
    resources: &[String],
) -> rusqlite::Result<Vec<Task>> {
    let reached = tx
        .query_row(
            "SELECT task FROM inbox WHERE id = ?1 AND task IS NOT NULL",
            [message.data["source"].as_str()],
            |row| Ok(Task { id: row.get(0)? }),
        )
        .optional()?;
    Ok(reached.into_iter().chain(owners(tx, resources)?).collect())
}

/// Whether the newest task keyed `key` failed or was dropped.
fn key_ended(tx: &Transaction, key: Option<&String>) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT 1 FROM tasks WHERE key = ?1 AND state IN ('failed', 'dropped')
         AND id = (SELECT max(id) FROM tasks AS newest WHERE newest.key = ?1)",
        [key],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// The active task that owns any of `resources`: one whose own key is among them, else the oldest.
pub(crate) fn owner(tx: &Transaction, resources: &[String]) -> rusqlite::Result<Option<Task>> {
    Ok(owners(tx, resources)?.into_iter().next())
}

/// Why `task` cannot take `resource`: another task owns it and `task` does not own it yet.
pub(crate) fn taken(
    tx: &Transaction,
    task: i64,
    resource: &str,
) -> rusqlite::Result<Option<String>> {
    let owns = tx
        .query_row(
            "SELECT 1 FROM owned WHERE task = ?1 AND resource = ?2",
            params![task, resource],
            |_| Ok(()),
        )
        .optional()?;
    if owns.is_some() {
        return Ok(None);
    }
    let other = owner(tx, &[resource.to_owned()])?.filter(|other| other.id != task);
    Ok(other.map(|other| format!("{resource} belongs to task {}, not this one", other.id)))
}

/// Every active task that owns any of `resources`, in the order `owner` ranks them. The oldest
/// active batch part of a resource, keyed `<resource>.<part>`, owns it too, and only that part.
/// Subscribes task `id` to the issue, pull request or thread of a request whose `pre` script
/// printed another `key`, so that resource's own events, which name no printed key, still reach it.
fn merged(tx: &Transaction, id: i64, message: &Message) -> rusqlite::Result<()> {
    if message.kind != route::TASK_REQUESTED {
        return Ok(());
    }
    let Some(resource) = message.data["pre"]["key"]["from"]
        .as_str()
        .and_then(crate::subscribe::resource)
    else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO subscriptions (task, resource, created) VALUES (?1, ?2,
           min(strftime('%Y-%m-%dT%H:%M:%SZ', ?3, 'unixepoch'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))",
        params![id, resource, message.time.as_second()],
    )
    .map(drop)
}

fn owners(tx: &Transaction, resources: &[String]) -> rusqlite::Result<Vec<Task>> {
    let resources = serde_json::to_string(resources).expect("resources serialize");
    tx.prepare(
        "SELECT DISTINCT tasks.id, tasks.key IN (SELECT value FROM json_each(?1)) AS keyed
         FROM tasks WHERE tasks.state NOT IN ('done', 'failed', 'dropped')
         AND (tasks.id IN (SELECT task FROM owned WHERE resource IN (SELECT value FROM json_each(?1)))
           OR tasks.id = (SELECT min(part.id) FROM tasks AS part, json_each(?1) AS named
             WHERE part.batch_entry IS NOT NULL
             AND part.state NOT IN ('done', 'failed', 'dropped')
             AND substr(part.key, 1, length(named.value) + 1) = named.value || '.'))
         ORDER BY keyed DESC, tasks.id",
    )?
    .query_map([resources], |row| Ok(Task { id: row.get(0)? }))?
    .collect()
}

/// The newest task in any state that owns `resource`, by key, claim, wait or subscription.
pub fn any_owner(db: &Connection, resource: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT max(task) FROM owned WHERE resource = ?1",
        [resource],
        |row| row.get(0),
    )
}

/// What `priory attach` names: a task, which stands for its live run, or one run.
pub enum Attach {
    Task(i64),
    Run(i64),
}

/// A live run's id, tmux session and the harness session its pane is tagged with.
pub struct Attached {
    pub id: i64,
    pub task: i64,
    pub tmux: String,
    pub session: String,
}

/// The live run `target` names, or why it has none.
pub fn live_run(db: &Connection, target: &Attach) -> Result<Attached, String> {
    let at = |error: rusqlite::Error| error.to_string();
    let live = |row: &rusqlite::Row| {
        Ok(Attached {
            id: row.get(0)?,
            tmux: row.get(1)?,
            session: row.get(2)?,
            task: row.get(3)?,
        })
    };
    match *target {
        Attach::Run(run) => {
            let found = db
                .query_row(
                    "SELECT id, tmux, session, task, ended_as FROM runs WHERE id = ?1",
                    [run],
                    |row| Ok((live(row)?, row.get::<_, Option<String>>(4)?)),
                )
                .optional()
                .map_err(at)?;
            match found {
                None => Err(format!("no run {run}")),
                Some((_, Some(end))) => Err(format!("run {run} ended as {end}")),
                Some((live, None)) => Ok(live),
            }
        }
        Attach::Task(task) => {
            let state: Option<String> = db
                .query_row("SELECT state FROM tasks WHERE id = ?1", [task], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(at)?;
            let Some(state) = state else {
                return Err(format!("no task {task}"));
            };
            db.query_row(
                "SELECT id, tmux, session, task FROM runs WHERE task = ?1 AND ended IS NULL",
                [task],
                live,
            )
            .optional()
            .map_err(at)?
            .ok_or_else(|| format!("task {task} is {state}, with no live run"))
        }
    }
}

/// Why the live run `run` shows no pane to the tmux server this process reaches.
pub fn no_pane(run: &Attached) -> String {
    format!(
        "tmux session {} on this tmux server runs no live pane of run {}'s agent; \
         run attach with the daemon's TMUX and TMUX_TMPDIR",
        run.tmux, run.id
    )
}

/// The rows of `query`, each column as text and `-` for null, tab-separated after `header`.
pub fn print(
    db: &Connection,
    header: &str,
    query: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<()> {
    emit(header);
    let mut statement = db.prepare(query)?;
    let columns = statement.column_count();
    let mut rows = statement.query(params)?;
    while let Some(row) = rows.next()? {
        let cells = (0..columns)
            .map(|i| {
                Ok(match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => "-".to_owned(),
                    rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                    value => value.as_str()?.to_owned(),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        emit(&cells.join("\t"));
    }
    Ok(())
}

/// The state's revision, which every committed change to what a listing shows bumps.
pub fn revision(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("SELECT value FROM revision", [], |row| row.get(0))
}

/// Refuses a person's change made on a read at `expected` once the state moved past it.
pub fn expect_revision(db: &Connection, expected: Option<i64>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let at = revision(db).map_err(|error| error.to_string())?;
    if at != expected {
        return Err(format!("the state is at revision {at}, not {expected}"));
    }
    Ok(())
}

/// The revision once it differs from `since`, or the same one after `timeout`.
pub fn wait(db: &Connection, since: i64, timeout: Duration) -> rusqlite::Result<i64> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let revision = revision(db)?;
        if revision != since || std::time::Instant::now() >= deadline {
            return Ok(revision);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Prints `line`. A reader that closed the pipe, as `head` does, ends the listing without a message.
pub fn emit(line: &str) {
    let Err(error) = writeln!(std::io::stdout(), "{line}") else {
        return;
    };
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("priory: stdout: {error}");
    std::process::exit(1);
}

/// Every trace record, oldest first, as CloudEvents JSON.
pub fn events(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare("SELECT event FROM trace ORDER BY seq")?;
    statement.query_map([], |row| row.get(0))?.collect()
}

/// One event as an agent reads it: type, subject and data on one line.
pub(crate) fn line(kind: &str, subject: Option<&str>, data: &str) -> String {
    format!("{kind} {} {data}", subject.unwrap_or("-"))
}

/// A queued task the daemon may start a run for.
pub(crate) struct Admission {
    pub task: i64,
    pub workflow: String,
    pub step: String,
    pub trigger: String,
    pub repo: Option<String>,
    /// Queued, not ready: its run is the task's first and gets the step's prompt.
    pub fresh: bool,
    /// The head task this task's last run was stopped for, until its next result, ask or wait.
    pub preempted: Option<i64>,
    /// The task's lane, whose `max_running` may hold its start.
    pub lane: Option<String>,
}

/// One pass over the queue.
pub(crate) struct Pass {
    /// The task to start, if any. The pass stops at it.
    pub admission: Option<Admission>,
    /// What live runs and held grants reserved in this pass.
    pub promised: room::Need,
    /// The tasks passed that wait for room, by what their queue's head lacks.
    pub waiting: BTreeMap<&'static str, u64>,
}

/// The first queued or ready task at a model step, in start order, that fits in the runner's room.
/// Each one before it gets the reason it waits.
pub(crate) fn admissible<'r>(
    db: &Connection,
    factory: &Factory,
    runner: Option<&str>,
    share: &Share,
    room: impl Fn() -> &'r Result<Room, String>,
    watched: &dyn Fn(&str) -> bool,
    checking: &dyn Fn(i64) -> bool,
) -> rusqlite::Result<Pass> {
    let mut statement = db.prepare(&format!(
        "SELECT id, workflow, step, trigger, state = 'queued', repo, preempted, lane FROM tasks
             WHERE state IN ('queued', 'ready')
             -- A task waits while a recipe runs in its worktree, or a teardown is owed there.
             AND NOT EXISTS (SELECT 1 FROM recipes WHERE recipes.task = tasks.id
               AND recipes.ended IS NULL
               AND (recipes.started IS NOT NULL OR recipes.moment = 'teardown'))
             -- With several runners, only a task whose key the log gives this one, and not one
             -- whose `kept` it waits to read back. A lead task stays on its ask's runner.
             AND (?1 IS NULL OR tasks.trigger IN (?2, 'priory/consult') OR EXISTS (SELECT 1 FROM claims WHERE claims.key = tasks.key
               AND claims.runner = ?1
               AND NOT (claims.hold != 'kept' AND claims.published IS 'kept')))
             ORDER BY {START_ORDER}"
    ))?;
    let queued = statement
        .query_map(params![runner, crate::tools::LEAD_TRIGGER], |row| {
            Ok(Admission {
                task: row.get(0)?,
                workflow: row.get(1)?,
                step: row.get(2)?,
                trigger: row.get(3)?,
                fresh: row.get(4)?,
                repo: row.get(5)?,
                preempted: row.get(6)?,
                lane: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let promised = promised(db, factory, None)?;
    let paused = paused(db)?;
    let book = crate::spend::Book::new(factory);
    // The first task that waits for room: no task behind it starts, so it is never starved.
    let mut head = None;
    let mut waiting = BTreeMap::new();
    let mut stall = None;
    let mut planned = false;
    for admission in queued {
        let Some(step) = factory
            .workflows
            .get(&admission.workflow)
            .and_then(|workflow| workflow.value.step(&admission.step))
            .filter(|step| step.decides == priory_factory::factory::Decider::Model)
        else {
            continue;
        };
        let helper = [crate::tools::LEAD_TRIGGER, crate::consult::TRIGGER]
            .contains(&admission.trigger.as_str());
        let speaker = match &step.debate {
            Some(debate) => Some(crate::debate::speaker_of(db, admission.task, step, debate)?),
            None => None,
        };
        let agent = speaker
            .as_ref()
            .or(step.agent.as_ref())
            .and_then(|name| Some((name, &factory.agents.get(name)?.value)));
        let busy = match agent {
            Some((name, agent)) if matches!(agent.session, Session::Shared) => serving(db, name)?
                .map(|task| {
                    format!("waits for agent {name}: its shared session serves task {task}")
                })
                .or(crate::idle::compacting(db, name)?.then(|| {
                    format!("waits for agent {name}: its shared session is being compacted")
                })),
            _ => None,
        };
        // A lead's or an investigator's task finishes a task that already ran, so no hold for new
        // work applies to it.
        let new_work = admission.fresh && !helper;
        // A pause holds only new tasks: a woken, resumed or revived one is `ready` and goes on.
        let busy = busy.or_else(|| paused.clone().filter(|_| new_work));
        let busy = match busy {
            Some(reason) => Some(reason),
            None => filtered(db, factory, runner, admission.task)?,
        };
        let busy = match busy {
            Some(reason) => Some(reason),
            None => crate::spend::waits(db, &book, admission.task)?,
        };
        // A check of its agent's claude.ai connectors runs in the background.
        let busy = busy
            .or_else(|| checking(admission.task).then(|| crate::connectors::CHECKING.to_owned()));
        // The review limit, as a pause, holds only a task's first run.
        let busy = match busy {
            Some(reason) => Some(reason),
            None if new_work => {
                let first_band = factory.bands().first().copied();
                crate::review::waits(db, factory.review_limit(), first_band, admission.task)?
            }
            None => None,
        };
        let busy = match busy {
            Some(reason) => Some(reason),
            None if helper => None,
            None => crowded(db, factory, &admission)?,
        };
        let waits = match (busy, head) {
            (Some(reason), _) => Some(reason),
            // The sweeper frees the room the queue waits for, so it never waits for room itself.
            (None, _) if admission.trigger == crate::stall::TRIGGER => None,
            (None, Some((head, cause))) => {
                *waiting.entry(cause).or_default() += 1;
                Some(format!("waits behind task {head}, which waits for room"))
            }
            (None, None) => {
                let need = room::need(agent.map(|(_, agent)| agent));
                match room() {
                    Ok(measured) => {
                        let short = room::wait(measured, share, promised, need);
                        let mut reason = short.as_ref().map(|short| short.reason.clone());
                        // A lead that can never fit never answers: dropping it returns its ask.
                        if let Some(reason) = reason
                            .as_ref()
                            .filter(|_| helper && !room::fits_ever(measured, share, need))
                        {
                            db.execute(
                                "UPDATE tasks SET state = 'dropped', reason = ?2 WHERE id = ?1",
                                params![admission.task, reason],
                            )?;
                            continue;
                        }
                        if let Some(short) = &short {
                            *waiting.entry(short.cause()).or_default() += 1;
                        }
                        // A task that can never fit would hold the queue forever.
                        if let Some(short) = short
                            .as_ref()
                            .filter(|_| room::fits_ever(measured, share, need))
                        {
                            head = Some((admission.task, short.cause()));
                            // Waiting only for live runs to end is backpressure, not a stall.
                            stall = room::wait(measured, share, room::Need::default(), need)
                                .map(|short| (admission.task, short));
                            // Only memory and declared disk are promises a stop gives back.
                            if short.resource != "CPU" {
                                planned = true;
                                let urgent = crate::preempt::Head {
                                    task: admission.task,
                                    need,
                                    measured,
                                    promised,
                                };
                                if let Some(plan) =
                                    crate::preempt::plan(db, factory, share, &urgent, watched)?
                                {
                                    reason = reason.map(|reason| format!("{reason}; {plan}"));
                                }
                            }
                        }
                        reason
                    }
                    Err(error) => {
                        head = Some((admission.task, room::UNMEASURED));
                        *waiting.entry(room::UNMEASURED).or_default() += 1;
                        Some(format!("waits for room: cannot measure it: {error}"))
                    }
                }
            }
        };
        let Some(mut reason) = waits else {
            return Ok(Pass {
                admission: Some(admission),
                promised,
                waiting,
            });
        };
        // A stopped task waits for the room its stop made; its own plan, if any, says more.
        if let Some(head) = admission.preempted
            && (reason.starts_with("waits behind")
                || reason.starts_with("waits for room") && !reason.contains("; "))
        {
            let (band, _) = crate::preempt::prompt(db, factory, head)?;
            reason = format!("stopped for task {head} ({band}); resumes when there is room");
        }
        db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
            params![admission.task, reason],
        )?;
    }
    if !planned {
        crate::preempt::withdraw(db)?;
    }
    crate::stall::record(db, stall.as_ref().map(|(task, short)| (*task, short)))?;
    Ok(Pass {
        admission: None,
        promised,
        waiting,
    })
}

/// Why this runner's filters hold `task` now, if they do. They hold only a task that never ran a
/// step, since one that started stays on its runner, and never a lead's or an investigator's. A
/// factory of one runner has no runner name, so its one runner file applies.
pub(crate) fn filtered(
    db: &Connection,
    factory: &Factory,
    runner: Option<&str>,
    task: i64,
) -> rusqlite::Result<Option<String>> {
    let Some(runner) = runner.or_else(|| match factory.runners.keys().collect::<Vec<_>>()[..] {
        [one] => Some(one.as_str()),
        _ => None,
    }) else {
        return Ok(None);
    };
    let (labels, started): (Option<String>, bool) = db.query_row(
        "SELECT labels, trigger IN (?2, ?3) OR EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id)
           OR EXISTS (SELECT 1 FROM actions WHERE actions.task = tasks.id)
         FROM tasks WHERE id = ?1",
        params![task, crate::tools::LEAD_TRIGGER, crate::consult::TRIGGER],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if started {
        return Ok(None);
    }
    let labels = labels
        .and_then(|labels| serde_json::from_str(&labels).ok())
        .unwrap_or_default();
    let now = priory_intake::intake::now().unwrap_or_else(|_| jiff::Timestamp::now());
    Ok(crate::runners::refusal(factory, runner, &labels, now))
}

/// Why a task waits for its lane's or trigger's `max_running`: `waits: lane core/intake has 2
/// running tasks, its max_running`. A run counts from its start to its end.
fn crowded(
    db: &Connection,
    factory: &Factory,
    admission: &Admission,
) -> rusqlite::Result<Option<String>> {
    let lane = admission.lane.as_deref().and_then(|lane| {
        let max = factory.lanes().lane.get(lane)?.max_running?;
        Some(("lane", lane, "tasks.lane", max))
    });
    let trigger = factory
        .triggers
        .get(&admission.trigger)
        .and_then(|trigger| trigger.value.max_running)
        .map(|max| ("trigger", admission.trigger.as_str(), "tasks.trigger", max));
    for (kind, name, column, max) in lane.into_iter().chain(trigger) {
        let running: i64 = db.query_row(
            &format!(
                "SELECT count(*) FROM runs JOIN tasks ON tasks.id = runs.task
                 WHERE runs.ended IS NULL AND {column} = ?1"
            ),
            [name],
            |row| row.get(0),
        )?;
        if running >= i64::from(max) {
            let tasks = if running == 1 { "task" } else { "tasks" };
            return Ok(Some(format!(
                "waits: {kind} {name} has {running} running {tasks}, its max_running"
            )));
        }
    }
    Ok(None)
}

/// Pauses admissions with `reason`: no queued task starts until `unpause`. A second pause keeps
/// the first one's start and takes the new reason.
pub fn pause(db: &Connection, reason: Option<&str>) -> rusqlite::Result<String> {
    db.execute(
        &format!(
            "INSERT INTO pause (id, reason, since) VALUES (1, ?1, {NOW})
             ON CONFLICT (id) DO UPDATE SET reason = ?1"
        ),
        [reason],
    )?;
    Ok("priory is paused: no queued task starts until `priory unpause`".to_owned())
}

/// Ends the pause, so queued tasks start again.
pub fn unpause(db: &Connection) -> rusqlite::Result<String> {
    Ok(match db.execute("DELETE FROM pause", [])? {
        0 => "priory was not paused".to_owned(),
        _ => "priory is unpaused".to_owned(),
    })
}

/// Why a queued task waits while admissions are paused: `waits: priory is paused[: <reason>]`.
fn paused(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT 'waits: priory is paused' || coalesce(': ' || reason, '') FROM pause",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// The socket line of `POST /api/seen`: `{"seen": {"person": ..., "at": ...}}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeenLine {
    pub seen: Visit,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visit {
    pub person: String,
    pub at: jiff::Timestamp,
}

/// Records that `person` looked at the desk at `at`, so its Since you looked starts there. Whole
/// seconds, as inbox times are: the desk lists events from the visit's second on, so an event in
/// that second shows again rather than never.
pub fn seen(db: &Connection, person: &str, at: jiff::Timestamp) -> rusqlite::Result<String> {
    let at = at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    db.execute(
        "INSERT INTO visits (person, seen) VALUES (?1, ?2)
         ON CONFLICT (person) DO UPDATE SET seen = ?2",
        [person, &at],
    )?;
    Ok(format!("{person} looked at {at}"))
}

/// The desk's line while admissions are paused: `paused since <since>[: <reason>]`.
pub fn pause_banner(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT 'paused since ' || since || coalesce(': ' || reason, '') FROM pause",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// The task whose live run holds `agent`'s shared session, which runs one turn at a time.
fn serving(db: &Connection, agent: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT task FROM runs WHERE agent = ?1 AND ended IS NULL",
        [agent],
        |row| row.get(0),
    )
    .optional()
}

/// The memory and disk the agents of live runs declared and do not use yet, except those of task
/// `except`, plus every held memory grant in full. A run's use is its scope's memory, which measured
/// free memory already leaves out. A run with no scope keeps its whole declaration until it ends.
pub(crate) fn promised(
    db: &Connection,
    factory: &Factory,
    except: Option<i64>,
) -> rusqlite::Result<room::Need> {
    let mut live = Vec::new();
    let promised = promised_with(db, factory, except, |run, tmux, session| {
        live.push((run, session.to_owned()));
        crate::usage::memory(run, tmux, session)
    })?;
    if except.is_none() {
        crate::usage::keep(&live);
    }
    Ok(promised)
}

/// `promised`, with `used` measuring what a live run, its tmux session and harness session, uses.
fn promised_with(
    db: &Connection,
    factory: &Factory,
    except: Option<i64>,
    mut used: impl FnMut(i64, &str, &str) -> Option<u64>,
) -> rusqlite::Result<room::Need> {
    let held: i64 = db.query_row(
        "SELECT coalesce(sum(bytes), 0) FROM grants WHERE state = 'held'",
        [],
        |row| row.get(0),
    )?;
    let mut statement = db.prepare(
        "SELECT id, agent, tmux, session FROM runs WHERE ended IS NULL AND task IS NOT ?1",
    )?;
    // A run recorded before schema 5 has no agent.
    let mut runs = statement.query_map([except], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    // A granted command may not have allocated yet, so a held grant counts in full.
    let held = room::Need {
        memory: held as u64,
        disk: 0,
    };
    runs.try_fold(held, |sum, run| {
        let (id, agent, tmux, session) = run?;
        let agent = agent.and_then(|name| factory.agents.get(&name));
        let need = room::need(agent.map(|agent| &agent.value));
        Ok(sum.plus(room::unused(need, used(id, &tmux, &session))))
    })
}

/// A task at a code step, whose action the daemon runs.
pub(crate) struct Due {
    pub task: i64,
    pub workflow: String,
    pub step: String,
    pub trigger: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

/// The queued or ready tasks at a code step, oldest first.
pub(crate) fn actionable(
    db: &Connection,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> rusqlite::Result<Vec<Due>> {
    let mut statement = db.prepare(
        "SELECT id, workflow, step, trigger, branch, worktree, repo FROM tasks
         WHERE state = 'ready' OR state = 'queued' AND NOT EXISTS (SELECT 1 FROM pause)
         ORDER BY id",
    )?;
    let tasks = statement.query_map([], |row| {
        Ok(Due {
            task: row.get(0)?,
            workflow: row.get(1)?,
            step: row.get(2)?,
            trigger: row.get(3)?,
            branch: row.get(4)?,
            worktree: row.get(5)?,
            repo: row.get(6)?,
        })
    })?;
    let mut due = Vec::new();
    for task in tasks {
        let task = task?;
        let code = workflows
            .get(&task.workflow)
            .and_then(|workflow| workflow.value.step(&task.step))
            .is_some_and(|step| step.decides == priory_factory::factory::Decider::Code);
        if code {
            due.push(task);
        }
    }
    Ok(due)
}

/// Records the process group of `run` action `id` and its start time, so a restarted daemon kills
/// it and never a process that reused its pid.
pub(crate) fn command_spawned(
    db: &Connection,
    id: i64,
    pid: u32,
    started: Option<i64>,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE actions SET pid = ?2, pid_started = ?3 WHERE id = ?1",
        params![id, pid, started],
    )?;
    Ok(())
}

/// Whether action `id`'s command is still wanted. A `run` action is its task's current attempt: the
/// task runs the action's step, and no later action exists, as one a cancel and revive would start.
/// An answer's command belongs to the task's generation it was answered in, while the task is active,
/// whatever the task does meanwhile.
pub(crate) const CURRENT_COMMAND: &str = "SELECT CASE actions.action
     WHEN 'answer' THEN tasks.generation IS actions.generation
       AND tasks.state NOT IN ('done', 'failed', 'dropped')
     ELSE tasks.state = 'running'
       AND tasks.step = actions.step
       AND actions.id = (SELECT max(id) FROM actions AS later WHERE later.task = actions.task) END
   FROM actions JOIN tasks ON tasks.id = actions.task WHERE actions.id = ?1";

pub(crate) fn command_wanted(db: &Connection, id: i64) -> rusqlite::Result<bool> {
    db.query_row(CURRENT_COMMAND, [id], |row| row.get(0))
}

/// The answers whose command waits to start: stored, never spawned, so never run.
pub(crate) fn unstarted_answers(db: &Connection) -> rusqlite::Result<Vec<(i64, i64, String)>> {
    let mut statement = db.prepare(
        "SELECT id, task, command FROM actions
         WHERE action = 'answer' AND ended IS NULL AND pid IS NULL ORDER BY id",
    )?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect()
}

/// The answer commands a stopped daemon left running, each with its process group and that
/// group's start time when recorded.
pub(crate) fn orphan_answers(
    db: &Connection,
) -> rusqlite::Result<Vec<(i64, Option<crate::stall::Group>)>> {
    let mut statement = db.prepare(
        "SELECT id, pid, pid_started FROM actions
         WHERE action = 'answer' AND ended IS NULL AND pid IS NOT NULL",
    )?;
    statement
        .query_map([], |row| {
            let pid: Option<u32> = row.get(1)?;
            Ok((row.get(0)?, pid.zip(row.get(2)?)))
        })?
        .collect()
}

/// The `run` actions a stopped daemon left running, each with its process group and that group's
/// start time when both were recorded.
pub(crate) fn orphan_commands(
    db: &Connection,
) -> rusqlite::Result<Vec<(i64, Option<crate::stall::Group>)>> {
    let mut statement = db.prepare(
        "SELECT id, pid, pid_started FROM actions WHERE action = 'run' AND ended IS NULL",
    )?;
    statement
        .query_map([], |row| {
            let pid: Option<u32> = row.get(1)?;
            Ok((row.get(0)?, pid.zip(row.get(2)?)))
        })?
        .collect()
}

/// What the latest result of a task says for an action to describe its pull request or issue.
pub(crate) struct Described {
    pub summary: String,
    pub evidence: Option<String>,
    /// The branches of the stack to open, bottom first; none for the task's branch alone.
    pub branches: Option<Vec<String>>,
}

/// The summary, evidence and stack branches of the task's latest result.
pub(crate) fn last_result(db: &Connection, task: i64) -> rusqlite::Result<Option<Described>> {
    db.query_row(
        "SELECT summary, evidence, result_branches.branches FROM results
         LEFT JOIN result_branches USING (run) WHERE task = ?1 ORDER BY run DESC LIMIT 1",
        [task],
        |row| {
            let branches: Option<String> = row.get(2)?;
            Ok(Described {
                summary: row.get(0)?,
                evidence: row.get(1)?,
                branches: branches
                    .map(|branches| serde_json::from_str(&branches).expect("branches are JSON")),
            })
        },
    )
    .optional()
}

/// What a new run needs recorded before its tmux session starts.
pub(crate) struct NewRun<'a> {
    pub task: i64,
    pub step: &'a str,
    pub agent: &'a str,
    pub tmux: &'a str,
    pub session: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub worktree: &'a str,
    /// The step's prompt, for a fresh task. A ready task has its prompt queued.
    pub prompt: Option<&'a str>,
    /// How the step ends, which every prompt that wakes the task repeats.
    pub ending: &'a str,
    /// The session is the agent's shared one, which other tasks' runs resume too.
    pub shared: bool,
    /// What the agent's shared session starts with, which a later run compares its own against.
    pub definition: &'a str,
    /// The line telling a resumed shared session what changed in its agent.
    pub changed: Option<&'a str>,
    /// Derives the lane of a task again, once, now that its branch is known.
    pub lanes: &'a Lanes,
    /// A hash of the factory's skills and instructions the run gets, if any.
    pub kit_hash: Option<&'a str>,
    /// The kit commit the run gets, if any.
    pub kit_commit: Option<&'a str>,
    /// The debate the run takes a turn of. Its prompt carries no kept event: debaters judge, and
    /// the next step reads the events.
    pub debate: Option<i64>,
    /// The memories repository's commit the run gets, if any.
    pub memories_commit: Option<&'a str>,
    /// The harness that runs it, as `harness` in the agent's file names it.
    pub harness: &'a str,
    /// It resumes a session rather than starting one.
    pub resumes: bool,
    /// The sandbox of the task's workflow its agent starts in, if any.
    pub sandbox: Option<&'a str>,
}

/// The agent's shared session, once a run of it reached `SessionStart`, which every task resumes.
pub(crate) fn shared(db: &Connection, agent: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT session FROM shared_sessions WHERE agent = ?1
         AND EXISTS (SELECT 1 FROM runs WHERE runs.session = shared_sessions.session AND ready = 1)",
        [agent],
        |row| row.get(0),
    )
    .optional()
}

/// The harness session of the task's latest run of `agent` that Claude started, which a new run of
/// that agent resumes.
pub(crate) fn resumable(
    db: &Connection,
    task: i64,
    agent: &str,
) -> rusqlite::Result<Option<String>> {
    db.query_row(
        // A session the agent once shared stays shared, so two tasks cannot both resume it, even
        // after a harness change gave the agent another shared session.
        "SELECT session FROM runs WHERE task = ?1 AND agent = ?2 AND ready = 1 AND debate IS NULL
         AND session NOT IN (SELECT session FROM shared_sessions)
         AND session NOT IN (SELECT session FROM runs GROUP BY session HAVING count(DISTINCT task) > 1)
         ORDER BY id DESC LIMIT 1",
        params![task, agent],
        |row| row.get(0),
    )
    .optional()
}

/// What the shared `session` started with, if it was recorded.
pub(crate) fn definition(db: &Connection, session: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT definition FROM shared_sessions WHERE session = ?1",
        [session],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// The harness's own id for `session`, when it differs from Priory's, as Codex's thread id does.
pub(crate) fn thread(db: &Connection, session: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT thread FROM runs WHERE session = ?1 AND thread IS NOT NULL ORDER BY id DESC LIMIT 1",
        [session],
        |row| row.get(0),
    )
    .optional()
}

/// Records a run and its first prompt: the step's prompt, if any, then the task's source event and
/// every event it kept that no prompt carried yet. Returns the run id.
pub(crate) fn start(db: &mut Connection, run: &NewRun) -> rusqlite::Result<i64> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO runs (task, step, agent, tmux, session, kit_hash, kit_commit, memories_commit,
           debate, harness, resumes, sandbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            run.task,
            run.step,
            run.agent,
            run.tmux,
            run.session,
            run.kit_hash,
            run.kit_commit,
            run.memories_commit,
            run.debate,
            run.harness,
            run.resumes,
            run.sandbox
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE tasks SET state = 'running', branch = ?2, worktree = ?3, reason = NULL, ending = ?4,
           retired = NULL
         WHERE id = ?1",
        params![run.task, run.branch, run.worktree, run.ending],
    )?;
    let (key, team, project, derived): (String, Option<String>, Option<String>, bool) = tx
        .query_row(
            "SELECT key, team, project, lane_derived FROM tasks WHERE id = ?1",
            [run.task],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if derived {
        let lane = run.lanes.lane(&Subject {
            key: &key,
            team: team.as_deref(),
            project: project.as_deref(),
            branch: Some(run.branch),
        });
        tx.execute(
            "UPDATE tasks SET lane = ?2, lane_derived = 0 WHERE id = ?1",
            params![run.task, lane],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
        params![run.task, route::branch(run.repo, run.branch)],
    )?;
    let mut text = run.prompt.unwrap_or_default().to_owned();
    let unread = match run.debate {
        Some(_) => Vec::new(),
        None => unread(&tx, run.task, true, None)?,
    };
    for (_, _, line) in &unread {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
    }
    let events: Vec<i64> = unread.iter().map(|(seq, _, _)| *seq).collect();
    if crate::triage::any(&tx, &events)? {
        text = format!("{text}\n{}", crate::triage::RULE);
    }
    let held = match run.debate {
        Some(_) => Vec::new(),
        None => held(&tx, run.task)?,
    };
    for (_, said) in &held {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(said);
    }
    // The step after a debate reads what came during it with the entry prompt the debate queued.
    let entry: Option<i64> = match run.prompt {
        None if !text.is_empty() => tx.query_row(
            "SELECT min(outbox.id) FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0
                 AND (SELECT debate FROM runs WHERE task = ?1 AND id < ?2 ORDER BY id DESC LIMIT 1)
                   IS NOT NULL",
            params![run.task, id],
            |row| row.get(0),
        )?,
        _ => None,
    };
    if !text.is_empty() {
        let prompt = match entry {
            Some(entry) => {
                tx.execute(
                    "UPDATE outbox SET text = text || char(10) || ?2 WHERE id = ?1",
                    params![entry, text],
                )?;
                entry
            }
            None => {
                tx.execute(
                    "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
                    params![run.task, text],
                )?;
                tx.last_insert_rowid()
            }
        };
        carry(&tx, run.task, prompt, &unread)?;
        for (id, _) in &held {
            tx.execute(
                "UPDATE inbox SET outbox = ?1 WHERE outbox = ?2",
                params![prompt, id],
            )?;
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
    }
    if run.shared {
        share(&tx, run)?;
    }
    tx.commit()?;
    Ok(id)
}

/// The texts a person said to `task` during a debate, by outbox id, which its next run after the
/// debate reads with its first prompt.
fn held(tx: &Transaction, task: i64) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut statement =
        tx.prepare("SELECT id, text FROM outbox WHERE task = ?1 AND state = 'held' ORDER BY id")?;
    statement
        .query_map([task], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// Each event `task` kept that no prompt carries, by seq, with whether a subscription delivered it
/// (its inbox entry belongs to another task) and its line: its own kept events, those a
/// subscription kept for it, and with `source` its source event, leaving out `except`.
pub(crate) fn unread(
    tx: &Transaction,
    task: i64,
    source: bool,
    except: Option<i64>,
) -> rusqlite::Result<Vec<(i64, bool, String)>> {
    let mut statement = tx.prepare(
        "SELECT seq, 0, type, subject, data, id = (SELECT source FROM tasks WHERE id = ?1)
         FROM inbox WHERE outbox IS NULL AND seq IS NOT ?3
           AND ((task = ?1 AND outcome = 'kept')
             OR (?2 AND id = (SELECT source FROM tasks WHERE id = ?1)))
         UNION ALL
         SELECT inbox.seq, 1, inbox.type, inbox.subject, inbox.data, 0
         FROM subscription_deliveries AS heard JOIN inbox ON inbox.seq = heard.seq
         WHERE heard.task = ?1 AND heard.outcome = 'kept' AND heard.outbox IS NULL
           AND heard.seq IS NOT ?3
         ORDER BY 1",
    )?;
    let mut rows = statement.query(params![task, source, except])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let (kind, subject) = (row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?);
        let data = row.get::<_, String>(4)?;
        let line = match row.get::<_, bool>(5)? {
            true => requested(tx, task, &kind, subject.as_deref(), &data)?,
            false => line(&kind, subject.as_deref(), &data),
        };
        events.push((row.get(0)?, row.get(1)?, line));
    }
    Ok(events)
}

/// The line of `task`'s source event, with the title and text its trigger's `pre` script gave in
/// place of the event's, and the context it added after it.
fn requested(
    tx: &Transaction,
    task: i64,
    kind: &str,
    subject: Option<&str>,
    data: &str,
) -> rusqlite::Result<String> {
    let (title, text, context, by): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = tx.query_row(
        "SELECT pre_title, pre_text, pre_context, pre_script FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut said = match (
        &title,
        &text,
        serde_json::from_str::<serde_json::Value>(data),
    ) {
        (None, None, _) | (_, _, Err(_)) => line(kind, subject, data),
        (_, _, Ok(mut value)) => {
            if let Some(title) = title {
                value["title"] = serde_json::json!(title);
            }
            if let Some(text) = text {
                let field = ["text", "body", "description", "prompt"]
                    .into_iter()
                    .find(|field| value[*field].is_string())
                    .unwrap_or("text");
                value[field] = serde_json::json!(text);
            }
            line(kind, subject, &value.to_string())
        }
    };
    if let Some(context) = context {
        let by = by.unwrap_or_default();
        said.push_str(&format!(
            "\nContext from the trigger's pre script {by}:\n{context}"
        ));
    }
    said.push_str(&intent(tx, task)?);
    Ok(said)
}

/// The lines of the brief that carry the intent and the no-op cases of `task`'s trigger, if it
/// named any.
fn intent(tx: &Transaction, task: i64) -> rusqlite::Result<String> {
    let (intent, noop): (Option<String>, Option<String>) = tx.query_row(
        "SELECT intent, noop FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut said = String::new();
    if let Some(intent) = intent {
        said.push_str(&format!("\nThe trigger's intent: {intent}"));
    }
    let noop: Vec<String> = noop
        .and_then(|noop| serde_json::from_str(&noop).ok())
        .unwrap_or_default();
    if !noop.is_empty() {
        said.push_str(
            "\nIf one of these cases holds, raise no ask and write nothing: end the step with a result that names the case.",
        );
        for case in noop {
            said.push_str(&format!("\n- {case}"));
        }
    }
    Ok(said)
}

/// Records that prompt `outbox` carries the `unread` events of `task`.
pub(crate) fn carry(
    tx: &Transaction,
    task: i64,
    outbox: i64,
    unread: &[(i64, bool, String)],
) -> rusqlite::Result<()> {
    for (seq, heard, _) in unread {
        if *heard {
            tx.execute(
                "UPDATE subscription_deliveries SET outbox = ?1 WHERE seq = ?2 AND task = ?3",
                params![outbox, seq, task],
            )?;
        } else {
            tx.execute(
                "UPDATE inbox SET outbox = ?1 WHERE seq = ?2",
                params![outbox, seq],
            )?;
        }
    }
    Ok(())
}

/// Records the run's session as its agent's shared one, and names the task in its first prompt.
fn share(tx: &Transaction, run: &NewRun) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO shared_sessions (agent, session, definition) VALUES (?1, ?2, ?3)
         ON CONFLICT (agent) DO UPDATE SET session = excluded.session, definition = excluded.definition",
        params![run.agent, run.session, run.definition],
    )?;
    let (key, trigger): (String, String) = tx.query_row(
        "SELECT key, trigger FROM tasks WHERE id = ?1",
        [run.task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let intro = format!(
        "This is task {} ({key}), from trigger {trigger}, in the worktree {}. Earlier turns of this conversation may belong to other tasks.",
        run.task, run.worktree
    );
    let intro = match run.changed {
        Some(changed) => format!("{changed}\n{intro}"),
        None => intro,
    };
    let prepended = tx.execute(
        "UPDATE outbox SET text = ?2 || char(10) || text WHERE id =
         (SELECT min(id) FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0)",
        params![run.task, intro],
    )?;
    if prepended == 0 {
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![run.task, intro],
        )?;
    }
    Ok(())
}

/// Blocks `task` with `reason`, ending its live run first as `ended_as`, if it has one, after reading
/// its transcript.
pub(crate) fn block(
    db: &mut Connection,
    trace: &Trace,
    task: i64,
    ended_as: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let live = tx
        .query_row(
            "SELECT id FROM runs WHERE task = ?1 AND ended IS NULL",
            [task],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(run) = live {
        // The step goes on after a resume, so the turn cut short still counts toward it.
        crate::trace::account(&tx, trace, run, None)?;
        // A run that never reached SessionStart never took the prompt it was launched with.
        tx.execute(
            "UPDATE outbox SET state = 'queued', run = NULL, attempted = NULL
             WHERE run = ?1 AND state IN ('attempted', 'uncertain') AND (SELECT NOT ready FROM runs WHERE id = ?1)",
            [run],
        )?;
    }
    tx.execute(
        &format!("UPDATE runs SET ended = {NOW}, ended_as = ?2 WHERE task = ?1 AND ended IS NULL"),
        params![task, ended_as],
    )?;
    // A run that started without its `.envrc` keeps saying so, since that may be why it stopped.
    tx.execute(
        "UPDATE tasks SET state = 'blocked', reason = ?2 || coalesce('; ' || CASE
           WHEN (SELECT direnv FROM runs WHERE id = ?3) = 2 THEN reason END, '')
         WHERE id = ?1",
        params![task, reason, live],
    )?;
    tx.commit()
}

/// How many tasks are queued or ready to start a run.
pub(crate) fn waiting(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row(
        "SELECT count(*) FROM tasks WHERE state IN ('queued', 'ready')",
        [],
        |row| row.get(0),
    )
}

pub(crate) struct LiveRun {
    pub id: i64,
    pub task: i64,
    pub step: String,
    pub agent: String,
    pub workflow: String,
    pub tmux: String,
    pub session: String,
    pub worktree: Option<String>,
    /// Its `SessionStart` arrived.
    pub ready: bool,
    /// No `SessionStart` arrived within `READY_WITHIN` of the start, or `DIRENV_READY_WITHIN` under
    /// direnv.
    pub late: bool,
    /// 1 when it started under `direnv exec`, 2 once direnv failed and it started without, else 0.
    pub direnv: i64,
    /// How long its current turn has run, in seconds.
    pub turn: Option<i64>,
    /// Claude's message for the permission prompt it shows, if any.
    pub prompted: Option<String>,
}

pub(crate) const READY_WITHIN: &str = "60 seconds";

/// `READY_WITHIN` for a run under direnv, whose `.envrc` may build a cold nix shell first.
pub(crate) const DIRENV_READY_WITHIN: &str = "600 seconds";

/// Marks run `run` as started under direnv (1), or as started without its `.envrc` after direnv
/// failed (2).
pub(crate) fn direnv(db: &Connection, run: i64, state: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE runs SET direnv = ?2 WHERE id = ?1",
        params![run, state],
    )?;
    Ok(())
}

pub(crate) fn live(db: &Connection) -> rusqlite::Result<Vec<LiveRun>> {
    let mut statement = db.prepare(&format!(
        "SELECT runs.id, runs.task, runs.step, tasks.workflow, runs.tmux, runs.session, tasks.worktree,
         NOT runs.ready AND runs.started <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now',
           CASE WHEN runs.direnv THEN '-{DIRENV_READY_WITHIN}' ELSE '-{READY_WITHIN}' END),
         unixepoch('now') - unixepoch(runs.turn), runs.prompted, coalesce(runs.agent, ''), runs.ready,
         coalesce(runs.direnv, 0)
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL ORDER BY runs.id"
    ))?;
    statement
        .query_map([], |row| {
            Ok(LiveRun {
                id: row.get(0)?,
                task: row.get(1)?,
                step: row.get(2)?,
                workflow: row.get(3)?,
                tmux: row.get(4)?,
                session: row.get(5)?,
                worktree: row.get(6)?,
                late: row.get(7)?,
                turn: row.get(8)?,
                prompted: row.get(9)?,
                agent: row.get(10)?,
                ready: row.get(11)?,
                direnv: row.get(12)?,
            })
        })?
        .collect()
}

/// A signal from a run's harness, as `priory hook` sends it on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Signal {
    pub event: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The session transcript Claude names in every hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// The harness's own session id, when it chose one apart from `session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// Why `SessionStart` fired, such as `startup` or `compact`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Applies a hook signal: the run is idle after `SessionStart` or `Stop`, and busy after
/// `UserPromptSubmit`, which also confirms its attempted prompt when the submitted text matches.
/// After `Stop`, the task moves on by the run's result or ask. Returns a tmux session to kill.
pub(crate) fn signal(
    db: &mut Connection,
    signal: &Signal,
    trace: &Trace,
    watched: &dyn Fn(&str) -> bool,
) -> rusqlite::Result<Option<String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = tx
        .query_row(
            "SELECT id FROM runs WHERE session = ?1 AND ended IS NULL",
            [&signal.session],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    else {
        let kill = crate::idle::signal(&tx, signal)?;
        tx.commit()?;
        return Ok(kill);
    };
    tx.execute(
        &format!("UPDATE runs SET signaled = {NOW} WHERE id = ?1"),
        [run],
    )?;
    let mut kill = None;
    match signal.event.as_str() {
        // Claude also fires it for its own compaction mid-turn; Codex 0.170 only with the next turn.
        "SessionStart" | "PostCompact"
            if signal.event == "PostCompact" || signal.source.as_deref() == Some("compact") =>
        {
            crate::tools::compacted(&tx, run)?;
        }
        "SessionStart" => {
            tx.execute(
                "UPDATE runs SET idle = 1, ready = 1, turn = NULL, nudgeable = 0, thread = coalesce(?2, thread),
                   session_started = coalesce(session_started, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 WHERE id = ?1",
                params![run, signal.thread],
            )?;
            let context = crate::trace::begin(&tx, trace, run, signal.transcript.as_deref())?;
            // A resumed session may already be past `compactAt`, as when every task ends in one result.
            crate::tools::overflow(&tx, &signal.session, context, trace.factory)?;
        }
        "Stop" => {
            tx.execute("UPDATE runs SET idle = 1, turn = NULL WHERE id = ?1", [run])?;
            let context = crate::trace::account(&tx, trace, run, signal.transcript.as_deref())?;
            // A run marked for an urgent wake ends here; its resumed session compacts instead.
            if let Some(head) = crate::preempt::due(&tx, run, watched)? {
                kill = crate::tools::preempt(&tx, &signal.session, trace, head)?;
            // The run settles its ask or wait once its compact is typed, so it never sleeps past it.
            // A result carried into the next step on this process compacts there. A run whose gate
            // grades waits, idle, for the verdict, which settles it.
            } else if crate::gates::grading(&tx, run)?.is_some() {
                kill = crate::tools::held_stop(&tx, &signal.session, trace)?;
            } else if !crate::tools::overflow(&tx, &signal.session, context, trace.factory)? {
                kill = crate::tools::stop(&tx, &signal.session, trace)?;
                crate::tools::overflow(&tx, &signal.session, context, trace.factory)?;
            }
            tx.execute("UPDATE runs SET nudgeable = 0 WHERE id = ?1", [run])?;
        }
        "UserPromptSubmit" => {
            tx.execute(
                &format!("UPDATE runs SET idle = 0, turn = {NOW}, nudgeable = 0 WHERE id = ?1"),
                [run],
            )?;
            let attempted = tx
                .query_row(
                    "SELECT id, text, nudge FROM outbox WHERE run = ?1 AND state IN ('attempted', 'uncertain')
                     AND compact = 0",
                    [run],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((id, text, nudge)) = attempted
                && signal.prompt.as_deref().is_some_and(|prompt| {
                    pasted(prompt)
                        .split_whitespace()
                        .eq(text.split_whitespace())
                })
            {
                tx.execute(
                    &format!(
                        "UPDATE outbox SET state = 'confirmed', confirmed = {CONFIRMED} WHERE id = ?1"
                    ),
                    [id],
                )?;
                tx.execute(
                    "UPDATE runs SET nudgeable = ?2 WHERE id = ?1",
                    params![run, !nudge],
                )?;
            }
        }
        "PermissionPrompt" => {
            tx.execute(
                "UPDATE runs SET prompted = coalesce(?2, 'a permission prompt') WHERE id = ?1",
                params![run, signal.message],
            )?;
        }
        _ => {}
    }
    tx.commit()?;
    Ok(kill)
}

/// The prompt inside the `pasted_content` tag that Claude Code 2.1.328 wraps a paste in, before its
/// hook sees it; `prompt` itself when it has no such tag.
fn pasted(prompt: &str) -> &str {
    prompt
        .trim()
        .strip_prefix("<pasted_content")
        .and_then(|rest| rest.split_once('>'))
        .and_then(|(attributes, body)| body.strip_suffix(&format!("</pasted_content{attributes}>")))
        .unwrap_or(prompt)
}

/// A prompt marked `attempted`, to type into its run's tmux session.
pub(crate) struct Attempt {
    pub outbox: i64,
    pub tmux: String,
    pub session: String,
    pub text: String,
    pub compact: bool,
}

/// How long after its `SessionStart` a run first gets a prompt. Claude fires the hook before its
/// input reads keys, and takes keys that wait for it, an Enter among them, as one paste.
const SETTLE: &str = "1 seconds";

/// Marks the oldest queued prompt of an idle live run `attempted` and the run busy, in one
/// transaction, before anything is typed. A task with an attempted or uncertain prompt gets none
/// while the process it went to may still hold its text: its run is live, as a run carried into the
/// next step takes over the prompts of the run it continues. A run whose keyboard a person holds
/// gets none until the grant ends, and a run gets none within `SETTLE` of its `SessionStart`.
pub(crate) fn attempt(db: &mut Connection) -> rusqlite::Result<Option<Attempt>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = tx
        .query_row(
            &format!("SELECT outbox.id, runs.id, runs.tmux, runs.session, outbox.text, outbox.compact FROM runs
             JOIN outbox ON outbox.task = runs.task AND outbox.state = 'queued'
             WHERE runs.ended IS NULL AND runs.idle = 1
             AND (runs.session_started IS NULL
               OR julianday(runs.session_started) <= julianday('now', '-{SETTLE}'))
             AND NOT EXISTS (SELECT 1 FROM gates WHERE gates.run = runs.id AND gates.state = 'grading')
             AND NOT EXISTS (
               SELECT 1 FROM outbox AS open JOIN runs AS typed ON typed.id = open.run
               WHERE open.task = runs.task AND open.state IN ('attempted', 'uncertain')
               AND typed.ended IS NULL)
             AND NOT EXISTS (SELECT 1 FROM keyboard_grants AS held WHERE held.run = runs.id
               AND held.ended IS NULL AND julianday(held.ends) > julianday('now'))
             ORDER BY outbox.id LIMIT 1"),
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(1)?,
                    Attempt {
                        outbox: row.get(0)?,
                        tmux: row.get(2)?,
                        session: row.get(3)?,
                        text: row.get(4)?,
                        compact: row.get(5)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((run, attempt)) = next else {
        return Ok(None);
    };
    let outbox = attempt.outbox;
    tx.execute(
        &format!(
            "UPDATE outbox SET state = 'attempted', run = ?2, attempted = {NOW} WHERE id = ?1"
        ),
        params![outbox, run],
    )?;
    // No UserPromptSubmit may stamp a compaction's turn, so its step timeout starts here.
    tx.execute(
        &format!(
            "UPDATE runs SET idle = 0,
             turn = CASE WHEN (SELECT compact FROM outbox WHERE id = ?2) THEN {NOW} ELSE turn END
             WHERE id = ?1"
        ),
        [run, outbox],
    )?;
    tx.commit()?;
    Ok(Some(attempt))
}

/// Whether the attempted prompt `outbox` still waits for its `UserPromptSubmit`: it is still
/// `attempted`, its run is live, and no prompt started that run's turn since. A `/compact` stamps
/// its turn when attempted and never fires one, so it never waits.
pub(crate) fn unsubmitted(db: &Connection, outbox: i64) -> rusqlite::Result<bool> {
    Ok(db
        .query_row(
            "SELECT outbox.state = 'attempted' AND runs.ended IS NULL AND runs.turn IS NULL
               AND NOT outbox.compact
             FROM outbox JOIN runs ON runs.id = outbox.run WHERE outbox.id = ?1",
            [outbox],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Marks the attempted prompt `outbox` `uncertain`: Enter was pressed twice and no
/// `UserPromptSubmit` came, so its text may still sit in the agent's input.
pub(crate) fn unconfirmed(db: &Connection, outbox: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE outbox SET state = 'uncertain' WHERE id = ?1 AND state = 'attempted'",
        [outbox],
    )?;
    Ok(())
}

/// Marks the task's oldest queued prompt `attempted` by `run`, which takes it as its first prompt
/// at launch, and returns its text.
pub(crate) fn claim(db: &mut Connection, run: i64) -> rusqlite::Result<Option<String>> {
    db.query_row(
        &format!(
            "UPDATE outbox SET state = 'attempted', run = ?1, attempted = {NOW} WHERE id =
             (SELECT min(outbox.id) FROM outbox JOIN runs ON runs.task = outbox.task
              WHERE runs.id = ?1 AND outbox.state = 'queued' AND outbox.compact = 0)
             RETURNING text"
        ),
        [run],
        |row| row.get(0),
    )
    .optional()
}

/// The tmux sessions and harness sessions of ended runs that no live run carries on in that tmux
/// session. A shared harness session lives on in another task's tmux session.
pub(crate) fn ended_sessions(
    db: &Connection,
) -> rusqlite::Result<std::collections::BTreeSet<(String, String)>> {
    let mut statement = db.prepare(
        "SELECT DISTINCT tmux, session FROM runs WHERE ended IS NOT NULL
         AND (tmux, session) NOT IN (SELECT tmux, session FROM runs WHERE ended IS NULL)",
    )?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// After a restart, a prompt attempted but never confirmed may or may not have been typed.
pub(crate) fn uncertain(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE outbox SET state = 'uncertain' WHERE state = 'attempted'",
        [],
    )
}

/// The socket line of `POST /api/tasks/{id}/move`: `{"move": {...}}`, `priory move` with its
/// `--expected`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveLine {
    pub r#move: Move,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Move {
    pub task: i64,
    pub band: Option<String>,
    pub place: Option<Place>,
    pub expected: i64,
}

/// Applies a move line as `priory move` does, and says what moved where. A line with neither a
/// band nor a place is refused, as the verb's usage is: `move_task` would reset the task's place.
pub fn move_line(db: &mut Connection, line: Move) -> Result<String, String> {
    if line.band.is_none() && line.place.is_none() {
        return Err("move takes a band, a place or both".to_owned());
    }
    move_task(
        db,
        line.task,
        line.band.as_deref(),
        line.place,
        Some(line.expected),
    )?;
    Ok(format!("task {} moved", line.task))
}

/// Where `priory move` puts a task in its band's person order. A socket line carries it as
/// `"top"`, `{"before": 7}` or `{"after": 7}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Top,
    Before(i64),
    After(i64),
}

/// Moves the active `task` to `band`, at `place` in that band's person order. Without a band it
/// stays in its own, or joins the band of the task it goes before or after. Without a place it
/// goes after the tasks a person ordered, by age.
pub fn move_task(
    db: &mut Connection,
    task: i64,
    band: Option<&str>,
    place: Option<Place>,
    expected: Option<i64>,
) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    expect_revision(&tx, expected)?;
    moved(&tx, task, band, place)?;
    tx.commit().map_err(|error| error.to_string())
}

fn moved(
    tx: &Transaction,
    task: i64,
    band: Option<&str>,
    place: Option<Place>,
) -> Result<(), String> {
    let error = |error: rusqlite::Error| error.to_string();
    let band_of = |id: i64| -> Result<String, String> {
        tx.query_row(
            "SELECT coalesce(band, '') FROM tasks WHERE id = ?1
             AND state NOT IN ('done', 'failed', 'dropped')",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?
        .ok_or_else(|| format!("task {id} is not active"))
    };
    let own = band_of(task)?;
    if let Some(band) = band {
        check_band(tx, band)?;
    }
    let other = match place {
        Some(Place::Before(other) | Place::After(other)) if other == task => {
            return Err(format!("task {task} cannot move next to itself"));
        }
        Some(Place::Before(other) | Place::After(other)) => Some((other, band_of(other)?)),
        _ => None,
    };
    let target = match (band, &other) {
        (Some(band), Some((other, theirs))) if band != theirs => {
            return Err(format!("task {other} is in band `{theirs}`, not `{band}`"));
        }
        (Some(band), _) => band.to_owned(),
        (None, Some((_, theirs))) => theirs.clone(),
        (None, None) => own,
    };
    let Some(place) = place else {
        tx.execute(
            "UPDATE tasks SET band = ?2, position = NULL WHERE id = ?1",
            params![task, target],
        )
        .map_err(error)?;
        return Ok(());
    };
    let mut order: Vec<i64> = tx
        .prepare(&format!(
            "SELECT id FROM tasks WHERE coalesce(band, '') = ?1 AND id != ?2
             AND state NOT IN ('done', 'failed', 'dropped') ORDER BY {START_ORDER}"
        ))
        .and_then(|mut statement| {
            statement
                .query_map(params![target, task], |row| row.get(0))?
                .collect()
        })
        .map_err(error)?;
    let at = match place {
        Place::Top => 0,
        Place::Before(other) | Place::After(other) => {
            let at = order
                .iter()
                .position(|id| *id == other)
                .expect("other is active in the band");
            at + usize::from(matches!(place, Place::After(_)))
        }
    };
    order.insert(at, task);
    tx.execute(
        "UPDATE tasks SET band = ?2 WHERE id = ?1",
        params![task, target],
    )
    .map_err(error)?;
    for (position, id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE tasks SET position = ?2 WHERE id = ?1",
            params![id, position as i64 + 1],
        )
        .map_err(error)?;
    }
    Ok(())
}

#[cfg(test)]
mod move_line_tests {
    use super::*;

    #[test]
    fn a_move_line_with_neither_band_nor_place_is_refused_and_changes_nothing() {
        let mut db = memory();
        let expected = revision(&db).unwrap();
        let line: MoveLine = serde_json::from_str(&format!(
            r#"{{"move": {{"task": 1, "expected": {expected}}}}}"#
        ))
        .unwrap();

        let moved = move_line(&mut db, line.r#move);

        assert_eq!(moved, Err("move takes a band, a place or both".to_owned()));
        assert_eq!(revision(&db).unwrap(), expected);
    }
}

#[cfg(test)]
mod promises {
    use super::*;

    const GB: u64 = 1 << 30;

    /// A state with tasks 1 to 3, each with a live run of an agent that declares the default 4 GB.
    fn three_runs() -> Connection {
        let db = memory();
        for task in 1..=3 {
            db.execute(
                "INSERT INTO tasks
                   (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
                 VALUES ('cli:' || ?1, 'build', 'implement', 'running', 'now', 'cli', 0, 'cli', 'a',
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                [task],
            )
            .unwrap();
            db.execute(
                "INSERT INTO runs (task, step, agent, tmux, session)
                 VALUES (?1, 'implement', 'implementer', 'priory-' || ?1, 's' || ?1)",
                [task],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn runs_that_use_near_their_promise_reserve_only_what_they_do_not_use_yet() {
        let db = three_runs();
        let used = |run: i64, _: &str, _: &str| match run {
            1 | 2 => Some(3 * GB + GB / 2),
            3 => Some(6 * GB),
            _ => None,
        };

        let promised = promised_with(&db, &Factory::default(), None, used).unwrap();

        // 0.5 GB each for runs 1 and 2, and none for run 3, which uses more than it promised.
        assert_eq!(promised.memory, GB);
    }

    #[test]
    fn a_run_with_no_scope_keeps_its_full_promise_and_held_grants_count_in_full() {
        let db = three_runs();
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, state)
             VALUES (?1, 1, 1, 'build', 'held')",
            [(10 * GB) as i64],
        )
        .unwrap();
        let used = |run: i64, _: &str, session: &str| {
            assert_eq!(session, format!("s{run}"));
            (run == 1).then_some(4 * GB)
        };

        let promised = promised_with(&db, &Factory::default(), Some(3), used).unwrap();

        // Run 1 uses all of its 4 GB, run 2 has no scope, run 3 is the grant's own, and the grant
        // counts all of its 10 GB.
        assert_eq!(promised.memory, 14 * GB);
    }
}

#[cfg(test)]
mod ready_within {
    use super::*;

    #[test]
    fn a_run_under_direnv_is_late_only_after_its_longer_window() {
        let db = memory();
        for (task, ago, direnv) in [(1, 120, None), (2, 120, Some(1)), (3, 700, Some(1))] {
            db.execute(
                "INSERT INTO tasks
                   (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
                 VALUES ('cli:' || ?1, 'build', 'implement', 'running', 'now', 'cli', 0, 'cli', 'a',
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                [task],
            )
            .unwrap();
            db.execute(
                "INSERT INTO runs (task, step, agent, tmux, session, started, direnv)
                 VALUES (?1, 'implement', 'implementer', 'priory-' || ?1, 's' || ?1,
                   strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ?2 || ' seconds'), ?3)",
                params![task, ago, direnv],
            )
            .unwrap();
        }

        let late: Vec<(i64, bool, i64)> = live(&db)
            .unwrap()
            .into_iter()
            .map(|run| (run.task, run.late, run.direnv))
            .collect();

        assert_eq!(late, [(1, true, 0), (2, false, 1), (3, true, 1)]);
    }
}

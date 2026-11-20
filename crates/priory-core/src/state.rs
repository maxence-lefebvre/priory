use crate::room::{self, Share};
use crate::route::{self, Decision};
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::factory::{Factory, Loaded, Session, Workflow};
use priory_factory::lanes::{Lanes, Subject};
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

const VERSION: i64 = 43;

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

/// Each schema change in order: a file at version `n` applies those from index `n` on.
const MIGRATIONS: [&str; VERSION as usize] = [
    SCHEMA, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8, SCHEMA_9,
    SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15, SCHEMA_16, SCHEMA_17,
    SCHEMA_18, SCHEMA_19, SCHEMA_20, SCHEMA_21, SCHEMA_22, SCHEMA_23, SCHEMA_24, SCHEMA_25,
    SCHEMA_26, SCHEMA_27, SCHEMA_28, SCHEMA_29, SCHEMA_30, SCHEMA_31, SCHEMA_32, SCHEMA_33,
    SCHEMA_34, SCHEMA_35, SCHEMA_36, SCHEMA_37, SCHEMA_38, SCHEMA_39, SCHEMA_40, SCHEMA_41,
    SCHEMA_42, SCHEMA_43,
];

/// The order the queue starts tasks in: band, then a person's order, then age. A band the factory
/// no longer has comes after the last one.
pub const START_ORDER: &str = "coalesce((SELECT rank FROM bands WHERE bands.name = tasks.band),
  (SELECT count(*) FROM bands)), tasks.position IS NULL, tasks.position, tasks.id";

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

pub(crate) struct Task {
    pub id: i64,
}

/// Opens the state file `PRIORY_DB`, creating its schema on first use.
pub fn open() -> Result<Connection, String> {
    let path = path()?;
    let at = |error: rusqlite::Error| format!("{}: {error}", path.display());
    let mut db = Connection::open(&path).map_err(at)?;
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
        "view {} changed its filter: {} issues entered and {} left, with no task requested",
        data["view"].as_str().unwrap_or_default(),
        count("entered"),
        count("left")
    )
}

/// Records `message` in the inbox and applies its route, in one transaction. A message
/// already in the inbox changes nothing. Returns the tmux session of a run it ended, to kill.
pub(crate) fn handle(
    db: &mut Connection,
    message: &Message,
    trace: &crate::trace::Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let factory = trace.factory;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let seen = tx
        .query_row(
            "SELECT 1 FROM inbox WHERE id = ?1",
            [message.id.to_string()],
            |_| Ok(()),
        )
        .optional()?;
    if seen.is_some() {
        return Ok(None);
    }
    let resources = route::resources(message);
    // A review event goes to its review task first, even when an older task is keyed by its PR.
    let (keyed, owners) = match (route::requests(message), route::review(message)) {
        (true, _) => (keyed(&tx, resources.first())?, Vec::new()),
        (false, Some(review)) => (keyed(&tx, Some(&review))?, owners(&tx, &resources)?),
        (false, None) => (None, owners(&tx, &resources)?),
    };
    let owner = keyed.as_ref().or(owners.first());
    let reopening = match owner {
        None => crate::tools::reopenable(&tx, message)?,
        Some(_) => None,
    };
    let (task, outcome) = match route::route(message, owner, factory) {
        _ if reopening.is_some() => (reopening, "reopened"),
        _ if keyed.is_some() && route::withdraws(message) => {
            (keyed.as_ref().map(|task| task.id), "withdrew")
        }
        Decision::Create(new) => {
            tx.execute(
                "INSERT INTO tasks
                   (key, workflow, step, state, band, lane, team, project, lane_derived, trigger, source, repo, entered, tldr)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?12)",
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
                        .map(|tldr| crate::search::fit(tldr, counter))
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
                params![id, new.key],
            )?;
            if let Some(hold) = &new.hold {
                crate::hold::filed(&tx, id, hold, factory)?;
            }
            (Some(id), "created")
        }
        Decision::Keep(id) if route::tick(message, factory) => {
            // A human step or a daemon's checks gate has no agent turn for a tick to wake.
            let asleep_at: Option<(String, String)> = tx
                .query_row(
                    "SELECT workflow, step FROM tasks WHERE id = ?1 AND state = 'asleep'
                     AND NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL AND daemon)",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let wakes = asleep_at.is_some_and(|(workflow, step)| {
                factory
                    .workflows
                    .get(&workflow)
                    .and_then(|workflow| workflow.value.step(&step))
                    .is_some_and(|step| step.decides == priory_factory::factory::Decider::Model)
            });
            (Some(id), if wakes { "woke" } else { "coalesced" })
        }
        Decision::Keep(id) => (Some(id), "kept"),
        Decision::Ignore => (None, "ignored"),
    };
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
    let outbox = if running && outcome == "kept" {
        let text = line(
            &message.kind,
            message.subject.as_deref(),
            &message.data.to_string(),
        );
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![task, text],
        )?;
        Some(tx.last_insert_rowid())
    } else {
        None
    };
    tx.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome, outbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
    )?;
    let seq = tx.last_insert_rowid();
    if message.kind == "linear.view.changed" {
        crate::post::report(&tx, "linear", &refiltered(&message.data))?;
    }
    crate::hold::event(&tx, &resources)?;
    // The keyed task keeps the event; another task that shares the resource hears it through its wait.
    for other in owners.iter().filter(|other| Some(other.id) != task) {
        crate::tools::event(&tx, other.id, seq)?;
    }
    match (task, outcome) {
        (Some(task), "kept") => drop(crate::tools::event(&tx, task, seq)?),
        (Some(task), "woke") => {
            let line = line(
                &message.kind,
                message.subject.as_deref(),
                &message.data.to_string(),
            );
            let text = message.data["prompt"].as_str().unwrap_or(&line);
            crate::tools::wake_on(&tx, task, text, Some(seq))?;
        }
        (Some(task), "reopened") => crate::tools::reopen(&tx, trace, task, seq)?,
        (Some(task), "withdrew") => {
            let tmux = crate::tools::cancel(&tx, task, "review request removed")?;
            tx.commit()?;
            return Ok(tmux);
        }
        _ => {}
    }
    tx.commit()?;
    Ok(None)
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

/// Records the factory's bands, highest first, so listings and moves read them without the factory.
pub(crate) fn bands(db: &mut Connection, bands: &[&str]) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM bands", [])?;
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

/// Fires every time wait whose instant passed and task wait whose task ended.
pub(crate) fn due(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::tools::due(&tx)?;
    crate::hold::due(&tx)?;
    tx.commit()
}

/// The resource and the agent's last prompt of each open wait that the intake polls for, then the
/// pull request or Linear issue and creation time of each unmet merge or issue condition of a hold.
pub fn watched(db: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    db.prepare(
        "SELECT resource, since FROM (
           SELECT 0 AS source, waits.id, waits.resource, coalesce(waits.since, waits.created) AS since
           FROM waits
           JOIN tasks ON tasks.id = waits.task AND tasks.state NOT IN ('done', 'failed', 'dropped')
           WHERE waits.fired IS NULL AND waits.withdrawn IS NULL
           AND waits.kind IN ('linear_comment', 'linear_state', 'slack_reply', 'pull_request', 'checks', 'review')
           UNION ALL
           SELECT 1, holds.id, holds.resource, holds.created FROM holds
           JOIN tasks ON tasks.id = holds.task AND tasks.state NOT IN ('done', 'failed', 'dropped')
           WHERE holds.kind IN ('merge', 'issue') AND holds.released IS NULL AND holds.met IS NULL
         ) ORDER BY source, id",
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
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

/// The active task whose own key is `key`: owning the key as a resource is not enough.
fn keyed(tx: &Transaction, key: Option<&String>) -> rusqlite::Result<Option<Task>> {
    tx.query_row(
        "SELECT id FROM tasks WHERE key = ?1
         AND state NOT IN ('done', 'failed', 'dropped') ORDER BY id LIMIT 1",
        [key],
        |row| Ok(Task { id: row.get(0)? }),
    )
    .optional()
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
            "SELECT 1 FROM owns WHERE task = ?1 AND resource = ?2",
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

/// Every active task that owns any of `resources`, in the order `owner` ranks them.
fn owners(tx: &Transaction, resources: &[String]) -> rusqlite::Result<Vec<Task>> {
    let resources = serde_json::to_string(resources).expect("resources serialize");
    tx.prepare(
        "SELECT DISTINCT tasks.id, tasks.key IN (SELECT value FROM json_each(?1)) AS keyed
         FROM owns JOIN tasks ON tasks.id = owns.task
         WHERE owns.resource IN (SELECT value FROM json_each(?1))
         AND tasks.state NOT IN ('done', 'failed', 'dropped') ORDER BY keyed DESC, tasks.id",
    )?
    .query_map([resources], |row| Ok(Task { id: row.get(0)? }))?
    .collect()
}

/// What `priory attach` names: a task, which stands for its live run, or one run.
pub enum Attach {
    Task(i64),
    Run(i64),
}

/// A live run's id, tmux session and the harness session its pane is tagged with.
pub struct Attached {
    pub id: i64,
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
        })
    };
    match *target {
        Attach::Run(run) => {
            let found = db
                .query_row(
                    "SELECT id, tmux, session, ended_as FROM runs WHERE id = ?1",
                    [run],
                    |row| Ok((live(row)?, row.get::<_, Option<String>>(3)?)),
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
                "SELECT id, tmux, session FROM runs WHERE task = ?1 AND ended IS NULL",
                [task],
                live,
            )
            .optional()
            .map_err(at)?
            .ok_or_else(|| format!("task {task} is {state}, with no live run"))
        }
    }
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
}

/// The first queued or ready task at a model step, in start order, that fits in the runner's room.
/// Each one before it gets the reason it waits.
pub(crate) fn admissible<'r>(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    room: impl Fn() -> &'r Result<Room, String>,
) -> rusqlite::Result<Option<Admission>> {
    let mut statement = db.prepare(&format!(
        "SELECT id, workflow, step, trigger, state = 'queued', repo FROM tasks
             WHERE state IN ('queued', 'ready') ORDER BY {START_ORDER}"
    ))?;
    let queued = statement
        .query_map([], |row| {
            Ok(Admission {
                task: row.get(0)?,
                workflow: row.get(1)?,
                step: row.get(2)?,
                trigger: row.get(3)?,
                fresh: row.get(4)?,
                repo: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let promised = promised(db, factory, None)?;
    let paused = paused(db)?;
    // The first task that waits for room: no task behind it starts, so it is never starved.
    let mut head = None;
    let mut stall = None;
    for admission in queued {
        let Some(step) = factory
            .workflows
            .get(&admission.workflow)
            .and_then(|workflow| workflow.value.step(&admission.step))
            .filter(|step| step.decides == priory_factory::factory::Decider::Model)
        else {
            continue;
        };
        let agent = step
            .agent
            .as_ref()
            .and_then(|name| Some((name, &factory.agents.get(name)?.value)));
        let busy = match agent {
            Some((name, agent)) if matches!(agent.session, Session::Shared) => serving(db, name)?
                .map(|task| {
                    format!("waits for agent {name}: its shared session serves task {task}")
                }),
            _ => None,
        };
        // A pause holds only new tasks: a woken, resumed or revived one is `ready` and goes on.
        let busy = busy.or_else(|| paused.clone().filter(|_| admission.fresh));
        let waits = match (busy, head) {
            (Some(reason), _) => Some(reason),
            // The sweeper frees the room the queue waits for, so it never waits for room itself.
            (None, _) if admission.trigger == crate::stall::TRIGGER => None,
            (None, Some(head)) => Some(format!("waits behind task {head}, which waits for room")),
            (None, None) => {
                let need = room::need(agent.map(|(_, agent)| agent));
                match room() {
                    Ok(measured) => {
                        let short = room::wait(measured, share, promised, need);
                        let reason = short.as_ref().map(|short| short.reason.clone());
                        // A task that can never fit would hold the queue forever.
                        if short.is_some() && room::fits_ever(measured, share, need) {
                            head = Some(admission.task);
                            // Waiting only for live runs to end is backpressure, not a stall.
                            stall = room::wait(measured, share, room::Need::default(), need)
                                .map(|short| (admission.task, short));
                        }
                        reason
                    }
                    Err(error) => {
                        head = Some(admission.task);
                        Some(format!("waits for room: cannot measure it: {error}"))
                    }
                }
            }
        };
        let Some(reason) = waits else {
            return Ok(Some(admission));
        };
        db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
            params![admission.task, reason],
        )?;
    }
    crate::stall::record(db, stall.as_ref().map(|(task, short)| (*task, short)))?;
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

/// Records that `person` looked at the desk at `at`, so its Since you looked starts there.
pub fn seen(db: &Connection, person: &str, at: &str) -> rusqlite::Result<String> {
    db.execute(
        "INSERT INTO visits (person, seen) VALUES (?1, ?2)
         ON CONFLICT (person) DO UPDATE SET seen = ?2",
        [person, at],
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

/// The memory and disk declared by the agents of live runs, except those of task `except`, plus every
/// held memory grant. No run reports its own use yet, so each one keeps its whole declaration until
/// it ends.
pub(crate) fn promised(
    db: &Connection,
    factory: &Factory,
    except: Option<i64>,
) -> rusqlite::Result<room::Need> {
    let held: i64 = db.query_row(
        "SELECT coalesce(sum(bytes), 0) FROM grants WHERE state = 'held'",
        [],
        |row| row.get(0),
    )?;
    let mut statement =
        db.prepare("SELECT agent FROM runs WHERE ended IS NULL AND task IS NOT ?1")?;
    // A run recorded before schema 5 has no agent.
    let mut agents = statement.query_map([except], |row| row.get::<_, Option<String>>(0))?;
    let held = room::Need {
        memory: held as u64,
        disk: 0,
    };
    agents.try_fold(held, |sum, agent| {
        let agent = agent?.and_then(|name| factory.agents.get(&name));
        Ok(sum.plus(room::need(agent.map(|agent| &agent.value))))
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

/// The summary and evidence of the task's latest result.
pub(crate) fn last_result(
    db: &Connection,
    task: i64,
) -> rusqlite::Result<Option<(String, Option<String>)>> {
    db.query_row(
        "SELECT summary, evidence FROM results WHERE task = ?1 ORDER BY run DESC LIMIT 1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
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
    /// Derives the lane of a task again, once, now that its branch is known.
    pub lanes: &'a Lanes,
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
        "SELECT session FROM runs WHERE task = ?1 AND agent = ?2 AND ready = 1
         AND session NOT IN (SELECT session FROM shared_sessions)
         AND session NOT IN (SELECT session FROM runs GROUP BY session HAVING count(DISTINCT task) > 1)
         ORDER BY id DESC LIMIT 1",
        params![task, agent],
        |row| row.get(0),
    )
    .optional()
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
        "INSERT INTO runs (task, step, agent, tmux, session) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run.task, run.step, run.agent, run.tmux, run.session],
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
    let mut events = Vec::new();
    let mut text = run.prompt.unwrap_or_default().to_owned();
    {
        let mut statement = tx.prepare(
            "SELECT seq, type, subject, data FROM inbox WHERE outbox IS NULL
             AND ((task = ?1 AND outcome = 'kept') OR id = (SELECT source FROM tasks WHERE id = ?1))
             ORDER BY seq",
        )?;
        let mut rows = statement.query([run.task])?;
        while let Some(row) = rows.next()? {
            events.push(row.get::<_, i64>(0)?);
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&line(
                &row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.as_deref(),
                &row.get::<_, String>(3)?,
            ));
        }
    }
    if !text.is_empty() {
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![run.task, text],
        )?;
        let outbox = tx.last_insert_rowid();
        for seq in events {
            tx.execute(
                "UPDATE inbox SET outbox = ?1 WHERE seq = ?2",
                params![outbox, seq],
            )?;
        }
    }
    if run.shared {
        share(&tx, run)?;
    }
    tx.commit()?;
    Ok(id)
}

/// Records the run's session as its agent's shared one, and names the task in its first prompt.
fn share(tx: &Transaction, run: &NewRun) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO shared_sessions (agent, session) VALUES (?1, ?2)
         ON CONFLICT (agent) DO UPDATE SET session = excluded.session",
        params![run.agent, run.session],
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
    tx.execute(
        "UPDATE tasks SET state = 'blocked', reason = ?2 WHERE id = ?1",
        params![task, reason],
    )?;
    tx.commit()
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
    /// No `SessionStart` arrived within `READY_WITHIN` of the start.
    pub late: bool,
    /// How long its current turn has run, in seconds.
    pub turn: Option<i64>,
    /// Claude's message for the permission prompt it shows, if any.
    pub prompted: Option<String>,
}

pub(crate) const READY_WITHIN: &str = "60 seconds";

pub(crate) fn live(db: &Connection) -> rusqlite::Result<Vec<LiveRun>> {
    let mut statement = db.prepare(&format!(
        "SELECT runs.id, runs.task, runs.step, tasks.workflow, runs.tmux, runs.session, tasks.worktree,
         NOT runs.ready AND runs.started <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{READY_WITHIN}'),
         unixepoch('now') - unixepoch(runs.turn), runs.prompted, coalesce(runs.agent, ''), runs.ready
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
        return Ok(None);
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
                "UPDATE runs SET idle = 1, ready = 1, turn = NULL, nudgeable = 0, thread = coalesce(?2, thread)
                 WHERE id = ?1",
                params![run, signal.thread],
            )?;
            crate::trace::begin(&tx, trace, run, signal.transcript.as_deref())?;
        }
        "Stop" => {
            tx.execute("UPDATE runs SET idle = 1, turn = NULL WHERE id = ?1", [run])?;
            crate::trace::account(&tx, trace, run, signal.transcript.as_deref())?;
            kill = crate::tools::stop(&tx, &signal.session, trace)?;
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
                        "UPDATE outbox SET state = 'confirmed', confirmed = {NOW} WHERE id = ?1"
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

/// Marks the oldest queued prompt of an idle live run `attempted` and the run busy, in one
/// transaction, before anything is typed. A task with an attempted or uncertain prompt gets none
/// while the process it went to may still hold its text: its run is live, as a run carried into the
/// next step takes over the prompts of the run it continues.
pub(crate) fn attempt(db: &mut Connection) -> rusqlite::Result<Option<(String, String, String)>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = tx
        .query_row(
            "SELECT outbox.id, runs.id, runs.tmux, runs.session, outbox.text FROM runs
             JOIN outbox ON outbox.task = runs.task AND outbox.state = 'queued'
             WHERE runs.ended IS NULL AND runs.idle = 1 AND NOT EXISTS (
               SELECT 1 FROM outbox AS open JOIN runs AS typed ON typed.id = open.run
               WHERE open.task = runs.task AND open.state IN ('attempted', 'uncertain')
               AND typed.ended IS NULL)
             ORDER BY outbox.id LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((outbox, run, tmux, session, text)) = next else {
        return Ok(None);
    };
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
    Ok(Some((tmux, session, text)))
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

/// Where `priory move` puts a task in its band's person order.
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

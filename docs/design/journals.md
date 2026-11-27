# Journals

This note designs journals: the state a scheduled automation keeps from one run to the next, kept
on the log every runner shares. With it, any runner can run any automation and start from where the
last run stopped. The note builds nothing. It holds what exists, the model, storage, security, how
an automation moves over, failure modes, five slices that each ship alone, and the open questions
with a recommendation each.

## What exists today

- **Automations keep their state in files on one machine.** A daily report appends a line per run
  to a `runs.jsonl`. A weekly retro writes dated notes files. A dependency upgrade leaves a handover
  file for the next run. A memory sweep reads a JSON history, then rewrites it. Each file lives in a
  folder of the machine that ran it, outside the task's worktree, so the next run finds it only on
  that machine.
- **A schedule trigger may start its next run anywhere.** Each firing publishes a task request
  ([schedule.md](../intake/schedule.md)). With a fixed `key`, every firing names one task, which
  stays on its runner while it waits. Without one, each firing is a new task, which any runner whose
  filters accept it may claim ([runners.md](../daemon/runners.md#claims)). A task also moves when
  its runner's lease ends, or when a person gives it away. Each case lands the next run on a machine
  that may lack the files, or holds an older copy of them.
- **Each task records its trigger.** The state file's `tasks` table has a `trigger` column, which
  this note uses to tie a journal to the trigger that owns it.
- **Memories already travel on the log.** An agent writes a candidate with `remember`, every runner
  reads `priory.memory.written` in log order, and a person accepts it before other tasks' runs get
  it. A memory file holds at most 4096 bytes ([memories.md](../reference/memories.md),
  [events.md](../reference/events.md#memories)).
- **The log keeps every message for ever.** Postgres stores each message as one `TEXT` value in
  `priory_log_messages`, with no size limit, and nothing prunes it. Every runner reads every message
  on its one-second tick ([runners.md](../daemon/runners.md#every-runner-reads-the-whole-log)). A
  factory of one machine uses herdr-bus's SQLite file instead
  ([ADR 0002](../adr/0002-a-factory-chooses-herdr-bus-or-postgres-for-its-event-log.md)).
- **Restricted roles exist.** The intake connects with a Postgres role whose row policy lists the
  event types it may insert. The diocese note plans the same for the gateway, which may insert only
  `priory.run.called` ([diocese.md](diocese.md#database-roles)).
- **The word is free for agents.** The upgrade helper's `$PRIORY_HOME/upgrade/journal.json` is an
  internal file no agent sees ([upgrades.md](../daemon/upgrades.md)). This note's journals are a
  different thing, and the docs should keep "upgrade journal" for that file.

## The model

### A journal per trigger

A journal is the state one trigger keeps between its runs. Its name is the trigger's name, such as
`daily-report`. A run of a task that its trigger started reads and writes that trigger's journal
without naming it.

The journal belongs to the trigger, not the task, because a schedule trigger without a fixed `key`
starts a new task on each firing. A per-task journal would start empty on every run, which is the
problem this note solves. A trigger with a fixed `key` gets the same journal either way.

A task that a person requests with no trigger, such as `priory request`, has no journal. A run calls
the tools and gets an error that says so.

### Why a journal is not a memory

| | Memory | Journal |
| - | ------ | ------- |
| What it holds | One fact for every run of the factory | One automation's working state |
| Who reads it | Every task's runs, once a person accepts it | The runs of one trigger |
| Who checks it | A person accepts each candidate | Nobody: the next run trusts it as it trusts its own files today |
| Size | 4096 bytes | Entries up to 16 KB, documents up to 256 KB |
| Lifetime | Until a person retires it, then git | Pruned by the trigger's `keep` |
| How often | A few a week | Every run |

A memory reaches the instructions of every later run, so a person reviews it. A journal reaches only
the next runs of the same automation, which already trusted their own files. Putting run state in
memories would flood the desk with candidates nobody wants to review and would put one automation's
notes into every other run's index. A run that learns a fact every task should know still calls
`remember`.

### Two shapes

The real cases need two shapes:

- **Entries**: append-only, in log order. A `runs.jsonl` line, a dated note, a handover. Each entry
  has a `kind`, such as `run` or `note`, and a body: a JSON value or Markdown text. A run reads the
  last N entries, or those since a time, optionally of one kind.
- **Documents**: latest wins, by name. A JSON history that each run reads and rewrites. A run puts a
  document of a name, and a later put of the same name replaces it.

A handover is an entry of kind `handover`: the next run reads the last one. It needs no third shape.

### Sizes

| | Limit | Why |
| - | ----- | --- |
| An entry | 16 KB | A `runs.jsonl` line or a day's notes fit; a dump does not |
| A document | 256 KB | A JSON history of a year of runs fits |
| A trigger's journal | 8 MB of live entries and documents | Bounds what a runner folds and keeps |

A write over a limit fails with an error that names the limit, and writes nothing. Larger files,
such as a report's rendered output, belong in the task's pull request or an artifact, not the
journal.

Slice 1 stores every document inline in its log message, so it needs no second store. A 256 KB
message is the largest the log carries. Open question 2 weighs a blob table for larger ones.

## Tools and commands

A run's MCP server gets four tools. Each acts on the journal of the run's own task's trigger.

| Tool | Does |
| ---- | ---- |
| `journal_append(kind, body)` | Appends an entry and returns its id |
| `journal_read(last?, since?, kind?)` | Returns entries, oldest first: the last `last` (default 20, at most 200), or those since the RFC 3339 time `since` |
| `journal_put(name, body)` | Replaces the document `name` |
| `journal_get(name?)` | Returns the document `name`, or the names, sizes and times of every document when `name` is absent |

A code step or a script calls the same through the CLI. Inside a run's pane, where `PRIORY_SESSION`
is set, or a `run` action, where `PRIORY_TASK` is set, each command finds its trigger from that
task:

```sh
priory journal append --kind run '{"date":"2026-11-26","items":14}'
priory journal read --last 7 --json
priory journal put history < history.json
priory journal get history > history.json
```

A person reads any journal by its trigger's name, with `--trigger daily-report`, and so does the web
view. A person also runs the two commands a run never gets:

- `priory journal import --trigger NAME`, to move an automation over (see
  [Moving an automation over](#moving-an-automation-over)).
- `priory journal prune --trigger NAME`, to apply the trigger's `keep` at once.

### Who may write and read

- A run writes only its own task's trigger's journal. The daemon finds the trigger from the run's
  session, so the tools take no trigger argument and a run cannot name another one.
- A run reads only that journal, plus the journals its trigger lists in `journal.reads`, so a weekly
  retro may read the daily report's entries. Open question 3 covers this.
- A person reads any journal and writes only through `import`. A person who wants to fix a document
  runs `put` from a shell with `--trigger`, which the daemon records with the person's name.
- The intake and the gateway write nothing (see [Security](#security)).

## Storage

### Events on the log

Each write is a message on the log, published through the daemon's `sends` table as claims and
memories are:

- `priory.journal.appended`: `trigger`, `kind`, `body`, `task`, `runner`. Its `id` is the entry's id.
  Subject: the trigger.
- `priory.journal.put`: `trigger`, `name`, `body`, `task`, `runner`. Subject: the trigger.
- `priory.journal.pruned`: `trigger`, `before`, the time before which entries go. Subject: the
  trigger.

Every runner reads them in log order and folds them into two tables of its state file,
`journal_entries` and `journal_documents`, keyed by trigger. A read answers from those tables, so a
run reads its journal with no round trip to the diocese. A runner refuses, and records as
`refused`, a message over its size limit, or one whose `task` it knows was started by another
trigger.

A table on the diocese beside the log was weighed:

| Way | For | Against |
| --- | --- | ------- |
| Events on the log | One order for every runner; an offline runner catches up; works on herdr-bus with no new code path; no new credential or role | Every runner stores every journal; the log grows with every write |
| A table on the diocese | Reads only what a run needs; prunes with one `DELETE` | A second store, a second schema step and a second role policy; a runner cut off from the diocese reads nothing; a SQLite factory needs another code path |

The recommendation is events on the log. A run reads its journal from its own runner's tables even
when the diocese is away, which matters more than the storage each runner spends. At 8 MB per
trigger and tens of triggers, each runner keeps well under a gigabyte.

### A factory on one machine

A factory on herdr-bus publishes and reads the same three types on its SQLite file. Nothing differs,
and moving to Postgres starts the log empty as ADR 0002 says. So `priory journal import` from the
old runner's tables, or the slice 5 export, carries journals over a backend switch.

### Retention and pruning

Each trigger names how long it keeps entries:

```toml
[journal]
keep = "90d"        # or a count, such as 500; documents keep only their latest put
reads = ["daily-report"]
```

The default is `90d`. Each runner deletes entries past `keep` from its own tables on its tick, and
`priory.journal.pruned` makes a person's `prune` reach every runner. A document keeps only its
latest body.

That prunes the runners' tables, not the log. The Postgres log keeps every message today, and
journals make it grow fastest of any type. Slice 4 adds log compaction: a `priory log compact`
command, run daily on the diocese, deletes journal messages older than every runner's cursor and
past their trigger's `keep`, and older puts of a document name. It deletes no other type, so claims,
answers and memories keep their history.

## Security

- **The intake and the gateway write no journal.** Their role policies list the types each may
  insert, and neither list holds a `priory.journal.*` type, so Postgres refuses the insert. Slice 1
  adds a test against Postgres that each role's insert of `priory.journal.appended` and
  `priory.journal.put` fails. Their row policies stay a list of what is allowed, so a new type is
  refused until someone adds it.
- **A runner writes for its own tasks only.** A runner role may still publish any message, as
  today. Every runner checks that a journal message's `task` belongs to its `trigger`, from the
  claims and requests it read, and refuses one that does not. A runner whose password leaks can
  still write any journal, as it can already answer any ask; the diocese note's per-runner roles are
  where that closes.
- **Journal text is untrusted.** The last run wrote it, from issues, pull requests and web pages it
  read. A run gets it from a tool result, never in its instructions, and the tool's description
  says to treat it as data.
- **No private terms.** The daemon screens each write for the factory's private terms, as
  [guard.md](../daemon/guard.md) screens what leaves the factory, and refuses a write that holds one.

## Moving an automation over

An automation moves once, by its trigger:

1. A person runs `priory journal import --trigger NAME --entries runs.jsonl --kind run`, and
   `--document history=history.json` for each document, on the machine that holds the files. Each
   line of a `.jsonl` file becomes an entry; each file of a notes folder, with `--entries notes/`,
   becomes an entry dated by its name or its time.
2. The command publishes the entries in their old order, with their old times in the body, as
   ordinary journal messages, and prints how many it wrote and the size of each document.
3. The person edits the automation's prompt or script to read and write through the tools and stop
   touching the files, and renames the old folder, so a missed path fails loudly.
4. After a week of runs on more than one runner, the person deletes the old files.

Importing twice writes every entry twice. The command refuses when the journal already has entries,
unless the person passes `--again`.

## Failure modes

| What fails | What happens |
| ---------- | ------------ |
| The diocese is away during a run | The run reads its runner's tables, which may lack the last few writes of another runner. Its own writes queue in `sends` and publish when the log is back |
| Two runs of one trigger at once | Both append; entries interleave in log order. Two puts of one document: the later in log order wins, as for memories. A trigger that cannot take this keeps a fixed `key`, so only one task runs |
| A runner back after days away | It reads the missed messages on its tick before it claims anything, as today |
| A write over a limit | The tool fails and writes nothing; the run decides what to drop |
| A message a runner refuses | No runner applies it, so every runner keeps the same tables |

## Slices

Each slice ships alone and leaves the factory working.

Slice 1 is on `main`. Slices 2 to 5 are later slices. Slice 1 screens a write for credentials, not
yet for the factory's private terms, and its CLI reaches a journal through `PRIORY_SESSION` only, not
through a `run` action's `PRIORY_TASK`.

### 1. Entries and documents on the log

- **Code.** The three event types and their schemas, the two state tables, the four tools, and
  `priory journal append|read|put|get`. Size limits. The intake's role policy test.
- **Tests.** A run on one runner appends and puts; a run of the same trigger on another runner reads
  them. A run of another trigger cannot read or write them. A write over a limit fails. A task with
  no trigger gets the error. The intake's role cannot insert either type.
- **Rollback.** Remove the tools. The messages stay on the log and no runner reads them.

### 2. Import

- **Code.** `priory journal import`, with `--entries`, `--kind`, `--document` and `--again`.
- **Tests.** A `runs.jsonl` of three lines becomes three entries in order. A second import without
  `--again` fails.
- **Rollback.** None needed: the old files stay until a person deletes them.

### 3. Retention and reads across triggers

- **Code.** The trigger's `[journal]` table with `keep` and `reads`, `priory check` rules for them,
  pruning on each runner's tick, `priory journal prune` and `priory.journal.pruned`.
- **Tests.** Entries past `keep` leave every runner's tables. A trigger reads a journal it lists and
  is refused one it does not.
- **Rollback.** Remove the keys; every journal keeps entries for the default.

### 4. Log compaction

- **Code.** `priory log compact`, which deletes journal messages that every cursor has passed and
  their trigger's `keep` has dropped, and older puts of each document. It deletes no other type.
- **Tests.** After a compact, a new runner that reads the log from the start builds the same tables
  as an old one. A claim older than every cursor stays.
- **Rollback.** Stop running it. The log grows as today.

### 5. The web view and export

- **Code.** A trigger's journal in the web view, newest first, and `priory journal export`, which
  writes a trigger's entries and documents in the shape `import` reads.
- **Tests.** An export then an import into an empty factory gives the same journal.
- **Rollback.** Remove the panel and the command.

## Open questions

1. **Per trigger or per task?** Per trigger, as this note says. A task with a fixed key already gets
   one journal, and one without a key needs the trigger's. A per-task journal adds nothing a trigger
   journal cannot do.
2. **Documents over 256 KB: inline or a blob table?** Inline only, in slice 1, and a hard limit. A
   blob table needs a second store, a second role policy and a second path on herdr-bus, for a case
   no known automation has. Revisit when one needs more, and then store the blob on the diocese,
   keyed by its hash, with the message carrying the hash.
3. **Who reads another trigger's journal?** Only a trigger that lists it in `journal.reads`, and only
   to read. A weekly retro reading the daily report is the case; a global read would let any run see
   every automation's notes.
4. **Should a person accept journal writes?** No. That is what memories do, and the volume, one or
   more writes per run, would bury the desk. A run that learns a factory-wide fact calls `remember`.
5. **Should the dreaming step read journals?** Yes, read-only, through `journal.reads` on its
   trigger, so its proposed memories can draw on what automations noted
   ([dreaming.md](dreaming.md)). Decide in that note's next slice.

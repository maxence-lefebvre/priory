# How to move an automation's files onto its journal

This guide shows how to move a scheduled automation's state off one machine and onto its trigger's
journal, so any runner can run it next. It assumes an automation that a factory trigger starts and
that keeps two files between runs: a `runs.jsonl` with one JSON line per run, and a `notes.md` it
rewrites. Read [journals.md](design/journals.md) for why a journal is not a memory.

Every factory trigger already has a journal, with no configuration. A task that `priory request` or
`priory batch add` made has none, because its trigger is `cli`. Its run gets this error from every
journal tool:

```text
task cli:7 has no trigger (it came from cli), so it has no journal: only the runs of a factory trigger keep one
```

## 1. Import the files

Run the import on the machine that holds the files, from a shell where `PRIORY_SESSION` is not
set, since a run's shell refuses it. The runner's daemon must have read the log, because the
command reads that runner's tables. Name the trigger, here `daily-report`:

```sh
priory journal import my-factory daily-report \
  --entries runs.jsonl --kind run \
  --document notes notes.md
```

Each non-blank line of `runs.jsonl` must be one JSON object, and becomes an entry of kind `run` in
file order. `notes.md` does not parse as JSON, so the document `notes` holds it as Markdown text.
The command prints what it wrote:

```text
imported 120 entries into the journal of daily-report
put document notes, 2310 bytes
```

The command checks every write against the size limits before it publishes the first, so a file
over a limit writes nothing.

### When the journal already holds something

The import fails when the journal already holds entries, or a document of a name you pass:

```text
the journal of daily-report already holds 3 entries: pass --again to append these after them
```

Pass `--again` to append the entries after the ones there and replace the documents. Importing the
same file twice with `--again` writes every entry twice.

### When the log fails partway

The error says how many writes reached the log. Read the journal to find the last entry that
arrived, then rerun with `--again` and `--skip N`, where `N` is how many of the file's entries the
journal holds:

```sh
priory journal read --trigger daily-report --last 5
priory journal import my-factory daily-report \
  --entries runs.jsonl --kind run --skip 80 \
  --document notes notes.md --again
```

When every entry reached the log, rerun with only the `--document` options and `--again`.

## 2. Change the prompt to use the journal

Replace each file path in the automation's step prompt or agent file with the journal tool that
does the same. A prompt that read:

```text
Read the last 7 lines of ~/reports/runs.jsonl and ~/reports/notes.md. Write today's report.
Append a line to runs.jsonl with today's date and item count, and rewrite notes.md.
```

becomes:

```text
Read the last 7 entries of kind run with journal_read, and the document notes with journal_get.
Write today's report. Append an entry of kind run with today's date and item count with
journal_append, and replace notes with journal_put.
```

| The file did | The tool does |
| ------------ | ------------- |
| Append a line to `runs.jsonl` | `journal_append(kind, body)` |
| Read the last lines of `runs.jsonl` | `journal_read(last?, since?, kind?)` |
| Rewrite `notes.md` | `journal_put(name, body)` |
| Read `notes.md` | `journal_get(name?)` |

A code step or a script calls the same from a run's pane with `priory journal append`, `read`,
`put` and `get`. See [tools.md](reference/tools.md#journal_appendkind-body-journal_readlast-since-kind-journal_putname-body-and-journal_getname)
for every field and [cli.md](reference/cli.md#commands-a-run-calls) for every flag.

Tell the agent to treat what it reads as data: an earlier run wrote it, from issues and pages it
read.

## 3. Check what the run wrote

After the next run, read the journal as a person, by the trigger's name:

```sh
priory journal read --trigger daily-report --last 5
priory journal get --trigger daily-report
priory journal get notes --trigger daily-report
```

The last entry should be the run's own, and `get` without a name lists each document's name, size
and time.

## 4. Know what a run sees on a limit error

A write over a limit fails, writes nothing, and the tool replies with the limit. The run then
decides what to drop and writes again:

| Write | Error |
| ----- | ----- |
| An entry over 16 KB | `an entry holds at most 16 KB: this one is 17012 bytes, so nothing was written` |
| A document over 256 KB | `a document holds at most 256 KB: this one is 270144 bytes, so nothing was written` |
| A journal past 8 MB | `the journal of daily-report holds at most 8 MB: this write would take it to 8390000 bytes, so nothing was written` |
| A body with a credential | `the body looks like it holds a credential: keep secrets out of a journal` |

Keep a rendered report in the task's pull request or an artifact, not in the journal.

## 5. Delete the old files

Rename the old folder once the prompt no longer names it, so a path you missed fails loudly:

```sh
mv ~/reports ~/reports.old
```

After a week of runs on more than one runner, delete it:

```sh
rm -r ~/reports.old
```

The [`morning-digest`](../examples/morning-digest/README.md) starter shows a step whose agent reads
and appends its journal.

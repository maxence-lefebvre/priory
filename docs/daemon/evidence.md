# Evidence

A run's result can carry evidence: screenshots, terminal recordings and screen recordings that show
the change working, so a person sees it work before reading the diff. This page explains where an
item comes from, how Priory keeps secrets out of it, and where it goes. The design is in
[reviewer-evidence.md](../design/reviewer-evidence.md).

## An evidence item

An item is one file. It belongs to a run, and the run's result carries it beside its `evidence`
text.

| Kind | What it is | Accepted as | Who may add it |
| ---- | ---------- | ----------- | -------------- |
| `image` | A screenshot | PNG, JPEG, WebP | The step's `record` command |
| `cast` | A terminal recording | asciicast v2 | The `record` command, or the agent |
| `video` | A screen recording | MP4, WebM | The `record` command |

Each item has a caption of one line, the commit it was made at, its size and its SHA-256. Its file
lives at `PRIORY_HOME/evidence/<task>/<sha256>.<ext>`, outside every worktree, so no sweep of a
worktree removes it and no commit can pick it up. A row in the state file names it. Nothing changes
an item once it is stored.

Priory ships no recorder. A factory's own script makes the screenshots and videos, with Playwright
or anything else.

## A step's `record` command

A model step names a command of the factory as its [`record`](../reference/workflows.md#step-keys):

```toml
[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
record = "scripts/record-login"
next = { passed = "done", failed = "stop" }
```

The path is relative to the factory folder, and `priory check` refuses one that leads outside it,
so the script is the factory's file, changed by a pull request on the factory. A task's branch
cannot edit it.

When the step's result is stored, the daemon owes the command a run at the result's `commit`, or
the head of the task's branch when the result names none. It runs it in the background, as a
[`command` scorer](score.md#a-command-scorers-checkout) runs: it adds a detached worktree of that
commit from Priory's clone under `PRIORY_HOME/records/`, runs the command there, and removes the
worktree. So a file the agent saved and did not commit, such as a `.env` it wrote, is not there to
be shown. A daemon that restarts runs an owed command again.

Before the command runs, the daemon searches every tracked file of the checkout for the
[secrets Priory knows](#never-secrets). When one holds a value, nothing is recorded, and the
reason names the file and the secret's variable, never the value: a token committed into a fixture
would be drawn on the page.

The command runs in its own process group, in the checkout, bounded by the factory's
`command_timeout`. Its environment is built from a short list, and holds none of the daemon's
other variables:

| Variable | Value |
| -------- | ----- |
| `PATH`, `LANG`, `TERM` | The daemon's, when set |
| `HOME` | A new, empty folder |
| `PRIORY_EVIDENCE` | The folder to write items to |
| `PRIORY_TASK` | The task's id |
| `PRIORY_COMMIT` | The commit it records |

It writes each item to `$PRIORY_EVIDENCE`, and each item's caption to a file of the same name
followed by `.caption`, such as `login.png` and `login.png.caption`. An item without a caption file
is left out. On exit 0, the daemon admits each item, by name, and stores the ones that pass. On any
other exit, it stores none, and the result stays as it was.

The result's `record` in the [task's API](../reference/serve.md#task-in-detail) says how it went:
`queued` while it is owed, `recorded`, `refused` after the tracked-file search, or `failed`, with a
reason. A recorded run's reason lists each file it left out, and why.

## A cast the agent adds

An agent adds a terminal recording from its run's pane, as it runs `priory gate`:

```sh
priory evidence add session.cast --caption "The migration, run twice"
```

It works only where `PRIORY_SESSION` names a live run, and only for a file inside the run's
worktree, followed through symbolic links. The worktree must be at the head of the task's branch,
which is the commit the item carries. The command refuses an image or a video with
`only a step's record command adds an image` (or `a video`): an agent's screenshot can show
anything its run could reach, and no check reads pixels.

## Never secrets

An item is checked before it is stored, or it is not stored. The checks, in order:

1. **Size.** A file over 10 MB is refused.
2. **Type.** The file's first bytes must match an accepted type, whatever its name. An SVG or an
   HTML file is refused, since it can carry script.
3. **The door.** An image or a video comes only from a `record` command.
4. **The caption.** It is one line of at most 200 characters, and holds no secret.
5. **The cast's text.** A cast's output events are joined into one stream, and so are its input
   events, so a value split across two events is found. The streams, the header's strings and the
   whole file are searched.
6. **The task's budget.** A task holds at most 20 items and 50 MB.

The secrets Priory knows are the values of the factory's secret variables, such as `[serve]
token_env` and each webhook's `secret_env`, and of [`[serve] redact_env`](../reference/factory.md#serve),
as the daemon's environment holds them. A value is searched whatever its length, where the
screen's redaction skips a value under 8 bytes: a short value that matches by chance refuses an
item, which is the safe mistake. The parts of a value that the screen also hides, such as a URL's
password, are searched when they are 8 bytes or more.

A match refuses the item. Priory does not redact it and keep it: a recording that printed one
secret may hold a second one Priory does not know. The refusal names the secret's variable, never
its value. A `record` command whose last line of output holds a secret fails without that line.

No check reads pixels, so the text rules do not carry over to an image or a video. The search knows
only the secrets the factory names. The application a `record` command records is the task's
committed code, which an agent wrote, and it runs as the person's Unix user, so code written to
read `~/.ssh` and draw it could. So the guarantee has two parts. No text item holds a secret Priory
knows. And every image and video was made by the factory's own script, from a checkout searched for
those secrets, in a process that was given none.

## On the task page

The [task's API](../reference/serve.md#task-in-detail) gives each result its `attachments`, and
`GET /api/evidence/{id}` serves an item's file to a token holder, with a fixed content type,
`nosniff` and a sandbox policy (see [the endpoint](../reference/serve.md#evidence)). Captions are
agent text: show them as text. The web view does not draw items yet.

`priory evidence TASK` lists a task's items in a terminal ([cli.md](../reference/cli.md)).

## On the pull request

The body `open-pr` writes gains an Evidence section after the summary and the `evidence` text: one
line per item, with its caption, kind and short commit. When the factory names its server's
address in [`[serve] url`](../reference/factory.md#serve), each line links to the task page,
`<url>/tasks/<id>`, which opens for whoever can reach the server. Without it, the section holds
the captions alone. A task with no item gets no section. Punctuation in a caption is escaped, so
it shows as text.

No bytes leave the machine: GitHub gets the captions and the links. The section lists the items
that existed when a person approved the step, the ones the approval covers. An item made after the
answer, such as a `record` command that finished late, shows in the API with `approved` false and
is not listed. A step the factory marks `autonomous` has no approval, and its section lists every
item.

## How long items stay

The sweep removes a task's items, files first, once the task has been `done`, `failed` or `dropped`
for 30 days. It never removes an active task's items, an asleep one's included. The sweep runs when
the daemon starts and every [`[sweep] every`](sweep.md).

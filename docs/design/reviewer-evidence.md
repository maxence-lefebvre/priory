# Evidence a reviewer can watch

This page explains how a run could attach screenshots, recordings or a live preview to its result,
so a person sees the change work before reading the diff. It builds nothing. It holds the model,
the first code slice with its tests, the slices after it, and the open questions with a
recommendation for each.

The idea comes from a read of other agent factories on 2026-11-22.
[Devin](https://docs.devin.ai/work-with-devin/testing-and-recordings) records the desktop while it
tests and sends the video. [Factory's Droid
Control](https://docs.factory.com/automations/droid-control) records before and after. [Mastra
Factory](https://factory.mastra.ai/llms-full.txt) gives each session an app preview link. Minutes of
a person's attention per merged pull request is Priory's first [success
measure](../prd.md#success-measures).

Evidence must never hold a secret. Code can enforce that for text. For pixels it can only
narrow where a secret could come from, and the model below says where that line is.

## What existed when this note was written

- A result's `evidence` is one optional string (`crates/priory-core/src/tools.rs:265`), stored in
  the `results` table, which nothing can update or delete (`crates/priory-core/src/state.rs:77`).
- `open-pr` writes the pull request body as the summary, a blank line, then that string
  (`crates/priory-core/src/daemon.rs:1704`), through `gh pr create --body`
  (`crates/priory-runtime/src/runtime.rs:394`).
- The task page's API gives each result's `evidence` as text
  ([serve.md](../reference/serve.md#task-in-detail)).
- A plan carries `testing` and `verification`, and each later model step gets `Prove the change
  with:` and the verification ([tools.md](../daemon/tools.md)). The proof is the agent's sentence.
- One proof is checked by code: `gate-passed`, from `priory gate`
  ([tools.md](../daemon/tools.md#proofs)).
- A run's screen is served with known secrets replaced by `[redacted]`: the values of the
  factory's secret variables and of `[serve] redact_env`
  (`crates/priory-core/src/screen.rs:53`, `crates/priory-factory/src/factory.rs:309`, `:1985`,
  [serve.md](../daemon/serve.md#a-runs-screen)).
- A `run` action runs a factory's command with no agent, in the task's worktree when it has one
  ([tools.md](../daemon/tools.md#the-run-action)).

## The model

### An evidence item

An evidence item is one file, or one link, that shows the change working. It belongs to a run and
is kept with that run's result.

| Kind | What it is | Accepted as |
| ---- | ---------- | ----------- |
| `image` | A screenshot | PNG, JPEG, WebP |
| `cast` | A terminal recording, as text with timings | asciinema v2 `.cast` |
| `video` | A screen recording | MP4, WebM |
| `preview` | The change running, for a person to try | made by Priory, later slice 2 |

Each item has a caption of one line, the commit it was made at, its size and its SHA-256. The file
lives at `PRIORY_HOME/evidence/<task>/<sha256>.<ext>`, outside the worktree, so a sweep or a retire
does not remove it and no commit can pick it up. A row in the state file names it. Like a result, an
item is never changed. The sweep removes a task's items 30 days after the task ended, and never an
active task's.

### Who may make an item

Priory can check text for a secret. It cannot check pixels. So the two are admitted by different
doors.

**A recording the daemon makes.** A step names a `record` command of the factory, such as a
Playwright script:

```toml
[[steps]]
id = "implement"
record = "scripts/record-login"
```

The path is relative to the factory folder, as `[asks] deliver` is, so the script is the
factory's file, changed by a pull request on the factory, and never a file the task's branch can
edit.

After the step's result, the daemon runs it in a scratch checkout of the result's commit, as a
command scorer runs ([score.md](../daemon/score.md)), and not in the agent's worktree. So a file
the agent saved and did not commit, such as a `.env` it wrote, is not there to be shown. The
command writes files and one caption per file to the folder `$PRIORY_EVIDENCE` names. It runs
with an environment built from a short list: `PATH`, `LANG`, `TERM`, an empty new `HOME`, and
Priory's own variables. It gets none of the factory's secrets and none of the daemon's other
variables. Images and video are admitted only through this door.

**A cast the agent adds.** The agent runs a command from its pane, as it runs `priory gate`:

```sh
priory evidence add session.cast --caption "The migration, run twice"
```

It works only in a run's pane, where `PRIORY_SESSION` is set, for a cast inside the run's
worktree. An agent cannot add an image or a video this way. See open question 2.

The run's result then carries every item of its step. The `evidence` string stays, as the words
beside the items.

### Never secrets

An item is admitted before it is stored, or it is not stored. The checks, in order:

1. **Type and size.** The file's first bytes must match an accepted type, whatever its name. An
   SVG or HTML file is refused, since it can carry script. A file over 10 MB is refused, and so is
   one that takes the task past 50 MB or 20 items.
2. **Text is searched whole.** A cast's output events are joined into one stream before the
   search, so a value split across two events is found. The stream, the cast's header and the
   caption are searched for every value of the factory's secret variables, of `[serve]
   redact_env` and of the secrets declared for agents
   ([agent-credentials.md](agent-credentials.md)), whatever its length. The screen's redaction
   skips a value under 8 bytes. An admission check does not: a short value that matches by chance
   refuses an item, which is the safe mistake.
3. **A match refuses the item.** It is not redacted and kept: a recording that printed one secret
   may hold a second one that Priory does not know. The agent reads why, without the value.
4. **A caption is one line** of at most 200 characters, checked as in rule 2.
5. **The commit must be the branch's head**, as `priory gate` requires.

Then two rules on where an admitted item goes:

6. **It is served only behind the server's token**, to whoever may already watch the run's
   terminal.
7. **Bytes leave the machine only under a person's approval.** Publishing them is an irreversible
   effect. It happens only inside an irreversible step a person approved, for the items that
   existed at the approved commit, which the approving ask lists.

Before the `record` command runs, the daemon searches the checkout's tracked files for the same
list of values, and refuses to record when one is there: a token committed into a fixture would
be drawn on the page.

The limits, stated plainly. No check reads pixels, so the text rules do not carry over to an
image. The search knows only the secrets the factory names. The recorder is the factory's
script, and the application it records is the task's committed code, which an agent wrote. That
code runs as the person's Unix user, so code written to read `~/.ssh` and draw it could. So the
guarantee has two parts. No text item holds a secret Priory knows. And every image and video was
made by the factory's own script, from a checkout searched for those secrets, in a process that
was given none. Closing the rest needs the wall of
[agent-credentials.md](agent-credentials.md).

### On the task page

`GET /api/tasks/{id}` gains `attachments` on each result, beside its `evidence` string: `id`,
`kind`, `caption`, `commit_sha`, `bytes`, `sha256` and `created`. `GET /api/evidence/{id}` serves
the file to a token holder with a fixed content type from the table above, `Content-Disposition:
inline` for images and video, `X-Content-Type-Options: nosniff` and a `Content-Security-Policy:
sandbox`. A cast is served as text and drawn by the page.

Later slice 1 draws a result's items under its summary on the task page: images as thumbnails, casts
and videos with a play control, each with its caption and commit. An item made at an older commit
than the branch's head says so. The approving ask's detail shows the same list. This is web view
work, so it starts from a design a person reviewed ([CONTRIBUTING.md](../../CONTRIBUTING.md#ui)).

Captions are agent text. The page prints them as text, never as Markdown or HTML.

### On the pull request

The pull request body gains an Evidence section after the summary: one line per item, with its
caption, kind, commit and a link to the task page. The link opens for whoever can reach the
factory's server, which is the person the pull request is for. A reader without that access sees
the captions and the count.

Images inline in the body need the bytes on GitHub, which rule 7 gates. See later slice 3 and open
question 1.

`priory evidence TASK [--json]` lists a task's items in a terminal.

## The first slice: the `record` command, casts and the API

1. The `evidence` table and its migration, the store under `PRIORY_HOME/evidence/`, and the
   admission checks.
2. A step's `record` key: the command, its environment, and its files as items.
3. `priory evidence add FILE.cast --caption TEXT` in a run's pane, and `priory evidence TASK`.
4. `attachments` on results in `GET /api/tasks/{id}`, and `GET /api/evidence/{id}`.
5. The Evidence section in the body `open-pr` writes, with links when the factory names its
   server's address, and captions alone when it does not.
6. The sweep removes a task's items 30 days after it ended.
7. [tools.md](../daemon/tools.md), [workflows.md](../reference/workflows.md),
   [serve.md](../reference/serve.md) and [cli.md](../reference/cli.md) gain the key, the verb, the
   field and the endpoint.

The slice changes no web view file. The task page's drawing of items is later slice 1, from a
reviewed design.

The type check reads magic bytes. A crate such as `infer` does it. The slice proposes it with its
license and last release, or writes the six signatures by hand, and names which in its pull
request.

Tests, through the CLI and the HTTP server:

- The `record` command runs in a checkout of the result's commit: an uncommitted file of the
  agent's worktree is absent there. A `record` path that leaves the factory folder fails `priory
  check`.
- A checkout whose tracked file holds the value of a factory secret is not recorded, and the
  reason names the file and not the value.
- A step's `record` command that writes a PNG and a caption gives the run's result one attachment
  in the API, with its caption, commit and SHA-256, and the endpoint returns the same bytes.
- The `record` command's environment holds no variable of the daemon beyond the short list: a
  script that prints `env` shows none of the factory's secret variables and a `HOME` that is an
  empty folder.
- A `record` command that exits 1 adds no item and leaves the step's result as it was.
- `priory evidence add` admits a cast, and refuses a PNG with `only a step's record command adds
  an image`. It exits 1 outside a run's pane, for a file outside the worktree, and through a
  symlink that leaves it.
- A file named `.png` that holds HTML, an SVG, an 11 MB image and a twenty-first item are each
  refused with the reason.
- A cast that prints the value of a factory secret is refused, and the refusal does not hold the
  value. So is a cast whose secret is split across two output events, a cast with the value in
  its header, a caption that holds it, and a cast that prints a 6-byte secret.
- The endpoint answers `401` without the token, and its headers hold `nosniff` and the sandbox
  policy.
- A caption with `<script>` is returned as the same text by the API.
- The pull request body of a task with two items holds an Evidence section with two lines, and a
  task with none holds no section.
- An item made after the approving answer shows in the API with its own commit, and is not in the
  list the approval covers.
- A `proptest` feeds the type check and the cast reader random and mutated files and asserts each
  returns or refuses without panicking.
- The sweep removes the items of a task that ended 31 days ago, and keeps those of a task that
  ended 29 days ago and of an asleep task.

## Later slices

1. The task page and the approving ask draw the items, from a reviewed design: thumbnails, a play
   control, captions as text.
2. A live preview. A person starts it from the task page: `priory preview TASK` runs the
   factory's preview command in the task's worktree, bound to the machine's tailnet address, and
   stops it after 15 minutes without a request. It passes the room check like a run, and starts
   with the `record` command's environment. It is a person's verb: no agent starts a server that
   outlives its turn.
3. Images on the pull request itself, for a public repository. `open-pr` pushes the approved
   items to a ref of the repository and links them in the body. It runs under the step's
   approval, as rule 7 says. See open question 1 for private repositories.
4. Before and after: the `record` command also runs at the task's `base_sha`, and the page shows
   the two side by side.
5. A proof, `evidence-recorded`: a step's outcome is refused until the `record` command exited 0
   at the head commit.
6. Images an agent adds itself, behind a person. See open question 2.

## Open questions

1. **Where do bytes for the pull request live?** GitHub has no documented API to attach an image
   to a pull request body. An image pushed to a ref of a private repository and linked as
   `blob/...?raw=true` renders broken in a pull request body, as the author saw on a private
   repository: only `github.com/user-attachments` URLs render there. A bucket adds a credential
   and a public URL. Recommendation: links to the task page in the first slice, and nothing more
   for a private repository. Later slice 3 is for public repositories only, behind the step's
   approval, until GitHub documents an upload.
2. **May an agent add its own screenshots?** An agent's screenshot can show anything its run
   could reach, a token included, and no check reads pixels. Storing it and hiding it until a
   person looks still stores it. Recommendation: not in the first slice. Later slice 6 admits
   them only once a run cannot reach the person's credentials, and then as evidence, never as
   proof.
3. **Video in the first slice?** A browser video needs a recorder on every runner. Recommendation:
   accept the `video` kind from the `record` command from the start, within the size limits, and
   ship no recorder. The factory's own script makes the file.
4. **How long are items kept?** A video per task fills a disk. Recommendation: 30 days after the
   task ended, as a constant, counted in the disk the sweep reports. Move it to `[sweep]` when a
   person asks.
5. **Should a screenshot be scanned for text?** Optical character recognition would catch a
   printed token sometimes, and adds a heavy dependency and a false sense of safety.
   Recommendation: no. Keep the two doors, and say plainly that no check reads pixels.
6. **Does the preview belong in this note?** It is a running process, not a file, with its own
   room, port and lifetime. Recommendation: keep it listed here as later slice 2 and give it its
   own design note before it is built.

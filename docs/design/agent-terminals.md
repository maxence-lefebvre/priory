# Agent terminals

This page explains how a person could watch a run's terminal, and type into it when allowed, from
the web view on a phone or a desktop and from Herdr. It covers the terminal panel of R8.3 and the
terminal attach of M2. It builds nothing. It holds the model, the first code slice with its tests,
the slices after it, and the open questions with a recommendation for each.

## What exists today

- Each run is a detached tmux session, `priory-<task id>`, launched at 200 columns by 50 rows, whose
  agent pane carries the run's `@priory` tag. A read-write attach resizes it to the client ([runtime.rs](../../crates/priory-runtime/src/runtime.rs)).
- `priory attach TASK [--read-only] [--print]` checks that a live pane carries the run's tag, then
  runs or prints `tmux -S <socket> attach-session [-r] -t '=priory-<id>'`
  ([sessions.md](../daemon/sessions.md#attaching-to-a-run)). tmux stays the owner (R5.6).
- The Herdr adapter already opens that command as a native terminal tab: a panel sets
  `priory.attach`, and the Priory bar runs `priory attach TASK --print` and opens it with
  `tab.new` in the active space. It refuses a space on another machine
  ([herdr.md](../adapters/herdr.md#how-it-reads-and-acts)). It always attaches read-write.
- The daemon already reads a pane with `tmux capture-pane -p` for a run that showed no
  `SessionStart`, and types prompts with `load-buffer` and `paste-buffer`, only once the run is idle
  and after it stored the attempt (R5.4, [outbox.md](../daemon/outbox.md)).
- `priory say` and `POST /api/tasks/{id}/say` already send a person's line to the agent through
  that outbox, prefixed `A person says:` ([desk.md](../daemon/desk.md#say)).
- The web view's Terminal panel is a stub that prints `priory attach`. DESIGN.md already describes
  the pane: dark `--term-*` tokens, no wrapping, sideways scroll, a "Fit" toggle, read-only by
  default, and a one-line input behind "Type".
- `serve` has 8 workers, at most 4 held `/api/events` requests, a 5 second limit for a reply to go
  out, no WebSocket and a CSP of `default-src 'none'; script-src 'self'; style-src 'self'`
  ([serve.md](../daemon/serve.md)).

## The model

### Watching: snapshots, not a stream

A terminal on a page shows a screen, and tmux already keeps the screen. So the server reads it
when asked, with `capture-pane`, and sends the text. It needs no terminal emulator on either side,
no per-viewer process and no state between requests.

`priory screen TASK [--run RUN] [--json]` prints the screen, so the CLI covers the panel (R8.1), and
`GET /api/runs/{id}/screen?have=HASH` returns its JSON:

| Field | Meaning |
| ----- | ------- |
| `run`, `task` | The run and its task. The endpoint answers only for a live run, so it sends no state |
| `width`, `height` | The pane's size, from `#{pane_width}` and `#{pane_height}` |
| `cursor` | `{ "x", "y" }`, or null when the cursor is hidden |
| `lines` | The visible screen, one string per row, blank rows kept, trailing spaces trimmed, after redaction |
| `hash` | A hash of the reply after redaction, so it never tells a hidden value changed. With `have` equal to it, the reply is `{"hash", "same": true}` and nothing else |
| `redacted` | How many secrets the server replaced, see [Secrets on screen](#secrets-on-screen) |

The server resolves the run's pane as `priory attach` does: the live pane with the run's tag. It
then reads that pane by its id, never the session's active pane, so a split or a second window
cannot show another program. The size and cursor come from the same pane, and change when a
read-write attach resizes it. The existing `screen()` helper does not fit: it drops blank rows and
keeps the last 30 lines.

tmux commands find their server from the caller's `TMUX` and `TMUX_TMPDIR`. A daemon's serve
shares the daemon's, but a standalone `priory serve` needs the same values in its environment, as
`priory attach` does. Without them every screen gets `404` with attach's sentence about the
daemon's tmux server.

It refuses a run that ended, and a pane whose tag does not match, with `404` and the same sentence
as `priory attach`. A run that ended keeps no screen on the server: the page keeps the last one it
painted, as DESIGN.md asks.

The screen keeps its colours, from `capture-pane -e`, as spans the page builds from the SGR codes
itself. The page never parses another escape: a cursor move or a title change in the text is shown
as text.

Scrollback is a second read, `?history=N` with `capture-pane -S -N`, capped at 2,000 lines, behind
a "Load earlier" button: the agent's transcript, not its scrollback, is the record of what it did.

### Typing: one line at a time, through the daemon, recorded

Typing has two paths, and the panel offers both.

1. **Say** stays the default for words. "Type" in the pane sends the line to `POST
   /api/tasks/{id}/say`, so it goes through the outbox, is typed only when the run is idle, cannot
   start a slash command, and wakes a sleeping task. It already exists.
2. **Keys** are for what Say cannot do: answer a menu or a harness dialog, press Escape to
   interrupt a turn, or press `C-c`. They go straight into the pane, now, whatever the agent is
   doing.

Keys need a person's explicit act, per run:

- The page shows the keys only after the person presses "Take the keyboard" in the pane's menu and
  confirms a sheet that says what it means (see [What typing bypasses](#what-typing-bypasses)).
  The sheet prevents an accident. It is no protection against a stolen token, since any token
  holder can take a grant.
- That sends `POST /api/runs/{id}/keyboard`. It needs no state revision: other runs' writes move it
  every second, and the run must be live either way. The daemon
  stores a grant in the state: run, start, end 15 minutes later, and a label, the `User-Agent`'s
  first 64 bytes. The label helps a person tell their devices apart; nobody can trust it. The reply
  carries the grant's secret, 128 random bits, to that page only. The state keeps its hash, and no
  log, error, trace or export holds it. The daemon appends a `factory.terminal.taken` record with
  the grant's public id, the run and the label.
- A grant ends at its end, when the run ends, on `POST /api/runs/{id}/keyboard/return`, on a new
  grant for the run, and when `[serve] terminal` no longer says `"type"`. It survives a daemon
  restart, since it is stored, and each request checks it again. `factory.terminal.returned` records
  the end and why.
- `POST /api/runs/{id}/keys` carries the grant's secret, a request id the page picks, and exactly
  one of `text` or `key`. `text` is at most 1 KiB of printable characters: any control character,
  newline and Escape included, gets `400`, so text cannot sidestep the key list. `key` is one of
  `Enter`, `Escape`, `Tab`, `BTab`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `C-c`,
  `C-d`, `y`, `n` and `1` to `9`. Anything else gets `400`.
- The daemon types text as the outbox does, through `load-buffer` from stdin and a bracketed
  `paste-buffer`, without the Enter, so the text never sits in a process's arguments and tmux never
  reads it as a command. It sends a key with `send-keys` and a name from the list. The target is
  always the pane the daemon resolved, never one from the request.
- The daemon stores the request, with the grant, the request id, the key or `text`, and the text's
  length, before it sends. After the send it stores the outcome and appends one
  `factory.terminal.keys` record that holds it. A repeated request id gets the first outcome and
  sends nothing. A stored request with no outcome, after a crash, becomes `uncertain`, with its
  record, and the daemon never sends it again, as R5.4 does for prompts.
- Like every write, these endpoints need the token, pass the Origin rule, and reach the daemon on
  its socket. Without a daemon they get `503`.

While a grant is open, the keyboard is the person's. The outbox types nothing into that run, and
`say` queues as usual. When the grant ends, the outbox resumes at the run's next idle point. Text
the person typed and did not submit still sits in the agent's input box then, and the next prompt
would join it, so "Give back" shows the screen and asks the person to clear or submit it first.
The daemon cannot read the input box, so this stays a known limit.

The trace keeps what kind of input went in and how long it was, never the text. A person may type a
password into a dialog or a shell that the agent never sees, and the trace is exported over OTLP
(R10.2).

A person's keys can start a turn Priory did not start. The daemon then sees that turn's hooks like
any other, as [sessions.md](../daemon/sessions.md#attaching-to-a-run) says, and R6.7's nudge does
not fire for it.

### What typing bypasses

The tool guard reads the agent's tool calls, in a `PreToolUse` hook that runs before any allow
rule. Keys from a person are not tool calls, so three kinds of input reach past it:

- **Direct shell.** Claude Code's bash mode, a prompt starting with `!`, runs the command itself,
  with no tool call for the hook to read, at the time of writing. A pane that fell back to a shell
  after the agent exited runs whatever is typed. Either way `gh pr merge 12` merges.
- **Harness settings.** Slash commands such as `/permissions` and `/config` change what the run
  may do for the rest of the run. Priory's `PermissionRequest` hook still denies permission
  prompts, and the guard still runs first on every Bash and MCP call.
- **Words to the agent.** They can ask for anything. The guard still reads the calls that follow,
  and denies the guarded ones as usual.

Harness behaviour changes often (see the PRD's Risks), so the sheet does not list these cases.

So taking the keyboard gives the device the power of a person at a shell on the runner, as
`priory attach` already does. The confirm sheet says so in one line: "Your keys reach the agent
and its shell directly. Priory's guard does not check them." The grant, the record and the 15
minute end make that act visible; they do not make it safe. A factory that does not want it sets
`[serve] terminal = "watch"`, see [Open questions](#open-questions).

### Secrets on screen

An agent can print a secret: `env`, a `.env` file, `gh auth token`, a key in a diff. The screen
endpoint then sends it to every device that holds the token, and through `tailscale serve` to a
phone. Four rules bound that:

1. **Redact the secrets Priory knows exactly.** Before it answers, the server replaces with
   `[redacted]` the bearer token it loaded, whether `token_env` names it or the default does, and
   the value of every variable the effective configuration reads a secret from: the webhook
   `secret_env`s, `url_env`, `headers_env` and Slack's `token_env`, defaults included, and any
   variable listed in `[serve] redact_env`. A composite value is split into its parts too: each
   header value of `headers_env`, and the password of a `url_env` URL. A value under 8 bytes is
   skipped, since it would match ordinary text.
2. **Redact well-known shapes.** It also replaces strings that match a short fixed list: GitHub
   tokens (`ghp_`, `gho_`, `ghs_`, `ghu_`, `github_pat_`), Anthropic and OpenAI keys (`sk-ant-`,
   `sk-`), AWS access keys (`AKIA` and 16 characters), Slack tokens (`xox[abpr]-`, `xapp-`), Linear keys
   (`lin_api_`) and PEM private key blocks. A row that held one says so in `redacted`, and the pane
   shows a `--warn-text` line, "2 secrets hidden on this screen".
3. **Keep nothing.** The screen reply carries `Cache-Control: no-store`. The page keeps no screen
   in local storage, unlike its listings, so an offline start shows no old screen, and drops the
   one it shows on sign-out, on a `401` and when the pane switches runs. The service worker already
   stores no `/api` reply. "Copy screen" copies the redacted text.
4. **Send nothing elsewhere.** No screen goes to the trace, a notification, the OTLP export or the
   server's log.

Redaction is best effort. A secret split across two rows, a token in a shape not on the list, or a
password in plain words gets through. The note says so where `[serve] terminal` is documented, and
the honest boundary stays the token: whoever holds it may read what the agent's screen shows.

### Transport within serve's limits

| Way | Fits serve? | Phone cost |
| --- | ----------- | ---------- |
| Short poll of `/screen`, with `have` | Yes, as it stands: one request, one buffered reply | One small request a second while the pane is visible; a full screen only when it changed |
| Long poll, held until the screen changes | Needs a worker to watch tmux while it holds the request, and a cap of its own beside the 4 `/api/events` waiters | Lower latency |
| Server-sent events of `pipe-pane` output | No: a handler builds one whole reply, which must go out within 5 seconds. A stream needs a new kind of reply, and a server-side emulator to turn bytes into a screen | Lowest latency |
| WebSocket | No: serve has none, and adding it is a new protocol on the trust boundary | Lowest latency |

Short polling fits. A terminal watched from a phone needs to show what the agent is doing within
about a second, not every keystroke. The page keeps at most one poll in flight per pane. It polls
every second while the pane is visible and the document is shown, backs off to 3 seconds after 10
unchanged replies, and stops when the page hides or gets a `401`. A changed `revision` from
`/api/events` polls at once, so a run that ends updates quickly.

A snapshot runs two tmux commands: one to find the tagged pane with its size and cursor, and one
`capture-pane` of it. Priory's command timeout is 5 minutes by default, so a hung tmux server
would hold workers. So a snapshot has its own 2 second deadline for both, and at most 2 snapshots
run at once; a third gets `503` with `Retry-After: 1`. With 4 `/api/events` waiters, that leaves 2
workers to other requests however tmux behaves. A reply over 256 KiB is cut to its last rows and
says so; a 200 by 50 screen of plain ASCII is about 12 KiB of JSON. Each poll is then a few
milliseconds of a worker, an estimate that the first slice measures on a busy factory.

Keys are writes, so each is one `POST`, with a poll right after it so the person sees the effect.

### Rendering

| Option | Licence | Last release | Fit |
| ------ | ------- | ------------ | --- |
| Styled snapshot: a `<pre>` of the rows, in the `--term-*` tokens | none | none | Shows what capture-pane sends. No dependency. Text is selectable and copyable, and a screen reader reads it |
| [xterm.js](https://github.com/xtermjs/xterm.js) (`@xterm/xterm`) | MIT | 6.0.0, 2025-12-22 | A full emulator, 345 KiB as its ES module. It creates `<style>` elements at run time, which `style-src 'self'` blocks, so it needs the CSP widened. It shines with a byte stream, which serve does not send |
| [ghostty-web](https://github.com/coder/ghostty-web) | MIT | 0.4.0, 2025-12-09 | Ghostty's parser compiled to WebAssembly. Needs `'wasm-unsafe-eval'` in `script-src`. Young |
| [ansi_up](https://github.com/drudru/ansi_up) | MIT | 6.0.6, 2025-05-16 | Turns SGR codes into HTML spans. Only colours, and the page's own few lines do the same |
| [vt100](https://crates.io/crates/vt100), server side | MIT | 0.18.1, 2026-10-19 | Parses a `pipe-pane` stream into a screen. Only needed for a stream |

The recommendation is the styled snapshot, which DESIGN.md's Terminal pane already describes. It
adds no dependency, keeps the CSP as it is, and the page draws agent text as text, as it does
everywhere. An emulator only pays off with a byte stream, and the transport above sends screens.

On a phone, 200 columns at 12 px do not fit. The pane scrolls sideways, and "Fit" scales the text
down to 9 px, which shows about 65 columns on a 390 px wide phone. Priory never resizes the window
for a viewer: the agent's layout stays the one its 200 columns drew.

### Herdr

Herdr already does the cheapest thing: it opens `priory attach --print` as a native terminal tab
on the runner's machine, a real tmux client that sees every byte. Two gaps remain:

- **It always attaches read-write.** The task panels should offer "Watch", which runs
  `priory attach TASK --read-only --print`, and keep "Attach" for typing. Watch becomes the default
  button, as read-only is the web pane's default.
- **Nothing records a read-write attach.** `priory attach` without `--read-only` should send the
  daemon one line, which appends a `factory.terminal.attach_issued` record with the task, the run
  and whether it printed or ran the command. It records that someone asked for a command that can
  type, not that a client attached, nor who typed: a printed command can run later or never, and
  tmux does not report a client's keys. Herdr gets it for free, since it runs the same verb.

A native tmux client sits outside everything the HTTP path enforces: redaction, grants, the key
list and `[serve] terminal`. It is a person at a shell on the runner.

A space on another machine stays refused. Attaching from there needs a shell on the runner, which
[views-from-any-machine.md](views-from-any-machine.md) leaves to the owner's own view.

## The first slice: watch a run from the web view

Watching answers the M2 done-when line's need, which is to see from a phone what a run is doing
before answering its ask. Typing waits for the second slice.

1. `priory-runtime` gains `snapshot(name, session, deadline) -> Option<Snapshot>`: one
   `list-panes` that finds the tagged pane with its id, size and cursor, then `capture-pane -p` of
   that pane id, each in its own process group and killed at the deadline.
2. `priory-core` gains `redact(text, exact) -> (String, usize)`, with the fixed shapes and the
   exact values.
3. `priory screen TASK [--run RUN] [--json]` prints it. `serve` gains
   `GET /api/runs/{id}/screen?have=HASH`, with `Cache-Control: no-store`, the 2 second deadline,
   the cap of 2 snapshots, and the `[serve] terminal` key, `"watch"` by default, where `"off"` gets
   `404` for the endpoint.
4. The web view's Terminal panel, and the task page's terminal under its runs table, poll it as
   above and draw the DESIGN.md pane, without "Type".
5. `docs/reference/serve.md`, `docs/daemon/serve.md`, `docs/reference/factory.md` and
   `docs/daemon/sessions.md` describe the endpoint, the key and the redaction.
6. Herdr's panels gain "Watch", as above.

A Claude Design artifact of the pane on a phone and a desktop, reviewed by a person, comes before
the web view code, as AGENTS.md asks.

Tests:

- A run whose tmux session prints a known screen: `/screen` and `priory screen --json` return its
  rows, blank rows included, its size and a hash; the same `have` returns `same: true`; new output
  returns a new hash.
- A run whose session has a second pane, active and printing other text, returns the tagged
  pane's rows. A resized window returns the new size.
- A pane whose tag names another run, a dead pane, an ended run and an unknown run each get `404`
  with `priory attach`'s sentence.
- A screen that prints the default `PRIORY_TOKEN`, a custom `token_env` value, a webhook secret, a
  header value from `headers_env`, a `ghp_` token, two secrets that overlap and a PEM block cut
  off at the bottom row returns each as `[redacted]`, with the right `redacted` count, and the
  server's log holds none of them. Two screens that differ only in a hidden value have one hash.
- `redact` has a `proptest` test: random and mutated text never panics, and the output never
  contains a given secret of 8 bytes or more that `[redacted]` does not itself contain.
- With 4 held `/api/events` and a tmux stand-in that hangs, a snapshot returns `404` within its
  deadline, a third concurrent one gets `503`, and `/api/desk` still answers.
- The reply carries `Cache-Control: no-store`, and `[serve] terminal = "off"` gets `404`.
- Without the token, or with a wrong one, `/screen` gets `401`, also with a `Host` and `Origin`
  that match.
- `node --test` for the panel: the poll schedule (a second while visible, 3 seconds after 10
  unchanged replies, none while hidden, at once on a new revision, one in flight); a stale reply
  for another run is dropped; a `401`, a sign-out and a run switch clear the screen and stop
  polling; local storage never holds a screen; and rows draw as text, so a `<script>` row stays
  text.
- The Herdr test for "Watch" checks the argv holds `--read-only`.

Each tmux test takes a `TmuxDir`, as CONTRIBUTING asks.

## Later slices

All five slices are built.

1. **Say from the pane.** "Type" sends a line to the existing `POST /api/tasks/{id}/say`. It
   needs no new endpoint, so it can ship right after the first slice, apart from keys.
2. **Keys.** The keyboard grant, `/keyboard`, `/keyboard/return`, `/keys`, the three trace records,
   the outbox pause and the confirm sheet, as
   [Typing](#typing-one-line-at-a-time-through-the-daemon-recorded) describes, behind
   `[serve] terminal = "type"`. On the CLI, `priory attach` stays the way to type, since a person
   at a shell already has the runner. Its tests cover a text with a newline, Escape or another
   control character refused; `;`, a leading `-` and Unicode typed as text; a repeated request id
   sent once; a crash between the record and the send left `uncertain`; an expired, returned or
   replaced grant refused; and the outbox typing nothing while a grant is open.
3. **The attach record.** `factory.terminal.attach_issued` from `priory attach` without
   `--read-only`.
4. **Colours.** `capture-pane -e`, with the page turning SGR codes into spans. DESIGN.md gains eight
   terminal colour tokens beside `--term-*`, dark in every theme.
5. **Scrollback.** `?history=N`, behind a "Load earlier" button.

## Open questions

1. **Should `[serve] terminal` default to `"watch"`, `"type"` or `"off"`?** Watching shows
   secrets the redaction misses to any device with the token, and typing gives that device a shell.
   Recommendation: `"watch"` by default, since the token holder already acts on every task and
   reads results, and `"type"` only when the factory sets it. A factory that serves beyond the machine can choose
   `"off"`.
2. **Should keys need a second factor beyond the token, such as a code shown on the runner?** The
   token sits in every signed-in browser's storage, and any holder can take a grant.
   Recommendation: not in the keys slice, which stays off unless a factory sets `"type"`. Keep the
   grant short, recorded and shown on the desk while it lasts. Revisit if views gain a stronger
   identity than the one token.
3. **Should a grant show on the desk?** Recommendation: yes, as a to-do, "Your keyboard reaches
   task 7's terminal until 14:32", with "Give back". A person forgets an open grant on a phone.
4. **Should the trace keep typed text or only its length?** Recommendation: only the kind and
   length, as above. Text typed into a dialog or a shell need not reach the agent's transcript, and
   the trace leaves the machine through OTLP. The agent's transcript already holds what it was
   told.
5. **Should Herdr typing use the keys endpoint instead of a tmux client, to record keys?**
   Recommendation: no. A native terminal is the reason to use Herdr, and the attach record already
   says a read-write command was issued.

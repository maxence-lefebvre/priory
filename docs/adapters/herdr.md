# How the Herdr adapter works

Priory's views render in a browser first (R8.4). The PRD also asks for a second view host that
renders the same views as an adapter (R8.6). The Herdr adapter is that host for
Herdr, a terminal workspace whose panes run JavaScript mods drawn natively
with gpui-kit. This page explains where the adapter lives, how it reads and changes the factory,
how it delivers answers, and how another front end would write its own adapter.
[How to install the Herdr adapter](../installing-the-herdr-adapter.md) installs it, and
[Herdr panels](../reference/herdr-panels.md) lists what each panel reads, shows and runs.
[DESIGN.md](../../DESIGN.md), under "Herdr adapter", holds the visual rules.

## Where it lives

The adapter lives in `adapters/herdr/` at the repository root, outside the Cargo workspace:

- `adapters/herdr/lib/priory.js`: the module every mod uses to read and act. It is the only code
  that runs `priory`. It also holds the words the panels share with the web view, such as `age`
  and `ago` for times and `eventWords` for event types.
- `adapters/herdr/mods/priory-tabs/`: the Priory bar. It sits left of Herdr's own tab bar, shows
  one button per aside and center tab in the layout, opens the Task tab, and attaches to a task's
  terminal. See [The layout](../reference/herdr-panels.md#the-layout).
- `adapters/herdr/mods/priory-desk/`: the Desk panel. See [The Desk panel](../reference/herdr-panels.md#the-desk-panel).
- `adapters/herdr/mods/priory-asks/`: the Asks panel. See [The Asks panel](../reference/herdr-panels.md#the-asks-panel).
- `adapters/herdr/mods/priory-queue/`: the Queue panel. See [The Queue panel](../reference/herdr-panels.md#the-queue-panel).
- `adapters/herdr/mods/priory-bonjour/`: the Bonjour panel. See
  [The Bonjour panel](../reference/herdr-panels.md#the-bonjour-panel).
- `adapters/herdr/mods/priory-costs/`: the Costs panel. See [The Costs panel](../reference/herdr-panels.md#the-costs-panel).
- `adapters/herdr/mods/priory-usage/`: the usage line, docked at the bottom of the window. See
  [The usage line](../reference/herdr-panels.md#the-usage-line).
- `adapters/herdr/mods/priory-flows/`: the Flows panel. See [The Flows panel](../reference/herdr-panels.md#the-flows-panel).
- `adapters/herdr/mods/priory-task/`: the Task panel. See [The Task panel](../reference/herdr-panels.md#the-task-panel).
- `adapters/herdr/mods/priory-inbox/`: the Inbox panel. See [The Inbox panel](../reference/herdr-panels.md#the-inbox-panel).
- `adapters/herdr/mods/priory-tasks/`: the Tasks panel. See [The Tasks panel](../reference/herdr-panels.md#the-tasks-panel).
- `adapters/herdr/mods/priory-runs/`: the Runs panel. See [The Runs panel](../reference/herdr-panels.md#the-runs-panel).
- `adapters/herdr/layouts/frame.json`: Herdr's chrome around the view: the sidebar, the top bar
  with the Priory bar, 420 px wide, and Herdr's tab bar, the work body with Herdr's `main` block,
  and the bottom row with Herdr's status line and the [usage line](../reference/herdr-panels.md#the-usage-line).
- `adapters/herdr/lib/layout.js`: turns a view into a Herdr layout inside the frame, and finds a
  view's blocks in the layout Herdr shows. See [The layout](../reference/herdr-panels.md#the-layout).
- `adapters/herdr/layout`: reads `priory views --json` on its standard input and prints every
  view's layout as one JSON object, keyed by its file name.
- `adapters/herdr/install`: links the mods into Herdr's folder and writes the layout. See
  [Installing](../installing-the-herdr-adapter.md#installing).
- `adapters/herdr/test/`: `node --test` tests for the module and the install script, run against a
  fake `priory`. CI runs them with `node --test adapters/herdr/test/*.test.js`. `test/kit.js`
  stands in for Herdr's `gpui-kit`, `gpui-base` and `herdr` modules, so a test can load a panel's
  `main.js`, draw it to a tree and press its elements. A press reaches each element from the one
  pressed up to the root, until a handler stops it, as gpui delivers it.
- `adapters/herdr/lib/tokens.js`: Priory's theme colors and font roles from
  [`design/tokens/themes.css`](../../design/tokens/themes.css), every theme, each `light-dark()`
  pair kept as a light and a dark value, and the structure tokens a pane draws with from
  [`design/tokens/structure.css`](../../design/tokens/structure.css). `palette(theme, mode)` picks
  one color per mode. A test fails when the file and `design/tokens/` differ.
- `adapters/herdr/deliver`: the factory's `[asks] deliver` command; see
  [Delivering answers](#delivering-answers).
- `adapters/herdr/attach`: the factory's `[runs] attach` command; see
  [Opening runs in Herdr](#opening-runs-in-herdr).

Herdr loads a mod's imports only from the mod's own folder, and it resolves symbolic links before it
checks. So each mod keeps a copy of `lib/priory.js` as its own `priory.js`, and a mod that draws
with Priory's colors keeps a copy of `lib/tokens.js`. The Desk and Queue panels also keep
`lib/add.js`, the Add action's view model, and `lib/add-form.js`, its form. The Desk, the Task
panel and the Priory bar keep `lib/layout.js`, whose `showAsks` opens the Asks panel to answer. A test fails when a copy differs from the
library. After you change a file in `lib/`, copy it into every mod that carries it.

The Costs, Flows, Inbox, Tasks and Runs panels keep the web view's own view models: `mods/priory-costs/view.js` is a
copy of `crates/priory-core/web/costs.js`, `mods/priory-flows/view.js` of `flows.js`,
`mods/priory-inbox/view.js` of `inbox.js`, `mods/priory-tasks/view.js` of `tasks.js`, and
`mods/priory-runs/view.js` of `runs.js`, beside its own copy of `tasks.js` for the facet filter. A test
fails when a copy differs from the web file. After you change one, copy it into its mod. Herdr's
engine has no `Intl`, so these files format numbers and dates by hand.

The adapter depends on no Priory crate, and no crate depends on it. It needs Node.js to install,
because `install` builds the layout with it, and for its tests.

## How it reads and acts

A panel reads the factory through Priory and acts through Priory's own verbs. It never opens the
state database and never writes a file in the factory.

- **Which Priory.** A mod runs the program that the `priory` object of Herdr's `settings.json`
  names, on the Herdr machine it names: `{"priory": {"binary": "~/.local/bin/priory-oss",
  "machine": "desktop", "factory": "/home/me/factory", "theme": "graphite", "web":
  "http://127.0.0.1:7070"}}`. Every field is optional.
  `theme` is `graphite`, `indigo`, `ink`, `parchment` or `abbey`, and defaults to `graphite`. A panel
  keeps the last theme it read while Priory is unreachable. `binary` defaults
  to `~/.local/bin/priory-oss`, and a leading `~` means the home folder on Priory's machine. A name
  without a slash is looked up on the `PATH`. `machine` defaults to the machine Herdr runs on. `factory` is the folder
  that `queue` and `schedules` read for up next, `costs` reads its prices from, and `flows` draws. The program must find its state file, so set
  `PRIORY_DB` in the environment, or name a wrapper script that sets it.
- **Reading.** A mod runs the CLI with Herdr's `exec` and parses its JSON: `priory desk --all --json`,
  `asks --all --json`, `tasks --json`, `queue --json`, `schedules --json`, `links --json`,
  `report show [TASK] --json`, `report list --json`, `costs [--by SPLIT] --json`, `report attention --json` and `flows --json`. Each answer carries the state's `revision`, except
  the reports `costs` and `report attention`, and `flows`, which reads the factory's files. `report show --json` wraps the
  report as `{"revision": STATE, "report": {...}}`: the outer `revision` is the state's, and the
  report's own `revision` is the one `report pick --expected` takes. Then the mod runs
  `priory wait --since REVISION`, which returns as soon as the state changes, or after 25 s, and
  reads again. It keeps the last good answer, with its age, when a call fails, and tries again a
  few seconds later. `follow` in `priory.js` is that loop, and every panel that waits uses it; a
  panel stops its loop when it reads something else, such as another task. `priory serve`
  answers the same JSON over HTTP, and its `/api/events` waits for a change as `priory wait` does:
  see [serve.md](../reference/serve.md).
- **Acting.** Every button runs a CLI verb: `answer --expected N [--run] -- ASK TEXT`,
  `dismiss --expected N ASK`, `snooze`, `say`, `report pick PICK OPTION --expected N`,
  `report comment ROW --expected N TEXT`, `report reply|resolve|reopen COMMENT`, `seen`,
  `resume`, `revive`, `todo done`, `inbox read`, `subscribe TASK RESOURCE --expected N`, `hold`, `release`, `move`, `cancel`, `pause`, `unpause` and
  `add FACTORY [--workflow W] [--band B] [--lane L] -- TEXT`. An answer always carries the revision the person read, so a
  changed question refuses it (R6.3).
- **Safety.** `priory.js` passes every value as its own argument, never through a shell, and never
  reads text output except `attach --print`. A mod draws agent text as plain text, apart from its links.
- **Autolinks.** `autolink(text, links, repo)` in `priory.js` splits text into spans, each
  `{ text }` or `{ text, url }`, with the rules `links()` reads from `priory links --json`. It
  returns the same spans as the web view, and links only to `https:` URLs. `spansHtml(spans)`
  escapes them into the HTML Herdr's `TextView.html` draws; Herdr opens a link with the system's
  browser. A panel reads the rules once, when it connects. When `priory links` fails, such as with
  no factory to read, the panel draws its text unlinked. See [Links](../reference/herdr-panels.md#links).
- **Attaching.** A panel asks the Priory bar to attach with
  `setShared("priory.attach", { task, at: Date.now(), readOnly })`. The bar runs
  `priory attach TASK --print`, with `--read-only` when `readOnly` is true, and opens a terminal tab
  that runs the printed command, which starts a tmux client on the task's live run (R5.6). A
  read-only tab, named "Watch task N", cannot type and leaves the agent's window size alone. tmux stays the owner. When the task has no live run,
  the bar shows Priory's error instead. The tab opens in the space shown when the panel asked. When
  that space is on another machine than Priory, the bar refuses and asks for a space on Priory's
  machine.
- **Opening.** A panel opens an ask in Asks with `setShared("priory.asks.open", { ask, at: Date.now() })`,
  and a task in the Task panel with `setShared("priory.task.open", { task, at: Date.now() })`. The
  Asks panel opens the ask once a read lists it, and leaves out a request made over 30 s before it
  started. The Priory bar opens or focuses the Task tab on each new `priory.task.open`, but not
  for one left from before it started.
- **Themes.** A panel draws with Priory's tokens, not Herdr's theme, so a theme looks the same in
  Herdr and in the browser. The adapter picks the light or dark value from Herdr's appearance. The
  Priory bar is part of Herdr's top bar, so it draws with Herdr's theme, as the tab bar beside it
  does.

## Delivering answers

`adapters/herdr/deliver` brings an outside ask's answer to the Claude session that asked, as
[asks.md](../daemon/asks.md) describes. A factory names it with
`[asks] deliver = "adapters/herdr/deliver"`, from a factory folder that holds the adapter. It needs
Python 3 and `hc`, and no other package.

It reads the ask's `session` as an hc terminal, optionally followed by a space and the Claude
session id. `priory ask` run in an hc terminal stores both. Then:

1. `hc get TERMINAL`: when the terminal is live and runs `claude`, and that `claude` runs the
   ask's Claude session, `hc send` types
   `A person answered ask <ID> (revision <N>): <answer>` into it, or
   `A person dismissed ask <ID> without an answer. Continue your task.` The script reads the
   session a `claude` runs from `~/.claude/sessions/<PID>.json`, where `<PID>` is the terminal's
   foreground process. An ask that names no Claude session skips this check.
2. Otherwise it finds the Claude session's transcript under `~/.claude/projects/` and reads the
   folder it ran in. It starts `claude --resume <session> <text>` there with
   `hc run --bg --keep --no-parent`, in a new tab named `ask-<ID>`, in the old terminal's
   `herdr.space` when hc still knows it.
3. With no Claude session id or no transcript, it exits 1, and the delivery is tried again, then
   fails on the desk.

A kept terminal whose `claude` exited runs a shell, so the script never types into it: the shell
would run the answer as a command. Nor does it type into a `claude` that runs another session,
such as a new conversation started in the same kept terminal. The check and the send are two
`hc` calls, so a `claude` that exits between them can still receive the text in its shell.

## Typing a compaction

The same script is the factory's `[asks] type` command, for a person's own session that asks for a
compaction (see [Compact from the session](../daemon/tools.md#compact-from-the-session)). A factory
names it with `type = "adapters/herdr/deliver"` under `[asks]`.

Each of `check`, `send` and `enter` reads the terminal with `hc get`, as above, but needs the Claude
session id, and never starts a session. `send` types only when hc shows the terminal `idle` or
`done`, and exits 75 for any other status. `hc send` itself waits up to a minute for a person's
unfinished draft.

## Opening runs in Herdr

`adapters/herdr/attach` opens a run in a kept hc terminal when the run starts, as
[sessions.md](../daemon/sessions.md#opening-a-run-when-it-starts) describes. The terminal waits in
the hc hub even while the Herdr app is closed, and shows in the Herdr space it is labelled with.
A factory names it with `[runs] attach = "adapters/herdr/attach"`, from a factory folder that holds
the adapter, and sets `attach = true` on each step to open. It needs Python 3 and `hc`, and no
other package.

The daemon runs it as `attach TASK RUN`. It reads the space from `PRIORY_HERDR_SPACE` in the
daemon's environment. Then:

1. `hc ls --label priory.run=RUN`: when a live terminal already carries the run's label, it exits 0
   and opens nothing.
2. Otherwise `priory attach --run RUN --read-only --print` checks that the run has a live pane. When
   it fails, the script prints its error and exits 1. `hc run --bg` alone would hide that error.
3. `hc run --bg --keep --no-parent --no-notify` starts `priory attach --run RUN --read-only`,
   labelled `herdr.space=<space>`, `herdr.tab=task-<TASK>`, `priory.task=<TASK>` and
   `priory.run=<RUN>`. The hc core starts it with its own environment, so the script passes the
   daemon's `PRIORY_DB`, `PRIORY_HOME` and `TMUX_TMPDIR` through `env`, and runs the daemon's
   own executable, `PRIORY_EXE`.

It exits 1 with `attach: hc is not on PATH; install herdr-core or fix the daemon's PATH` when the
daemon's `PATH` has no `hc`, and with `attach: set PRIORY_HERDR_SPACE to the Herdr space that shows
runs` or `attach: PRIORY_EXE is unset; run this from a daemon that sets it` when a variable is unset.
The daemon records each as a `factory.run.attach_failed` record.

The terminal only shows the run: use the Priory bar's attach to type. So it protects the run from no
budget, spend cap or urgent wake. Before it attaches, it sets its own size to 200x51, which keeps
the agent's 200x50 window under tmux's one-line status. A Herdr window that shows the terminal
resizes it, and with it the agent's window while no other client can type.

When the run ends, the terminal's program exits and leaves no shell: `--keep` gives a shell only
to a terminal whose program is lost with the hc core. The next run of the task opens a new
terminal in the same tab. The script does not check that `PRIORY_HERDR_SPACE` names a space that
exists.

## Writing another adapter

Another front end, such as Orca, gets Priory the same way:

1. Read the factory through the CLI's JSON or `serve`, with the fields that
   [Data each panel needs](../reference/herdr-panels.md#data-each-panel-needs) lists. Treat agent text in
   them as untrusted: render it as text or sanitized Markdown.
2. Act only through Priory's verbs, and send the revision with every answer.
3. Draw with the tokens in `design/tokens/`, resolved for the host's mode. Keep DESIGN.md's
   structure, state words and copy. Change density only through tokens, as the Herdr adapter's
   `--pane-*` tokens do.
4. Put the adapter in its own folder under `adapters/`, outside the Cargo workspace.

## What the core serves

The core serves every field that
[Data each panel needs](../reference/herdr-panels.md#data-each-panel-needs) lists:

- JSON: `priory desk`, `asks`, `tasks`, `queue`, `schedules` and `report show` take `--json`.
  [serve.md](../reference/serve.md) lists each field.
- HTTP: `priory serve` serves the same JSON, and `/api/events` waits for a change (R8.5).
- Snooze: `priory snooze` and the snooze fields; see [desk.md](../daemon/desk.md#snooze).
- Since you looked: `priory seen` stores a visit; see
  [desk.md](../daemon/desk.md#since-you-looked).
- Up next: each schedule's [next firing](../intake/schedule.md#next-firing), in `priory queue
  --json`.
- Queue: each row's `title`, the factory's `bands`, and `--expected` on `move`, `hold`, `release`
  and `cancel`; see [serve.md](../reference/serve.md#queue) and [runs.md](../daemon/runs.md#queue).
- Reports, picks and comments on rows: [reports.md](../daemon/reports.md).
- Ask fields: `what_waits`, `next_action`, `title` and a Markdown `text`, in
  [tools.md](../reference/tools.md#askquestion-options-recommendation-tradeoff-confidence-rationale-title-text-whatwaits-nextaction-command).
- Ask history: each ask's `revisions` and `refused` answers, in `priory asks --json`.
- Task detail: `priory task ID --json` prints a task's `brief`, `plan`, `pull_request` and runs.

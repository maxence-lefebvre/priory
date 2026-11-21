# The Herdr adapter

Priory's views render in a browser first (R8.4). The PRD also asks for a second view host that
renders the same views as an adapter (R8.6). The Herdr adapter is that host for
Herdr, a terminal workspace whose panes run JavaScript mods drawn natively
with gpui-kit. This page explains where the adapter lives, how it reads and changes the factory,
what data each panel needs, and how another front end would write its own adapter.
[DESIGN.md](../../DESIGN.md), under "Herdr adapter", holds the visual rules.

## Where it lives

The adapter lives in `adapters/herdr/` at the repository root, outside the Cargo workspace:

- `adapters/herdr/lib/priory.js`: the module every mod uses to read and act. It is the only code
  that runs `priory`.
- `adapters/herdr/mods/priory-tabs/`: the Priory bar. It sits left of Herdr's own tab bar, shows
  one button per aside and center tab in the layout, opens the Task tab, and attaches to a task's
  terminal. See [The layout](#the-layout).
- `adapters/herdr/mods/priory-desk/`: the Desk panel. See [The Desk panel](#the-desk-panel).
- `adapters/herdr/mods/priory-asks/`: the Asks panel. See [The Asks panel](#the-asks-panel).
- `adapters/herdr/mods/priory-queue/`: the Queue panel. See [The Queue panel](#the-queue-panel).
- `adapters/herdr/mods/priory-bonjour/`: the Bonjour panel. See
  [The Bonjour panel](#the-bonjour-panel).
- `adapters/herdr/mods/priory-costs/`: the Costs panel. See [The Costs panel](#the-costs-panel).
- `adapters/herdr/mods/priory-flows/`: the Flows panel. See [The Flows panel](#the-flows-panel).
- `adapters/herdr/mods/priory-task/`: the Task panel. See [The Task panel](#the-task-panel).
- `adapters/herdr/mods/priory-inbox/`: the Inbox panel. See [The Inbox panel](#the-inbox-panel).
- `adapters/herdr/mods/priory-tasks/`: the Tasks panel. See [The Tasks panel](#the-tasks-panel).
- `adapters/herdr/mods/priory-runs/`: the Runs panel. See [The Runs panel](#the-runs-panel).
- `adapters/herdr/layouts/frame.json`: Herdr's chrome around the view: the sidebar, the top bar
  with the Priory bar, 420 px wide, and Herdr's tab bar, the work body with Herdr's `main` block,
  and the status line.
- `adapters/herdr/lib/layout.js`: turns a view into a Herdr layout inside the frame, and finds a
  view's blocks in the layout Herdr shows. See [The layout](#the-layout).
- `adapters/herdr/layout`: reads `priory views --json` on its standard input and prints the
  layout for the `default` view.
- `adapters/herdr/install`: links the mods into Herdr's folder and writes the layout. See
  [Installing](#installing).
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

Herdr loads a mod's imports only from the mod's own folder, and it resolves symbolic links before it
checks. So each mod keeps a copy of `lib/priory.js` as its own `priory.js`, and a mod that draws
with Priory's colors keeps a copy of `lib/tokens.js`. The Desk and Queue panels also keep
`lib/add.js`, the Add action's view model, and `lib/add-form.js`, its form. The Desk and the
Priory bar keep `lib/layout.js`. A test fails when a copy differs from the
library. After you change a file in `lib/`, copy it into every mod that carries it.

The Costs, Flows, Inbox, Tasks and Runs panels keep the web view's own view models: `mods/priory-costs/view.js` is a
copy of `crates/priory-core/web/costs.js`, `mods/priory-flows/view.js` of `flows.js`,
`mods/priory-inbox/view.js` of `inbox.js`, `mods/priory-tasks/view.js` of `tasks.js`, and
`mods/priory-runs/view.js` of `runs.js`, beside its own copy of `tasks.js` for the facet filter. A test
fails when a copy differs from the web file. After you change one, copy it into its mod. Herdr's
engine has no `Intl`, so these files format numbers and dates by hand.

The adapter depends on no Priory crate, and no crate depends on it. It needs Node.js to install,
because `install` builds the layout with it, and for its tests.

## Installing

Installing needs Node.js. Pipe the views into `install` on the machine Herdr runs on:

```sh
priory views --json | adapters/herdr/install
ssh xps13 priory views --json | adapters/herdr/install   # Priory on another machine
```

`install` links each folder in `adapters/herdr/mods/` into Herdr's mods folder, and writes the
layout for the `default` view as `layouts/priory.json` in its layouts folder. Herdr's folder is
`$HERDR_APP_HOME`, or `~/.config/herdr-app`. The links point into your checkout, so a `git pull`
updates the mods, and Herdr reloads them when their files change. The layout is a file: run
`install` again after you change a view. Its root id ends in a hash of the views `install` read, such
as `priory-generated-00bc386a7e08`. `install` says `layout is up to date with the view files` when
the layout it replaces has the same hash, and `layout was older than the view files, and is
rewritten` when not. `priory check` warns, and still passes, when the layout in Herdr's folder on
its machine has another hash than the factory's views. A layout on another machine is not checked.

- `install --dry-run` prints each link it would make, then `write` and the layout, and changes
  nothing.
- `install --copy` copies each mod folder in place of linking it, and writes `.priory-installed`
  into the copy, holding the adapter's Git commit (`unknown` outside a checkout). Use it when
  Herdr's folder reaches another machine through Syncthing, which carries no links, such as a Herdr
  on Windows. A later `install --copy` replaces a folder that holds that marker, files that left
  the source included, and a link into this adapter. It stops on any other folder. A plain
  `install` stops on a copy, so the two never mix. A copy you made by hand, without the marker,
  is not the script's: after you check one is a copy of this adapter, mark it with
  `git -C adapters/herdr rev-parse HEAD > ~/.config/herdr-app/mods/priory-desk/.priory-installed`,
  one folder at a time, then run `install --copy`.
- A link that already points into this adapter stays as it is. The layout is written again on
  every run.
- `install` removes what an earlier install made of a mod this adapter no longer ships: a folder
  that holds `.priory-installed`, or a link into this adapter. It prints `remove` and the path, and
  `--dry-run` only prints. It leaves every other mod in Herdr's folder alone.
- Any other file or link at a mod's target stops the script before it changes anything. So does a
  `priory.json` that `install` did not write: a file whose JSON root id is not
  `priory-generated`, alone or with 12 hex digits after a hyphen, or a link elsewhere than to this adapter. Move that file away and run it
  again.
- Input that is not `priory views --json`, a node or panel Priory does not ship, or a `default`
  view with two `terminal` panels stops the script before it changes anything.
- `install` writes the layout beside its target and renames it over the target, so a failed write
  keeps the old layout.

## The layout

`lib/layout.js` lays the view's panels out in the frame's work body. One map, `PANELS`, says where
each panel opens and which mod shows it. The view's `row`, `column` and `tabs` nodes only order the
panels, and a view that places a panel twice is refused.

| Panel | Opens as | Mod |
| ----- | -------- | --- |
| `desk`, `asks`, `queue` | an aside | `priory-desk`, `priory-asks`, `priory-queue` |
| `reports` | a tab in the center | `priory-bonjour` |
| `workflow`, `costs`, `inbox`, `tasks`, `runs` | a tab in the center | `priory-flows`, `priory-costs`, `priory-inbox`, `priory-tasks`, `priory-runs` |
| `terminal` | the frame's `main` block, where Herdr shows the active tab, which is always in the center | none |

The work body is a row: the center, then each aside in the view's order, then Herdr's own panel.

- **The center** is the `center` stack of `main` and the wide panels, with `main` active. It is
  `main` alone when the view has no wide panel.
- **An aside** is a column `aside-NAME`, 360 px wide, between 280 and 700, holding the panel's
  block, whose id is the panel's name and whose `props` hold `{ "panel": NAME }`. The first aside
  starts shown, and the others hidden. Several can be open side by side. Herdr remembers which are
  open, as it does for its own Files and Changes.
- **Herdr's own Files and Changes** stay last, in the frame's `panel` column, as Herdr's `main`
  layout has them: hidden until the tab bar opens one, 368 px wide, with `files` active.

A view carries no sizes, so the layout has none beyond these.

The Priory bar shows a button for each aside, then one for each tab in the center, labelled with
its panel. An aside's button shows or hides that aside. A tab's button selects it in the center,
and the shown tab's button gives the center back to `main`. The bar scrolls sideways when its
buttons are wider than it. Before it opens the Task tab or a task's terminal, it brings `main`
into view. To toggle an aside from the keyboard, bind `layout.toggle:aside-NAME` in Herdr's
`keys.json`, such as `"layout:priory": { "ctrl-alt-d": "layout.toggle:aside-desk" }`. A
layout's bindings go under `layout:` and its name, here `priory`.

Herdr writes `gpui-kit.d.ts` and `jsconfig.json` into each mod folder it loads. Through the links,
they land in your checkout, and Git ignores them.

## Installing for a Herdr on another machine

The mods run on the machine that runs Herdr, and `priory` runs where the `priory` object of
`settings.json` says: its `machine` is the Herdr machine that runs the program, such as `"xps13"`.
Copy the mods with `install --copy` on the machine whose folder Syncthing shares, and set `machine`
in `settings.json`, which Herdr reads from the same folder, to the machine that runs Priory. Without
`machine`, a mod runs `priory` on the machine Herdr's window runs on, which is the Windows machine
and has no Priory.

## How it reads and acts

A panel reads the factory through Priory and acts through Priory's own verbs. It never opens the
state database and never writes a file in the factory.

- **Which Priory.** A mod runs the program that the `priory` object of Herdr's `settings.json`
  names, on the Herdr machine it names: `{"priory": {"binary": "~/.local/bin/priory-oss",
  "machine": "xps13", "factory": "/home/me/factory", "theme": "indigo", "web":
  "http://127.0.0.1:7070"}}`. Every field is optional.
  `theme` is `indigo`, `ink`, `parchment` or `abbey`, and defaults to `indigo`. `binary` defaults
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
  no factory to read, the panel draws its text unlinked. See [Links](#links).
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

## The Desk panel

`priory-desk` opens as a tab in the center stack, from the Priory bar. It follows DESIGN.md's
Herdr Desk: the count, the next thing, the room and intake banners, the saved views, then the rows.

- **What it reads.** `desk --all --json`, `asks --all --json`, `tasks --json` and
  `queue FACTORY --json`, read together, then again after each `priory wait`, even one that ends
  with the same revision, so it also catches changes that raise no revision, such as a worktree item following the disk. `--all` keeps the
  snoozed items for the Snoozed view; the panel leaves them out of everything else.
- **The count** counts the open asks that are not snoozed, then the oldest wait, the stopped items
  (`blocked`, `failed`, `prompt` and `delivery`) and `counts.snoozed`.
- **The next thing** is the oldest open ask that is not snoozed, with its recommendation and
  trade-off from `priory asks`, and its task's `progress.nextAction`.
- **Saved views.** Needs me lists Asks, Stopped, then Later: the asleep count and the desk's other
  items, without `queue` rows. Running, Asleep and All tasks list `priory tasks`. Snoozed lists the
  snoozed desk items with when each comes back.
- **A selected row** opens in place under the row. It shows its task's published progress and its
  actions.
- **Up next** shows `queue`'s `up_next`, under the list below 1024 px and in the left column from
  1024 px. A line opens its task in the Task panel. A schedule that starts a new task names no task,
  so its line opens nothing. **Since you looked** shows from 1024 px.
- **Since you looked** keeps the events the panel opened on, and adds each newer event to the top.
  The first time the panel draws the list, from 1024 px, it runs `priory seen --at` once, with
  the time it started its first read. A panel that stays narrower records no visit. `herdr-app check` runs no
  `priory` command at all.
- **Acting.** Snooze 1 h, Snooze until tomorrow 09:00 and Unsnooze run `priory snooze KIND ID`
  with `--for 1h`, `--until` or `--clear`. Resume runs `priory resume TASK` for a blocked task or an
  uncertain prompt, and Revive runs `priory revive TASK` for a failed one. Done runs
  `priory todo done ID` for a to-do, from its row or the next thing card.
  Read runs `priory inbox read SEQ` for a report, from its row, so the row leaves the desk.
  Open task opens the row's task in the [Task panel](#the-task-panel), and Attach asks the Priory
  bar with `priory.attach`; the next thing card offers both. Answer… opens the ask in the Asks
  panel: it sets `priory.asks.open` and brings the first block that shows `asks` into view. When
  the view has no `asks` panel, it opens or selects an Asks tab in `main`.
  Add, or `N`, opens the [Add form](#the-add-action).
- **Keys.** `J` and `K` move the selection, `Enter` opens the selected ask in Asks or the Asleep
  view from the asleep row, `S`
  snoozes the selected row for 1 h, or unsnoozes a snoozed one, `R` resumes or revives it, and `N`
  opens Add. The
  mod is focusable, so `pane.go` moves the keyboard onto it.
- **Width.** Below 1024 px, one column. From 1024 px, a 440 px column beside the list and the key
  hint bar. The list becomes the wide table with Recommends once it has 720 px of its own. The
  panel measures its own width, as gpui-kit has no container queries.

The panel draws agent text as plain text, apart from its [links](#links), and never parses it. Its view model, `view.js`, is pure:
the listings in, the count and rows out. `adapters/herdr/test/desk.test.js` runs it against JSON
captured from the CLI.

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

## The Asks panel

`mods/priory-asks/` is the Asks panel. It replaces Fleet's `fleet-asks`.
DESIGN.md's "Herdr adapter" section holds its visual rules; this section says how it works.

- **Files.** `view.js` turns `priory asks --all --json` into saved views, sections, rows and the
  ask detail, with no drawing and no IO. `main.js` draws them with gpui-kit and runs the verbs.
  The node tests in `test/asks.test.js` check `view.js` against JSON captured from a real
  `priory asks --all --json`.
- **Reading.** The panel reads `priory asks --all --json`, then waits with `priory wait` and reads
  again. With each read and each new selection, it reads the selected ask's task with
  `priory task ID --json`. When a call fails, it keeps the last list and shows "Reconnecting to priory" with the age
  of its last update, and the options stay disabled until it reconnects.
- **Routed asks.** An open ask with `routed_to` shows "with <lead>" in its row, and its detail says
  the lead is answering and that the person can still answer first. An answered ask shows
  `answered_by` as "by agent:<lead>" or "by a person", in its row, its decided line and the
  History. The Desk panel shows the same mark under a routed ask's question. The helpers
  `routedMark`, `routedNote` and `answeredBy` live in `lib/priory.js`, as in the web view's
  `asks.js`; a test keeps the two word for word.
- **List.** The saved views are Open, Snoozed, Answered and All, each with its count. The facet
  chips Command, Lane, Band, Risk and Task narrow every view. Band lists the bands in the factory's
  order from the listing's `bands`, and puts an ask with no band, such as an outside ask, under
  `none`. Risk lists `irreversible`, `red`, `yellow`, `green`, then `none` for an ask whose `risk`
  is empty, and counts an ask under each risk it carries. A row shows its tile, task key, step,
  age, question, and what it recommends, its answer or why it was withdrawn. An ask whose answer
  lets an irreversible step run shows its History line under the context line in its detail.
- **Width.** Below 720 px the selected row opens under itself: the decide part, the section chips
  and Why. A section chip opens that section with the answer bar under it, since gpui-kit has no
  sticky positioning. From 720 px the list and the detail sit side by side. The decide part stays
  pinned at the top, and the sections scroll under it.
- **Answering.** An option answers after a 4 s Undo, with `priory answer --expected N`. An option
  that runs the ask's command arms on the first press, shows the command word by word under it,
  and runs on a second press within 4 s, with `--run`. "Write an answer…" sends free text the same
  way. Dismiss runs `priory dismiss` at once. A step's ask and a hold's ask take only their
  options, so they offer neither "Write an answer…", its `w` key, nor Dismiss.
- **Refused answers.** When a verb fails, the panel reads the asks again and says why on the ask:
  the question changed to another revision, someone else answered, the ask was withdrawn, or the
  CLI's error. When the ask has left the view, the reason shows in a strip under the header
  instead. A refused answer goes back into the text field, so the person can send it again. While
  one answer, snooze or dismissal is in flight, the panel starts no other.
- **Opened from the Desk.** A new `priory.asks.open` selects its ask, clears the facet chips, and
  moves to All when the current view hides the ask, such as a snoozed one.
- **Snooze.** "Snooze 1 h" and the overflow menu's "Snooze until tomorrow 09:00" run
  `priory snooze ask ID`. They show only on an ask that belongs to a task. On such an ask, the menu's
  "Open task" opens the task in the [Task panel](#the-task-panel), and "Attach to task" asks the
  Priory bar to attach, as [Attaching](#how-it-reads-and-acts) describes.
- **Outside asks.** An ask from outside Priory lists and answers like the others, by its name. Its
  command is a string that never runs, so its options answer after the Undo.
- **Keys.** `J` and `K` move, `Enter` opens a section below 720 px, `1` to `9` press an option
  (twice for a command), `W` writes an answer, `S` snoozes for 1 h, `[` and `]` move between
  sections and `Esc` closes. Single keys never fire while the text field has the keyboard.
- **Agent text.** The options, trade-off and what waits draw as plain text. The question and text
  draw with their [links](#links).

The detail shows the sections Why, Brief, Plan, PR, Trace, History and Command. Brief shows the
request and the `groom` step's result, Plan the `plan` step's result or else the task's
`progress`, PR the pull request's title, branches, state and checks, then the task's stack when it
has two or more pull requests (see [Stacks](#stacks)), and Trace each run with its
span, outcome and tokens. A section the task lacks is left out with its chip, as DESIGN.md says.
History lists every revision and refused answer of the ask, and its answer, newest first. A refused
answer shows on a warn tile. Briefs, plans and results draw as plain text, like the agent's text.

## Stacks

A task can own a stack of pull requests, each based on the one below (see
[Stacks in routing.md](../daemon/routing.md#stacks)). `prStack` in `lib/priory.js` turns a task's
`stack` from `priory tasks --json` into what the panels show:

- A Desk or Queue row whose task owns two or more pull requests says how many, as `3 PRs`, after
  its state.
- The Asks detail's PR section lists the stack under the latest pull request, bottom first: each
  one's number as a link, its branch and base, its state, and the checks of the latest one. The
  lowest open one reads "open, next to merge".
- A task with one pull request keeps the single PR look.

The web view keeps the same `prStack` in `crates/priory-core/web/stack.js`. A test in
`test/priory.test.js` keeps the two the same.

## The Queue panel

`mods/priory-queue/` is the Queue panel. It replaces Fleet's
`fleet-queue`. DESIGN.md's "Herdr adapter" section holds its visual rules; this section says how it
works.

- **Files.** `view.js` turns `priory queue DIR --json` into saved views, sections, rows, Up next,
  reorder places and Move targets, with no drawing and no IO. `main.js` draws them with gpui-kit and
  runs the verbs. The node tests in `test/queue.test.js` check `view.js` against JSON captured from a
  real `priory queue DIR --json`.
- **Reading.** The panel reads `priory queue DIR --json`, with the factory that settings name, and
  `priory desk --json` for whether starts are paused and what the queue is short of, and
  `priory tasks --json` for each task's stack of pull requests, which the queue does not carry. Then it waits
  with `priory wait` and reads again. When a call fails, it keeps the last read and shows
  "Reconnecting to priory" with the age of its last update, and the actions stay disabled.
- **List.** The saved views are Next, Held and Last day, each with its count, and All. Next shows Up
  next, Running, each band in start order, Held, and Last day folded. All shows the same sections
  without Up next, and Last day open. The Lane and Band chips narrow the waiting and held tasks.
  Running runs and ended tasks carry no lane or band, so a chip hides them.
- **Why it waits.** Every waiting row shows its place in its band and why it has not started: starts
  are paused, it waits for room while the queue is stalled on it, it starts when the runner has room,
  or it starts after the task before it. A held row shows its hold's reason and each condition, met
  or not.
- **Up next.** The next three runs, queued, held or scheduled, each with when and why. A line opens
  its task when the queue lists it. A schedule that starts a new task, or wakes an asleep one, names
  no row, so its line does not open.
- **Width.** Below 720 px the selected row opens under itself. From 720 px the list and the opened
  row sit side by side, and the row adds how the order works.
- **Acting.** An opened waiting row offers Move…, Hold… and Cancel…. Move… runs `priory move`, to
  the top of the band or into any other band of the factory, even an empty one. Hold… asks for a reason and runs
  `priory hold TASK --reason`. A held row offers Release and Cancel…, and a running row Attach and
  Cancel…. Every opened row offers Open task, which opens the task in the
  [Task panel](#the-task-panel). Cancel… asks once more, then runs `priory cancel`. The header's menu runs `priory pause`
  or `priory unpause`. Move, Hold, Release and Cancel send `--expected` with the revision of the
  read the panel drew, so priory refuses them once the queue changed. When a verb fails or is
  refused, the panel says why on the task.
- **Reordering.** The grip in each waiting row's trailing lane drags the task onto another row of
  its band, and leaving the pane ends the drag without a move. `Shift J` and `Shift K` move the
  selected task down or up one place. Both run
  `priory move TASK --before OTHER` or `--after OTHER`, and never leave the band.
- **Keys.** `J` and `K` move the selection, and `N` opens Add. Below 720 px, `Enter` opens or closes the row and `Esc`
  closes it. `Esc` also closes an open menu.
- **Agent text.** Reasons, hold reasons and keys draw as plain text.

Each row shows its task's title under its key and step. The header's Add, or `N`, opens the
[Add form](#the-add-action); the new task opens in place once the queue lists it. The opened row's
History is left out: the panel would need each task's events.

## Links

Each panel links the factory's keys in the text the web view links: an ask's question and text, a
task's title and a report row's text. A text shows its links once: in the panel's detail where it
has one, else in its row. A row drawn as a button that truncates to one line keeps plain text.

| Panel | Linked text |
| ----- | ----------- |
| Desk | The next thing's question, and each row's text: an ask's question, a task's title or reason |
| Asks | The opened ask's question, and its text under Why |
| Queue | The opened row's title |
| Bonjour | Each report row's text, and each pick card's text |

A report and the Queue name no repository, so only the factory's own links apply there, not the
built-in `#n` link.

## The Add action

The Desk and Queue panels share one Add form, `lib/add-form.js`, over the view model `lib/add.js`.
It holds one field, then chips for workflow, band and lane. The band chips are the factory's bands
from `priory queue --json`, and the workflow chips its `workflows`. A queue read without the
factory folder lists no workflows, so the chips then offer the workflows the listed tasks carry.
The lane chips are the lanes the listed tasks carry. An unpicked chip leaves the
value to the factory's [`[request]`](../reference/factory.md#request) table, as `priory add` does.

`Enter` runs `priory add FACTORY -- TEXT` with a flag for each picked chip, and the form shows
"Added KEY." The Desk then selects the task in All tasks, and the Queue opens its row in Next, once
the next read lists it. When priory refuses, the form says why and keeps the text. `Esc` closes the
form. The panel's single-key shortcuts stay off while the field has focus.

## The Bonjour panel

`mods/priory-bonjour/` is the Bonjour panel, the morning dossier. The Priory bar opens it as a tab
in the center stack. It replaces Fleet's `fleet-bonjour`. DESIGN.md's "Herdr adapter" section holds
its visual rules; this section says how it works.

- **Files.** `view.js` turns `priory report show --json` into the toolbar, the greeting, sections,
  rows, picks and threads, with no drawing and no IO. `main.js` draws them with gpui-kit and runs
  `priory report pick` and the comment verbs. The node tests in `test/bonjour.test.js` check `view.js` against JSON
  captured from a real `priory report show --json`.
- **Reading.** The panel reads the report written last, or the one its arrows reached, and
  `priory report list`, then waits with `priory wait` on the state's revision and reads again. When no task has written a report yet, it says so and waits for
  one. When a call fails, it keeps the last report and shows "Reconnecting to priory" with the age
  of its last update, and the picks stay disabled until it reconnects.
- **Sections.** The greeting, then the report's sections in the report's order. The panel knows no
  section by name. A section shows its `count`, or its number of rows when the report gives no
  count. A row shows its state tile, or a page icon when it has no `state`, then its key, text and
  meta, and its owner or time on the right. A time today shows its clock, an earlier one its date.
- **Toolbar.** Older and newer arrows around the report's date, "n of m agreed" with a bar, Next
  pick, which selects the next pick to agree and scrolls to it, and Search. The arrows walk
  `priory report list` and read that task's report with `priory report show TASK`; the newest one
  is followed as it changes, an older one stays until an arrow moves on. A priory without
  `report list` leaves both arrows off.
- **Search.** Search opens a field under the toolbar. Its text keeps the rows whose key, text, meta,
  owner or comments hold it, ignoring case, with "3 found" beside it. Closing it clears the text.
- **Views.** All sections, and Picks, which keeps only the rows with a pick to agree. From 1024 px
  the views and the sections sit in a 200 px list on the left; below, they are chips under the
  toolbar. A section in that list scrolls to it.
- **Picking.** An open pick is a segmented control of its options, with a `check` on the
  recommended one and "Recommends Take" beside it. A press waits out a 4 s Undo, since the pick wakes the
  report's agent, then runs `priory report pick PICK OPTION --expected REVISION` with the report's
  revision the person read. While one pick is in flight, the panel starts no other. When the report
  stops offering the pick during the Undo, because another task wrote a report or someone picked
  first, the panel cancels the pick and says so.
- **Refused picks.** When `report pick` fails, the panel reads the report again and says why on the
  row: someone already picked, the report changed to another revision, the report no longer holds
  the pick, or the CLI's error. When the row has left the view, the reason shows in a strip under
  the toolbar instead. A pick that Priory took stays sent even when the read after it fails. A read
  older than the one shown is dropped, so a slow read never rolls the report back.
- **Agreed picks.** A chosen pick folds to one line: its key, the choice and, once the workflow
  applied it, its `applied` text, else "Waiting for the workflow to apply it." with Change. Change
  reopens the segmented control with the choice solid, and Keep folds it again. Another option goes
  through the same Undo and `priory report pick`, which records the change until the workflow
  applies the choice.
- **Threads.** A row's threads sit under it. An open thread shows each comment's author, time and
  text, with Reply and Resolve; a resolved one folds to one line with Reopen. The selected row, when
  it has an id, ends with a comment field; a pick card offers Comment to select it. Comment runs
  `priory report comment ROW --expected REVISION TEXT` with the report's revision the person read,
  and Reply runs `priory report reply`. Priory gives a row id's new thread to the newest report that
  holds the row, so an older report offers Reply, Resolve and Reopen but no new thread. A refused comment says why under the row and keeps the text
  in the field. Rows that share an id share their threads.
- **Keys.** `J` and `K` move between rows, `N` goes to the next pick, `1` to `9` press an option of
  the selected pick, and `Esc` undoes a pick still waiting out its Undo.
- **Agent text.** Every row's meta, owner and the greeting draw as plain text. Its text draws with
  its [links](#links). A press on linked text stays there, so a link opens without selecting the
  row; the rest of the row still selects it.

The web view's Reports panel draws the same report with the same view model, so Herdr has no
separate Reports panel: Bonjour is it.

## The Costs panel

`mods/priory-costs/` is the Costs panel. The Priory bar opens it as a tab in the center stack.
DESIGN.md's "Costs panel" section holds its visual rules; this section says how it works.

- **Files.** `view.js`, a copy of the web view's `costs.js`, turns the two reports into tiles, bars,
  the split table and each task's time, with no drawing and no IO. `main.js` draws them with
  gpui-kit. The node tests in `test/costs.test.js` check `view.js` against JSON captured from a real
  `priory costs --json` and `priory report attention --json`.
- **Reading.** The panel reads `priory costs FACTORY --by SPLIT --json` and
  `priory report attention --json` together, then reads them again every minute: they carry no
  revision to wait on, and their window ends at the time of the read. When a call fails, it keeps the last read, says why, and shows "Costs as
  read" with the time of that read. It shows that line too while the read is over a minute old.
- **Views.** From 1024 px Costs and Attention sit side by side, with a hairline between them.
  Below, the header's Costs and Attention chips pick one.
- **Costs.** This week's cost, merges and cost per merge against last week, what is not in the
  cost, one bar per day of both weeks, and this week split by workflow, agent, model, machine or
  repo. A split chip reads the costs again with that `--by`. While priory is unreachable, the
  chips are disabled.
- **Attention.** Your minutes per merge, the ask waits and where each task's time went.

## The Inbox panel

`mods/priory-inbox/` is the Inbox panel. The Priory bar opens it as a tab in the center stack.
DESIGN.md's "Inbox" section holds its visual rules; this section says how it works.

- **Files.** `view.js`, a copy of the web view's `inbox.js`, picks each group's route action and
  words the line for events no task ever owned. `main.js` reads and draws.
- **Reading.** The panel reads `priory inbox --unrouted --json` and follows its revision with
  `priory wait`. Its one view, Unrouted, counts the events of a task's resources.
- **Groups.** Each resource whose events reached no task shows its newest owner with the owner's
  state tile, its 5 newest events and how many are older. Pressing the owner opens it in the
  [Task panel](#the-task-panel). A line under the groups counts the events no task ever owned,
  per source.
- **Routing.** Each group has one button, by its owner's state. Subscribe task N runs
  `priory subscribe N RESOURCE --expected REVISION`, with the revision the panel read, for an
  active owner. Revive task N runs `priory revive N` for a
  failed or dropped one. Request a task runs `priory add FACTORY --workflow W -- TEXT` with the
  group's `request_text` for a done one, so the new task owns the resource. A note under the
  group says what the verb did, or why priory refused it. One verb runs at a time, and after one
  succeeds the buttons stay disabled until a read shows its result.

## The Tasks panel

`mods/priory-tasks/` is the Tasks panel. The Priory bar opens it as a tab in the center stack.
DESIGN.md's "Tasks" section holds its visual rules; this section says how it works.

- **Files.** `view.js`, a copy of the web view's `tasks.js`, splits the tasks into saved views and
  filters them by facet. `main.js` reads and draws.
- **Reading.** The panel reads `priory tasks --json` and follows its revision with `priory wait`.
- **Views.** Active lists the tasks not done, failed or dropped, in start order. Blocked lists the
  blocked ones. Ended lists the done, failed and dropped ones, newest first. Each view's chip
  shows its count.
- **Facets.** State, Lane and Workflow chips each open a menu of every task's values, A to Z, with
  counts, as the web's do. A picked chip shows its value, and pressing it again clears it. A task
  with no lane shows under `none`. The view chips count the tasks the picked values keep.
- **Rows.** Each row shows the task's tile, key, workflow, step, state and how long it has been in
  that state (a date after a day), then its title, then its lane and band, or the reason of a
  blocked, held, failed or dropped task, and its batch. Pressing a row opens it in the [Task panel](#the-task-panel).

## The Runs panel

`mods/priory-runs/` is the Runs panel. The Priory bar opens it as a tab in the center stack.
DESIGN.md's "Runs" section holds its visual rules; this section says how it works.

- **Files.** `view.js`, a copy of the web view's `runs.js`, gives each run its state, saved views,
  facets, duration and tokens. `tasks.js`, a copy of the web's, filters by facet. `main.js` reads
  and draws.
- **Reading.** The panel reads `priory runs --json`, every run, and follows its revision with
  `priory wait`.
- **Views.** Recent lists the live runs and those started in the last 7 days, newest first, as
  `/api/runs` does, then a "Show 3 older runs" button that lists the rest. Its count includes
  them. Live lists the runs still going.
- **Facets.** Outcome, Agent and Step, as the Tasks panel's. Outcome holds both how a run ended
  and its result's outcome, so `moved` and `passed` both find a moved run that passed.
- **Rows.** Each row shows the run's state tile, task key, step, state and how long ago it
  started (a date after a day), then its agent and model, or "no agent", then its id, outcome, duration and tokens. Pressing a row
  opens its task in the [Task panel](#the-task-panel), whose History lists the task's runs.

## The Flows panel

`mods/priory-flows/` is the Flows panel. The Priory bar opens it as a tab in the center stack.
DESIGN.md's "Flows panel" section holds its visual rules; this section says how it works.

- **Files.** `view.js`, a copy of the web view's `flows.js`, lays out each workflow and gives each
  node and edge its outline as an SVG path. `path.js` turns those paths into gpui-kit `PathBuilder`
  calls, scaled to the pane. `main.js` paints them. The node tests in `test/flows.test.js` check
  both against JSON captured from a real `priory flows DIR --json`.
- **Reading.** The panel reads `priory flows FACTORY --json` once, as the web view reads
  `/api/flows`: the factory's workflows change only when its files do. Each irreversible step's
  history is as of that read. "Read again" in the header reads it again. When the read fails, the panel says why and offers Retry.
- **Drawing.** The key, then each workflow: its name and step count, the drawing at most 560 px
  wide, each irreversible step's history when the state was read (see
  [serve.md's Flows](../reference/serve.md#flows)), the paths that reach an irreversible step with
  no person's decision, and "Steps as text", which lists every step and where each outcome leads.

## The Task panel

`mods/priory-task/` is the Task panel: one task in detail, since Herdr has no task view. The
Priory bar opens it as a tab in the center stack when another panel sets `priory.task.open`.

- **Files.** `view.js` turns `priory task ID --json` into the header, the title, and the Asks,
  Pull request and History sections, with no drawing and no IO. `main.js` draws them. The node
  tests in `test/task.test.js` check `view.js` against JSON captured from a real
  `priory task ID --json`, and draw the panel with the fake kit.
- **Reading.** The panel reads the task that `priory.task.open` names, then waits with
  `priory wait` and reads again. When another task opens, it stops that loop and starts one for
  the new task. When a call fails, it keeps the last read and shows "Reconnecting to priory" with
  its age.
- **What it shows.** The header holds the task's state tile, key and state, then Watch and Attach. Then the
  title with its [links](#links), the workflow, step, lane and band, and the task's reason. Asks
  lists the task's asks, open ones first. Pull request shows the latest pull request's title as a
  link, its branches and state, its checks, and the task's stack (see [Stacks](#stacks)). History
  lists the last 10 runs, newest first, each with its step, outcome and start time. A section the
  task lacks is left out.
- **Acting.** Watch asks the Priory bar for a read-only terminal, and Attach for one that types,
  as [Attaching](#how-it-reads-and-acts) describes. Watch comes first, since it cannot change the
  run. Answering stays in the Asks panel.

## Data each panel needs

Each list below names the fields a panel reads. Times are RFC 3339 strings. A field in *italics*
does not exist in Priory yet.

### Every item that waits on a person

Asks and Desk items (blocked tasks, failed tasks, uncertain prompts) share these fields:

- `id`, `kind` (`ask`, `blocked`, `failed`, `prompt`), `task` (id and key), `step`, `lane`,
  `since`.
- `snoozed_until`: when the item comes back on its own, or null.
- `snoozed_at`: when a person snoozed it. A meaningful change after this time brings the item
  back early.
- `back_early`: null, or `{ "at": TIME, "reason": "new revision" }`. Reasons are
  `new revision`, `task state changed` and `new failure`.

A snoozed item leaves the Needs me count, the Open count and the Desk badge. The agent keeps
waiting: snoozing changes what a person sees, never what the daemon does. `priory snooze KIND ID
--for 1h`, `--until TIME` and `--clear` set and clear it.

### Desk

- The open asks and stopped tasks above, oldest first, without the snoozed ones.
- The room banner: what is short, by how much, since when, and the waiting task. `priory desk`
  prints it today.
- Counts: asleep tasks, reports waiting for review, snoozed items.
- Each task's `progress` from `priory tasks --json`: its goal, acceptance criteria, latest change
  and next action, as its agent last published them.
- Since you looked: the events since the person's last visit, each with task, what happened and
  when, from `last_visit` and `since_you_looked`. `priory seen` stores the visit.

### Asks

- Every ask with its `state` (`open`, `answered`, `withdrawn`), `revision`, `question`,
  `options`, `recommendation`, `tradeoff`, and for a command ask the command line, its target and
  the revision it is bound to (R6.8).
- An answered ask adds `answer`, `answered_by` and `answered_at`; a withdrawn one adds `withdrawn_reason`,
  `withdrawn_why`, the reason in words that the view shows, and `withdrawn_at`.
- The detail adds the agent's text and the run that asked, `what_waits` and `next_action`, and
  the ask's `revisions` and `refused` for its history. `priory task ID --json` adds the task's
  `brief`, `plan`, `pull_request` and `runs`.

### Queue

- The waiting tasks in start order (R4.4), each with `band`, its position in the band, `state`,
  and why it has not started: no room, a band above it, or its hold and the hold's condition.
- The running runs, with run id, step and start time.
- The tasks ended in the last day.
- Up next: the next three runs, queued or scheduled, each with when it should start and why, from
  `queue`'s `up_next`.
- A title for each task: its TL;DR, else the first line of its request.
- The factory's bands, so Move… can offer a band that holds no task now.

### Bonjour

Bonjour draws a report that a factory's bonjour workflow writes. The panel knows no section by
name: it draws the sections in the order the report gives them.

```json
{
  "date": "2026-11-20",
  "since": "2026-11-19T18:00:00+01:00",
  "greeting": "Bonjour, Max",
  "sections": [
    {
      "id": "sentry",
      "title": "Sentry",
      "count": 3,
      "rows": [
        {
          "id": "fnd-2409",
          "key": "FND-2409",
          "text": "Release build pauses on a DWARF breakpoint at app.rs:216",
          "meta": "41 events, 3 users since 18:00",
          "state": null,
          "owner": null,
          "pick": {
            "id": "2026-11-20_sentry_fnd-2409",
            "options": ["Take", "Later", "Drop"],
            "recommended": "Take",
            "chosen": null,
            "applied": null
          },
          "comments": []
        }
      ]
    }
  ]
}
```

- A section has a `title`, an optional `count` and `rows`. A row has a `text`, and optionally a
  `key`, `meta`, a task `state` for its tile, an `owner` ("on Max"), a time, a `pick` and
  `comments`.
- A pick records the person's choice with `priory report pick ID OPTION --expected REVISION`.
  The workflow applies it and fills `applied`. [reports.md](../daemon/reports.md) holds the rules,
  and `docs/schemas/report.json` the schema.
- A row with an `id` takes comments: `priory report comment ROW --expected REVISION TEXT`, then
  `reply`, `resolve` and `reopen` by comment number. Its `comments` lists its threads, each with
  its replies, and a comment wakes the report's agent. See
  [reports.md](../daemon/reports.md#comment-on-a-row).
- Max's Rayon factory lists Reading, Slack notes, Sentry and Backlog, then To agree, Blocked and
  Moved.

### Costs and Flows

- Costs: `priory costs --json` and `priory report attention --json`, the objects `/api/costs` and
  `/api/attention` serve; see [serve.md's Costs](../reference/serve.md#costs) and
  [Attention](../reference/serve.md#attention).
- Flows: `priory flows --json`, the object `/api/flows` serves; see
  [serve.md's Flows](../reference/serve.md#flows).

### Tasks

- `priory tasks --json`, the object `/api/tasks` serves; see
  [serve.md's Tasks](../reference/serve.md#tasks).

### Runs

- `priory runs --json`, the object `/api/runs?all=1` serves; see
  [serve.md's Runs](../reference/serve.md#runs).

### Inbox

- `priory inbox --unrouted --json`, the object `/api/inbox?unrouted=1` serves; see
  [serve.md's Unrouted](../reference/serve.md#unrouted).

## Writing another adapter

Another front end, such as Orca, gets Priory the same way:

1. Read the factory through the CLI's JSON or `serve`, with the fields above. Treat agent text in
   them as untrusted: render it as text or sanitized Markdown.
2. Act only through Priory's verbs, and send the revision with every answer.
3. Draw with the tokens in `design/tokens/`, resolved for the host's mode. Keep DESIGN.md's
   structure, state words and copy. Change density only through tokens, as the Herdr adapter's
   `--pane-*` tokens do.
4. Put the adapter in its own folder under `adapters/`, outside the Cargo workspace.

## Core gaps

The panels still need these from Priory's core. Each is a separate slice.

1. Earlier reports listed by date, for the Bonjour panel's older and newer arrows.
2. A `report pick` that replaces a choice the workflow has not applied yet, for Change.

The core already serves the rest of the fields above:

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
  [tools.md](../daemon/tools.md).
- Ask history: each ask's `revisions` and `refused` answers, in `priory asks --json`.
- Task detail: `priory task ID --json` prints a task's `brief`, `plan`, `pull_request` and runs.

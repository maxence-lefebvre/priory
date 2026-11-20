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
- `adapters/herdr/mods/priory-tabs/`: the Priory bar. It sits left of Herdr's own tab bar, opens or
  focuses the Desk and Bonjour tabs, shows or hides the Asks and Queue panels, and attaches to a
  task's terminal.
- `adapters/herdr/mods/priory-desk/`: the Desk panel. See [The Desk panel](#the-desk-panel).
- `adapters/herdr/mods/priory-asks/`: the Asks panel. See [The Asks panel](#the-asks-panel).
- `adapters/herdr/mods/priory-queue/`: the Queue panel. See [The Queue panel](#the-queue-panel).
- `adapters/herdr/mods/priory-bonjour/`: the Bonjour panel. See
  [The Bonjour panel](#the-bonjour-panel).
- `adapters/herdr/layouts/priory.json`: Herdr's default layout with the Priory bar in the top bar,
  and Asks and Queue in the side panel stack, 420 px wide by default, between 280 and 900. Desk and
  Bonjour open as tabs in the center, from the Priory bar.
- `adapters/herdr/install`: links the mods and the layout into Herdr's folder. See
  [Installing](#installing).
- `adapters/herdr/test/`: `node --test` tests for the module and the install script, run against a
  fake `priory`. CI runs them with `node --test adapters/herdr/test/*.test.js`.
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
`lib/add.js`, the Add action's view model, and `lib/add-form.js`, its form. A test fails when a copy differs from the
library. After you change a file in `lib/`, copy it into every mod that carries it.

The adapter depends on no Priory crate, and no crate depends on it. It needs Node.js only for its
tests.

## Installing

`adapters/herdr/install` links each folder in `adapters/herdr/mods/` into Herdr's mods folder and
`layouts/priory.json` into its layouts folder. Herdr's folder is `$HERDR_APP_HOME`, or
`~/.config/herdr-app`. The links point into your checkout, so a `git pull` updates the mods, and
Herdr reloads them when their files change.

- `install --dry-run` prints each link it would make and changes nothing.
- A link that already points into this adapter stays as it is.
- Any other file or link at a target stops the script before it links anything. Move that file away
  and run it again.

The layout names the Asks and Queue panel mods, and the Priory bar opens the Desk and Bonjour
mods. Until a mod is installed, its block or tab stays empty.

Herdr writes `gpui-kit.d.ts` and `jsconfig.json` into each mod folder it loads. Through the links,
they land in your checkout, and Git ignores them.

## How it reads and acts

A panel reads the factory through Priory and acts through Priory's own verbs. It never opens the
state database and never writes a file in the factory.

- **Which Priory.** A mod runs the program that the `priory` object of Herdr's `settings.json`
  names, on the Herdr machine it names: `{"priory": {"binary": "~/.local/bin/priory-oss",
  "machine": "xps13", "factory": "/home/me/factory", "theme": "indigo"}}`. Every field is optional.
  `theme` is `indigo`, `ink`, `parchment` or `abbey`, and defaults to `indigo`. `binary` defaults
  to `~/.local/bin/priory-oss`, and a leading `~` means the home folder on Priory's machine. A name
  without a slash is looked up on the `PATH`. `machine` defaults to the machine Herdr runs on. `factory` is the folder
  that `queue` and `schedules` read for up next. The program must find its state file, so set
  `PRIORY_DB` in the environment, or name a wrapper script that sets it.
- **Reading.** A mod runs the CLI with Herdr's `exec` and parses its JSON: `priory desk --all --json`,
  `asks --all --json`, `tasks --json`, `queue --json`, `schedules --json`, `links --json` and
  `report show --json`. Each answer carries the state's `revision`. `report show --json` wraps the
  report as `{"revision": STATE, "report": {...}}`: the outer `revision` is the state's, and the
  report's own `revision` is the one `report pick --expected` takes. Then the mod runs
  `priory wait --since REVISION`, which returns as soon as the state changes, or after 25 s, and
  reads again. It keeps the last good answer, with its age, when a call fails. `priory serve`
  answers the same JSON over HTTP, and its `/api/events` waits for a change as `priory wait` does:
  see [serve.md](../reference/serve.md).
- **Acting.** Every button runs a CLI verb: `answer --expected N [--run] -- ASK TEXT`,
  `dismiss --expected N ASK`, `snooze`, `say`, `report pick PICK OPTION --expected N`,
  `report comment ROW --expected N TEXT`, `report reply|resolve|reopen COMMENT`, `seen`,
  `resume`, `revive`, `hold`, `release`, `move`, `cancel`, `pause`, `unpause` and
  `add FACTORY [--workflow W] [--band B] [--lane L] -- TEXT`. An answer always carries the revision the person read, so a
  changed question refuses it (R6.3).
- **Safety.** `priory.js` passes every value as its own argument, never through a shell, and never
  reads text output except `attach --print`. A mod draws agent text as plain text.
- **Autolinks.** `autolink(text, links, repo)` in `priory.js` splits text into spans, each
  `{ text }` or `{ text, url }`, with the rules `links()` reads from `priory links --json`. It
  returns the same spans as the web view, and links only to `https:` URLs. No panel draws links
  yet.
- **Attaching.** A panel asks the Priory bar to attach with
  `setShared("priory.attach", { task, at: Date.now() })`. The bar runs
  `priory attach TASK --print` and opens a terminal tab that runs the printed command, which starts
  a tmux client on the task's live run (R5.6). tmux stays the owner. When the task has no live run,
  the bar shows Priory's error instead. The tab opens in the space shown when the panel asked. When
  that space is on another machine than Priory, the bar refuses and asks for a space on Priory's
  machine.
- **Themes.** A panel draws with Priory's tokens, not Herdr's theme, so a theme looks the same in
  Herdr and in the browser. The adapter picks the light or dark value from Herdr's appearance. The
  Priory bar is part of Herdr's top bar, so it draws with Herdr's theme, as the tab bar beside it
  does.

## The Desk panel

`priory-desk` opens as a tab in the center stack, from the Priory bar. It follows DESIGN.md's
Herdr Desk: the count, the next thing, the room banner, the saved views, then the rows.

- **What it reads.** `desk --all --json`, `asks --all --json`, `tasks --json` and
  `queue FACTORY --json`, read together, then again after each `priory wait`, even one that ends
  with the same revision: a snooze ends by the clock, which no revision records. `--all` keeps the
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
  1024 px. **Since you looked** shows from 1024 px.
- **Since you looked** keeps the events the panel opened on, and adds each newer event to the top.
  The first time the panel draws the list, from 1024 px, it runs `priory seen --at` once, with
  the time it started its first read. A panel that stays narrower records no visit. `herdr-app check` runs no
  `priory` command at all.
- **Acting.** Snooze 1 h, Snooze until tomorrow 09:00 and Unsnooze run `priory snooze KIND ID`
  with `--for 1h`, `--until` or `--clear`. Resume runs `priory resume TASK` for a blocked task or an
  uncertain prompt, and Revive runs `priory revive TASK` for a failed one.
  Attach asks the Priory bar with `priory.attach`. Answer… opens the ask in the Asks panel: it sets
  `setShared("priory.asks.open", { ask, at: Date.now() })` and shows the side panel on Asks.
  Add, or `N`, opens the [Add form](#the-add-action).
- **Keys.** `J` and `K` move the selection, `Enter` opens the selected ask in Asks or the Asleep
  view from the asleep row, `S`
  snoozes the selected row for 1 h, or unsnoozes a snoozed one, `R` resumes or revives it, and `N`
  opens Add. The
  mod is focusable, so `pane.go` moves the keyboard onto it.
- **Width.** Below 1024 px, one column. From 1024 px, a 440 px column beside the list and the key
  hint bar. The list becomes the wide table with Recommends once it has 720 px of its own. The
  panel measures its own width, as gpui-kit has no container queries.

The panel draws agent text as plain text and never parses it. Its view model, `view.js`, is pure:
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

`mods/priory-asks/` is the Asks panel, in the side panel stack. It replaces Fleet's `fleet-asks`.
DESIGN.md's "Herdr adapter" section holds its visual rules; this section says how it works.

- **Files.** `view.js` turns `priory asks --all --json` into saved views, sections, rows and the
  ask detail, with no drawing and no IO. `main.js` draws them with gpui-kit and runs the verbs.
  The node tests in `test/asks.test.js` check `view.js` against JSON captured from a real
  `priory asks --all --json`.
- **Reading.** The panel reads `priory asks --all --json`, then waits with `priory wait` and reads
  again. With each read and each new selection, it reads the selected ask's task with
  `priory task ID --json`. When a call fails, it keeps the last list and shows "Reconnecting to priory" with the age
  of its last update, and the options stay disabled until it reconnects.
- **List.** The saved views are Open, Snoozed, Answered and All, each with its count. The facet
  chips Command, Lane, Band, Risk and Task narrow every view. Band lists the bands in the factory's
  order from the listing's `bands`, and puts an ask with no band, such as an outside ask, under
  `none`. Risk lists `irreversible`, `red`, `yellow`, `green`, then `none` for an ask whose `risk`
  is empty, and counts an ask under each risk it carries. A row shows its tile, task key, step,
  age, question, and what it recommends, its answer or why it was withdrawn.
- **Width.** Below 720 px the selected row opens under itself: the decide part, the section chips
  and Why. A section chip opens that section with the answer bar under it, since gpui-kit has no
  sticky positioning. From 720 px the list and the detail sit side by side. The decide part stays
  pinned at the top, and the sections scroll under it.
- **Answering.** An option answers after a 4 s Undo, with `priory answer --expected N`. An option
  that runs the ask's command arms on the first press, shows the command word by word under it,
  and runs on a second press within 4 s, with `--run`. "Write an answer…" sends free text the same
  way. Dismiss runs `priory dismiss` at once.
- **Refused answers.** When a verb fails, the panel reads the asks again and says why on the ask:
  the question changed to another revision, someone else answered, the ask was withdrawn, or the
  CLI's error. When the ask has left the view, the reason shows in a strip under the header
  instead. A refused answer goes back into the text field, so the person can send it again. While
  one answer, snooze or dismissal is in flight, the panel starts no other.
- **Snooze.** "Snooze 1 h" and the overflow menu's "Snooze until tomorrow 09:00" run
  `priory snooze ask ID`. They show only on an ask that belongs to a task. "Attach to task" asks the
  Priory bar to attach, as [Attaching](#how-it-reads-and-acts) describes.
- **Outside asks.** An ask from outside Priory lists and answers like the others, by its name. Its
  command is a string that never runs, so its options answer after the Undo.
- **Keys.** `J` and `K` move, `Enter` opens a section below 720 px, `1` to `9` press an option
  (twice for a command), `W` writes an answer, `S` snoozes for 1 h, `[` and `]` move between
  sections and `Esc` closes. Single keys never fire while the text field has the keyboard.
- **Agent text.** The question, options, text, trade-off and what waits draw as plain text.

The detail shows the sections Why, Brief, Plan, PR, Trace, History and Command. Brief shows the
request and the `groom` step's result, Plan the `plan` step's result or else the task's
`progress`, PR the pull request's title, branches, state and checks, and Trace each run with its
span, outcome and tokens. A section the task lacks is left out with its chip, as DESIGN.md says.
History lists every revision and refused answer of the ask, and its answer, newest first. A refused
answer shows on a warn tile. Briefs, plans and results draw as plain text, like the agent's text.

## The Queue panel

`mods/priory-queue/` is the Queue panel, in the side panel stack under Asks. It replaces Fleet's
`fleet-queue`. DESIGN.md's "Herdr adapter" section holds its visual rules; this section says how it
works.

- **Files.** `view.js` turns `priory queue DIR --json` into saved views, sections, rows, Up next,
  reorder places and Move targets, with no drawing and no IO. `main.js` draws them with gpui-kit and
  runs the verbs. The node tests in `test/queue.test.js` check `view.js` against JSON captured from a
  real `priory queue DIR --json`.
- **Reading.** The panel reads `priory queue DIR --json`, with the factory that settings name, and
  `priory desk --json` for whether starts are paused and what the queue is short of. Then it waits
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
  Cancel…. Cancel… asks once more, then runs `priory cancel`. The header's menu runs `priory pause`
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

## The Add action

The Desk and Queue panels share one Add form, `lib/add-form.js`, over the view model `lib/add.js`.
It holds one field, then chips for workflow, band and lane. The band chips are the factory's bands
from `priory queue --json`. The workflow and lane chips are those the listed tasks carry, so the
Queue panel, which reads no `priory tasks`, offers no workflow chip. An unpicked chip leaves the
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
- **Reading.** The panel reads the report written last, then waits with `priory wait` on the
  state's revision and reads again. When no task has written a report yet, it says so and waits for
  one. When a call fails, it keeps the last report and shows "Reconnecting to priory" with the age
  of its last update, and the picks stay disabled until it reconnects.
- **Sections.** The greeting, then the report's sections in the report's order. The panel knows no
  section by name. A section shows its `count`, or its number of rows when the report gives no
  count. A row shows its state tile, or a page icon when it has no `state`, then its key, text and
  meta, and its owner or time on the right. A time today shows its clock, an earlier one its date.
- **Toolbar.** The report's date, "n of m agreed" with a bar, and Next pick, which selects the next
  pick to agree and scrolls to it.
- **Views.** All sections, and Picks, which keeps only the rows with a pick to agree. From 1024 px
  the views and the sections sit in a 200 px list on the left; below, they are chips under the
  toolbar. A section in that list scrolls to it.
- **Picking.** An open pick is a segmented control of its options, with a `check` on the
  recommended one and "Recommends Take" beside it. A press waits out a 4 s Undo, since the first
  choice stands, then runs `priory report pick PICK OPTION --expected REVISION` with the report's
  revision the person read. While one pick is in flight, the panel starts no other. When the report
  stops offering the pick during the Undo, because another task wrote a report or someone picked
  first, the panel cancels the pick and says so.
- **Refused picks.** When `report pick` fails, the panel reads the report again and says why on the
  row: someone already picked, the report changed to another revision, the report no longer holds
  the pick, or the CLI's error. When the row has left the view, the reason shows in a strip under
  the toolbar instead. A pick that Priory took stays sent even when the read after it fails. A read
  older than the one shown is dropped, so a slow read never rolls the report back.
- **Agreed picks.** A chosen pick folds to one line: its key, the choice and, once the workflow
  applied it, its `applied` text, else "Waiting for the workflow to apply it."
- **Threads.** A row's threads sit under it. An open thread shows each comment's author, time and
  text, with Reply and Resolve; a resolved one folds to one line with Reopen. The selected row, when
  it has an id, ends with a comment field; a pick card offers Comment to select it. Comment runs
  `priory report comment ROW --expected REVISION TEXT` with the report's revision the person read,
  and Reply runs `priory report reply`. A refused comment says why under the row and keeps the text
  in the field. Rows that share an id share their threads.
- **Keys.** `J` and `K` move between rows, `N` goes to the next pick, `1` to `9` press an option of
  the selected pick, and `Esc` undoes a pick still waiting out its Undo.
- **Agent text.** Every row's text, meta, owner and the greeting draw as plain text.

The board's older and newer arrows, search and Change are left out. Priory keeps only the latest
report of each task, so there is no older report to open. A pick's first choice stands, so Change
has no verb to run. Comments on rows stay hidden until Priory stores them.

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
- An answered ask adds `answer`, `answered_by` and `answered_at`; a withdrawn one adds why.
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

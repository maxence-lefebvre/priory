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
- `adapters/herdr/mods/priory-asks/`: the Asks panel. See [The Asks panel](#the-asks-panel).
- `adapters/herdr/mods/priory-desk/`, `priory-queue/` and `priory-bonjour/`: one Herdr mod per
  panel, each a `mod.json`, a `main.js` and its copy of `priory.js`. They are still to come.
- `adapters/herdr/layouts/priory.json`: Herdr's default layout with the Priory bar in the top bar,
  and Asks and Queue in the side panel stack, 420 px wide by default, between 280 and 900. Desk and
  Bonjour open as tabs in the center, from the Priory bar.
- `adapters/herdr/install`: links the mods and the layout into Herdr's folder. See
  [Installing](#installing).
- `adapters/herdr/test/`: `node --test` tests for the module and the install script, run against a
  fake `priory`. CI runs them with `node --test adapters/herdr/test/*.test.js`.
- `adapters/herdr/lib/tokens.js`: Priory's theme colors and font roles from
  [`design/tokens/themes.css`](../../design/tokens/themes.css), every theme, each `light-dark()`
  pair kept as a light and a dark value. `palette(theme, mode)` picks one per mode. A test fails
  when the file and `themes.css` differ.
- `adapters/herdr/deliver`: the factory's `[asks] deliver` command; see
  [Delivering answers](#delivering-answers).

Herdr loads a mod's imports only from the mod's own folder, and it resolves symbolic links before it
checks. So each mod keeps a copy of `lib/priory.js` as its own `priory.js`, and a mod that draws
with Priory's colors keeps a copy of `lib/tokens.js`. A test fails when a copy differs from the
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

The layout names the four panel mods. Until they are installed, their blocks stay empty.

Herdr writes `gpui-kit.d.ts` and `jsconfig.json` into each mod folder it loads. Through the links,
they land in your checkout, and Git ignores them.

## How it reads and acts

A panel reads the factory through Priory and acts through Priory's own verbs. It never opens the
state database and never writes a file in the factory.

- **Which Priory.** A mod runs the program that the `priory` object of Herdr's `settings.json`
  names, on the Herdr machine it names: `{"priory": {"binary": "~/.local/bin/priory-oss",
  "machine": "xps13", "factory": "/home/me/factory"}}`. Every field is optional. `binary` defaults
  to `~/.local/bin/priory-oss`, and a leading `~` means the home folder on Priory's machine. A name
  without a slash is looked up on the `PATH`. `machine` defaults to the machine Herdr runs on. `factory` is the folder
  that `queue` and `schedules` read for up next. The program must find its state file, so set
  `PRIORY_DB` in the environment, or name a wrapper script that sets it. `theme` names the
  Priory theme a panel draws with: `indigo` (the default), `ink`, `parchment` or `abbey`.
- **Reading.** A mod runs the CLI with Herdr's `exec` and parses its JSON: `priory desk --json`,
  `asks --all --json`, `tasks --json`, `queue --json`, `schedules --json` and
  `report show --json`. Each answer carries the state's `revision`. Then the mod runs
  `priory wait --since REVISION`, which returns as soon as the state changes, or after 25 s, and
  reads again. It keeps the last good answer, with its age, when a call fails. Once `serve` exists,
  a mod may subscribe to its event stream instead.
- **Acting.** Every button runs a CLI verb: `answer --expected N [--run] -- ASK TEXT`,
  `dismiss --expected N ASK`, `snooze`, `say`, `report pick PICK OPTION --expected N`, `seen`,
  `hold`, `release` and `move`. An answer always carries the revision the person read, so a
  changed question refuses it (R6.3).
- **Safety.** `priory.js` passes every value as its own argument, never through a shell, and never
  reads text output except `attach --print`. A mod draws agent text as plain text.
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

## Delivering answers

`adapters/herdr/deliver` brings an outside ask's answer to the Claude session that asked, as
[asks.md](../daemon/asks.md) describes. A factory names it with
`[asks] deliver = "adapters/herdr/deliver"`, from a factory folder that holds the adapter. It needs
Python 3 and `hc`, and no other package.

It reads the ask's `session` as an hc terminal, optionally followed by a space and the Claude
session id. `priory ask` run in an hc terminal stores both. Then:

1. `hc get TERMINAL`: when the terminal is live and runs `claude`, `hc send` types
   `A person answered ask <ID> (revision <N>): <answer>` into it, or
   `A person dismissed ask <ID> without an answer. Continue your task.`
2. Otherwise it finds the Claude session's transcript under `~/.claude/projects/` and reads the
   folder it ran in. It starts `claude --resume <session> <text>` there with
   `hc run --bg --keep --no-parent`, in a new tab named `ask-<ID>`, in the old terminal's
   `herdr.space` when hc still knows it.
3. With no Claude session id or no transcript, it exits 1, and the delivery is tried again, then
   fails on the desk.

A kept terminal whose `claude` exited runs a shell, so the script never types into it: the shell
would run the answer as a command. The check and the send are two `hc` calls, so a `claude` that
exits between them can still receive the text in its shell.

## The Asks panel

`mods/priory-asks/` is the Asks panel, in the side panel stack. It replaces Fleet's `fleet-asks`.
DESIGN.md's "Herdr adapter" section holds its visual rules; this section says how it works.

- **Files.** `view.js` turns `priory asks --all --json` into saved views, sections, rows and the
  ask detail, with no drawing and no IO. `main.js` draws them with gpui-kit and runs the verbs.
  The node tests in `test/asks.test.js` check `view.js` against JSON captured from a real
  `priory asks --all --json`.
- **Reading.** The panel reads `priory asks --all --json`, then waits with `priory wait` and reads
  again. When a call fails, it keeps the last list and shows "Reconnecting to priory" with the age
  of its last update, and the options stay disabled until it reconnects.
- **List.** The saved views are Open, Snoozed, Answered and All, each with its count. The facet
  chips Command, Lane and Task narrow every view. A row shows its tile, task key, step, age,
  question, and what it recommends, its answer or why it was withdrawn.
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

The detail shows the sections Why, History and Command. `priory` has no command that prints a
task's brief, plan, pull request or runs, so the Brief, Plan, PR and Trace sections are left out
with their chips, as DESIGN.md does for any section a task lacks. History shows the ask's current
revision and its answer, since Priory keeps no earlier revisions or refused answers.

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
- *Since you looked*: the events since the person's last visit, each with task, what happened and
  when. This needs a stored last visit per person.

### Asks

- Every ask with its `state` (`open`, `answered`, `withdrawn`), `revision`, `question`,
  `options`, `recommendation`, `tradeoff`, and for a command ask the command line, its target and
  the revision it is bound to (R6.8).
- An answered ask adds `answer`, `answered_by` and `answered_at`; a withdrawn one adds why.
- The detail adds the agent's text and the run that asked, *what waits* and *what the agent does
  next*, the brief and plan paths, the pull request summary, the task's runs for the trace, and
  the ask's revisions and refused answers for its history.

### Queue

- The waiting tasks in start order (R4.4), each with `band`, its position in the band, `state`,
  and why it has not started: no room, a band above it, or its hold and the hold's condition.
- The running runs, with run id, step and start time.
- The tasks ended in the last day.
- *Up next*: the next three runs, queued or scheduled, each with when it should start and why.
  Schedules need their next firing time.

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

The panels need these from Priory's core. Each is a separate slice.

1. `--json` on `priory desk`, `asks`, `tasks` and `report`, with the fields above.
2. `serve`: the same data over HTTP with an event stream (R8.5).
3. Snooze: the fields above, `priory snooze`, and the counts that leave snoozed items out.
4. A stored last visit per person, for Since you looked.
5. The next firing time of each schedule, for Up next.
6. Report sections and picks: done, see [reports.md](../daemon/reports.md). Comments on rows are
   still to come.
7. Ask fields for what waits and what the agent does next: `ask` stores them as `what_waits` and
   `next_action`, with a `title` and a Markdown `text`. The Asks panel shows them.
8. A task's brief, plan, pull request summary and runs from the CLI, for the Asks panel's Brief,
   Plan, PR and Trace sections. `/api/tasks/{id}` serves the runs but has no CLI twin, and nothing
   serves the brief, the plan or the pull request summary yet.
9. An ask's earlier revisions and refused answers, for its History section.

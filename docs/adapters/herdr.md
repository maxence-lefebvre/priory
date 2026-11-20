# The Herdr adapter

Priory's views render in a browser first (R8.4). The PRD also asks for a second view host that
renders the same views as an adapter (R8.6). The Herdr adapter is that host for
Herdr, a terminal workspace whose panes run JavaScript mods drawn natively
with gpui-kit. This page explains where the adapter lives, how it reads and changes the factory,
what data each panel needs, and how another front end would write its own adapter.
[DESIGN.md](../../DESIGN.md), under "Herdr adapter", holds the visual rules.

## Where it lives

The adapter will live in `adapters/herdr/` at the repository root:

- `adapters/herdr/mods/priory-desk/`, `priory-asks/`, `priory-queue/` and `priory-bonjour/`: one
  Herdr mod per panel, each a `mod.json` and a `main.js`.
- `adapters/herdr/layouts/priory.json`: a Herdr layout that places the panels. Desk and Bonjour
  open as tabs in the center stack. Asks and Queue share the side panel stack, 420 px wide by
  default, between 280 and 900.
- `adapters/herdr/tokens.js`: Priory's theme tokens, generated from
  [`design/tokens/`](../../design/tokens/), with each `light-dark()` pair resolved per mode.

- `adapters/herdr/deliver`: the factory's `[asks] deliver` command. It ships today; see
  [Delivering answers](#delivering-answers).

Installing the adapter links the mods into Herdr's mods folder (`~/.config/herdr-app/mods/`) and
the layout into its layouts folder. The adapter depends on no Priory crate, and no crate depends on
it.

## How it reads and acts

A panel reads the factory through Priory and acts through Priory's own verbs. It never opens the
state database and never writes a file in the factory.

- **Reading.** Until `serve` exists, a mod runs the CLI with Herdr's `exec` and parses its JSON:
  `priory desk --json`, `priory asks --json`, `priory tasks --json` and `priory report show --json`. It
  polls every few seconds and keeps the last good answer, with its age, when a call fails. Once
  `serve` exists, a mod subscribes to its event stream instead and stops polling.
- **Acting.** Every button runs a CLI verb: `priory answer ASK OPTION --revision N`,
  `priory resume`, `priory revive`, `priory cancel`, `priory hold`, `priory release`,
  `priory move`, `priory request` and `priory snooze`. An answer always carries the revision the
  person read, so a changed question refuses it (R6.3).
- **Themes.** A mod draws with Priory's tokens, not Herdr's theme, so a theme looks the same in
  Herdr and in the browser. The adapter picks the light or dark value from Herdr's appearance.

Today `priory desk`, `asks` and `tasks` print text for people. The JSON output and `serve` are
core work the adapter waits on; the list of gaps is at the end of this page.

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
would run the answer as a command.

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
   `next_action`, with a `title` and a Markdown `text`. The Asks panel does not show them yet.

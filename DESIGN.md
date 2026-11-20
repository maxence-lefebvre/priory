# DESIGN.md

The design system for Priory's web view (`docs/prd.md`, R8). Follow it for every panel, view and
component. The live reference is the
[Priory design system artifact](https://claude.ai/artifact/3YYj2s5hmKksarCCC3VgHb): tokens, the
main components and their states in every theme. The token files in
[`design/tokens/`](design/tokens/) are the source of truth for values. When the artifact, this
file and the token files disagree, the token files win for values and this file wins for rules.
Fix the others.

The M2 test is "one working day of asks answered from a phone only". Every decision below serves
that first: a person on a phone, often one-handed, reads an ask and answers it correctly.

The view has one structure, called Ledger, and four themes: Indigo (the default), Ink, Parchment
and Abbey, each in light and dark. A person picks a theme and a mode on the Appearance screen.

## Principles

1. **The desk comes first.** What waits on a person (asks, blocked tasks, uncertain prompts, room
   banners) is the default view and the first tab. Everything else is context for an answer.
2. **Ink for attention, the action color for actions, color for state.** The next ask is printed
   in solid ink. Buttons a person taps take the theme's action color. Every other color belongs
   to a state tile. A screen with nothing to do looks quiet.
3. **Dense, not cramped.** Rows show state, key, step and age in one glance. Density comes from
   tight type and hairlines, never from small touch targets.
4. **Phone first, desktop extends.** Design each panel at 360 px wide first. Wider screens show
   more panels side by side; they do not get different components.
5. **State is a tile: shape and word, then color.** Every state has an icon and its daemon name.
   Color only reinforces it.
6. **One structure, many themes.** A theme changes colors and the three font roles, nothing else.
7. **Never answer by accident.** No gesture answers an ask. Answers to command asks need a second,
   deliberate tap.
8. **Agent text is untrusted.** Render questions, options, summaries and terminal output as text
   or sanitized Markdown. Never as HTML.
9. **Offline is normal.** Show the last known state with its age when the connection drops.
   Never blank a panel.

## Structure and themes

Tokens are CSS custom properties in two files. Components style only through their names. Never
write a literal color, typeface, size or radius in a component rule.

- [`design/tokens/structure.css`](design/tokens/structure.css): spacing, radii, sizes, the type
  scale, elevation and motion. The same in every theme.
- [`design/tokens/themes.css`](design/tokens/themes.css): colors and font roles, one block per
  theme. Every theme defines every token.

A theme may set only:

- **Colors.** Every color token listed under [Theme tokens](#theme-tokens), each as one
  `light-dark()` pair so the theme works in both modes.
- **The three font roles.** `--font-display` for the ask count, titles and questions;
  `--font-ui` for everything a person reads or taps; `--font-mono` for machine text. A role may
  carry its tuning: `--display-stretch`, `--count-stretch`, `--count-weight` and
  `--count-tracking`.

A theme never changes layout, spacing, radii, sizes, the type scale, motion, icons, copy or which
components appear. A design idea that needs any of these is a change to the structure, for every
theme, in this file.

### Choosing a theme and a mode

The view sets two attributes on `<html>`:

```html
<html data-theme="indigo" data-mode="system">
```

- `data-theme` is `indigo`, `ink`, `parchment` or `abbey`. On `<html>`, a missing or unknown
  value gives Indigo.
- `data-mode` is `system`, `light` or `dark`. `system` follows `prefers-color-scheme` through
  `color-scheme: light dark`, with no script. `light` and `dark` pin the mode.
- The view stores both choices per device in local storage and applies them before first paint,
  so a theme never flashes.
- The same two attributes on any element scope a theme and mode to it. The Appearance previews
  use this to show every theme on one screen.

`light-dark()` needs Safari 17.5, Chrome 123 or Firefox 120.

## Structure tokens

[`design/tokens/structure.css`](design/tokens/structure.css) holds the values. The rules:

- **Spacing:** a 4 px grid, `--space-1` (4 px) to `--space-8` (48 px). Lay out siblings with
  `gap`, not margins. The side gutter is `--gutter`, 16 px or the safe-area inset.
- **Radii:** `--radius-sm` (5 px) for tags and key hints, `--radius-tile` (8 px) for state tiles,
  `--radius-md` (12 px) for inputs, banners, the segmented control, the workflow grid and the
  terminal, `--radius-card` (14 px) for the next-ask card, option buttons and theme cards,
  `--radius-full` for the Answer capsule, live dots and badges. Rows have no radius.
- **Elevation:** hairlines (`--line`) separate. Only the Answer capsule (`--shadow-float`) and
  sheets (`--shadow-sheet`) cast a shadow.
- **Motion:** `--dur-1` to `--dur-3` with `--ease-out` and `--ease-in`, all zero under
  `prefers-reduced-motion`.

### Type scale

| Token             | Size / line | Role    | Weight   | Use                                            |
| ----------------- | ----------- | ------- | -------- | ---------------------------------------------- |
| `--type-count`    | 104 / 0.8   | display | theme    | the desk's ask count, nothing else             |
| `--type-count-wide` | 88 / 0.8  | display | theme    | the ask count in the wide desk column          |
| `--type-screen`   | 34 / 40     | display | 700      | screen titles, such as Appearance              |
| `--type-task`     | 32 / 38     | display | 700      | the task title on wide screens                 |
| `--type-question` | 25 / 31     | display | 700      | the question in answer mode                    |
| `--type-hero`     | 24 / 27     | display | 700      | "asks need you" beside the count               |
| `--type-next`     | 18 / 24     | display | 600      | the question in the next-ask card              |
| `--type-option`   | 17 / 22     | UI      | 600      | option labels, the Answer capsule              |
| `--type-body`     | 15 / 20     | UI      | 400, 500 | row text, default text                         |
| `--type-small`    | 14 / 20     | UI      | 400, 500 | reasons, trade-off, banner text                |
| `--type-caption`  | 13 / 18     | UI      | 400, 600 | row metadata, section heads, ages              |
| `--type-mono`     | 12.5 / 19   | mono    | 400, 500 | task keys, ids, shas, commands, wide terminal  |
| `--type-mono-compact` | 12 / 16 | mono    | 400      | the terminal on compact screens                |
| `--type-tab`      | 11 / 14     | UI      | 600      | tab bar labels                                 |

- The display role takes `--display-stretch`; the count also takes `--count-stretch`,
  `--count-weight` and `--count-tracking`. A grotesk theme uses them for its wide cut; a serif
  theme sets them to plain values.
- Set inputs at 16 px at every width. Smaller inputs make iOS Safari zoom the page.
- Use `font-variant-numeric: tabular-nums` for every column of numbers, durations and times.
- Keep reading text under 72 characters per line.

## Themes

| Theme     | Character                                           | Display / UI / mono                         | Action | Waits on a person | Emphasis   |
| --------- | --------------------------------------------------- | ------------------------------------------- | ------ | ----------------- | ---------- |
| Indigo    | The default. Ink emphasis with an indigo action.    | Mona Sans / Mona Sans / JetBrains Mono      | indigo | indigo            | ink        |
| Ink       | Monochrome. What needs you is printed in solid ink. | Mona Sans / Mona Sans / JetBrains Mono      | ink    | ink               | ink        |
| Parchment | Parchment and iron-gall ink, rubric red and lapis.  | EB Garamond / IBM Plex Sans / IBM Plex Mono | lapis  | rubric red        | ink        |
| Abbey     | Vellum, lapis and gold, rubric red.                 | Alegreya / Alegreya Sans / IBM Plex Mono    | lapis  | rubric red        | deep lapis |

- **Rubric red marks only what waits on a person.** In Parchment and Abbey, red appears on ask
  tiles, the ask count and the Desk badge, and nowhere else. Actions, including the recommended
  option and the Answer capsule, take the lapis action color. Failure takes dark oxblood, and
  danger text a pale rose in dark mode, both apart from rubric red in lightness.
- **The recommended option takes `--recommended`, which is the theme's action color in every
  theme.** It is never `--wait`.
- **Fonts:** all seven families are under the SIL Open Font License 1.1. Ship their `woff2` files
  with the daemon and serve them from the view. Never load fonts from a CDN: the view must work on
  a factory with no internet. Load a theme's families only when that theme is active or shown
  in an Appearance preview.

### Theme tokens

| Token                                       | Meaning                                                                |
| ------------------------------------------- | ---------------------------------------------------------------------- |
| `--bg`                                      | page ground                                                            |
| `--surface`                                 | cards, the detail pane, option buttons, inputs                         |
| `--raised`                                  | pressed rows, the segmented control track, skeletons                   |
| `--line`                                    | hairlines between rows and around panels; never the only edge of a control |
| `--control-line`                            | edges of inputs, option buttons and secondary buttons, at least 3:1    |
| `--bar`                                     | the tab bar and the fade behind the Answer capsule                     |
| `--text`, `--text-2`, `--text-3`            | primary, secondary and metadata text                                   |
| `--emph`, `--on-emph`, `--on-emph-2`        | solid ink: the next-ask card, the current workflow step, the active tab |
| `--action`, `--on-action`                   | primary buttons, the Answer capsule, the recommended option            |
| `--action-soft`, `--action-text`            | selected rows and sidebar items, links, ghost buttons, the focus ring  |
| `--wait`, `--on-wait`                       | the ask tile: what waits on a person                                   |
| `--count`                                   | the desk's ask count                                                   |
| `--badge`, `--on-badge`                     | the count of open asks on the Desk tab                                 |
| `--tile-live`, `--tile-warn`, `--tile-danger`, `--tile-ok`, `--tile-hold`, `--tile-muted` | state tile fills |
| `--on-tile`, `--on-tile-live`               | the icon on a state tile; `--on-tile-live` on the live tile            |
| `--warn-text`, `--danger-text`, `--ok-text` | reasons and state words in text                                        |
| `--term-bg`, `--term-fg`, `--term-dim`      | the terminal, dark in every theme and mode                             |
| `--recommended`, `--on-recommended`         | derived: equal to `--action` and `--on-action`                         |

Contrast, checked for every theme in both modes: text tokens, state text tokens and
`--action-text` reach 4.5:1 on `--bg` and `--surface`; `--on-action`, `--on-emph`, `--on-emph-2`,
`--on-badge` and the terminal text reach 4.5:1 on their fill, and so does `--on-tile` on
`--tile-danger`, which is also the danger button; tile icons, `--on-wait` and `--control-line`
reach 3:1; the count reaches 3:1 on `--bg`. Re-check every pair with a contrast
tool after any change to a theme.

### The illuminated initial is dropped

An illuminated initial, the count in a gilded square, was tried for Parchment and Abbey. It is
dropped: it would be the one place a theme changes layout, breaking principle 6, and its 84 px
square shrinks the count. Those themes keep their character through their serif display role
and the rubric red `--count`.

## Layout

### Breakpoints

| Name    | Width         | Layout                                                                |
| ------- | ------------- | --------------------------------------------------------------------- |
| compact | below 600 px  | One panel at a time. Bottom tab bar. Detail pages push over the list. |
| medium  | 600 to 1023   | List and detail side by side, list 320 to 400 px wide.                |
| wide    | 1024 and up   | Sidebar, desk column and detail, or the view file's tree as written.  |

- A view file (R8.2) describes the wide layout. On compact screens, each leaf panel becomes a tab
  in the tree's order. The bottom bar shows the first four and a "More" tab for the rest.
- The default view puts the desk first, then queue, tasks and runs. Inbox, workflows, reports and
  terminals sit under "More".
- Panels lay out by their own width with container queries, not by the viewport.
- Every task, run and ask has its own URL, such as `/asks/42`. A link from a notification opens
  that ask directly, on any screen size.

### Frame on a phone

- **Top bar**, `--top-bar` (52 px), with a hairline under it: the factory name in the display
  role, the connection state as a dot and a word ("live", "reconnecting"), and a search button.
  It stays sticky below the safe-area inset.
- **Tab bar**, `--tab-bar` (60 px plus the bottom safe area), on `--bar`: Desk, Queue, Tasks,
  Runs, More, each an icon and a label. The active tab is a solid `--emph` pill. The Desk tab
  carries the open-ask count in `--badge`.
- The page never scrolls sideways. Only tables, diagrams and the terminal scroll inside their own
  box. Use `height: 100dvh` so mobile browser bars do not hide the tab bar.

### Frame on a wide screen

- **Top bar**: the factory name and connection state, a command bar in the middle ("Search or run
  a command", `Ctrl K`), and the runner name on the right.
- **Sidebar**, `--sidebar` (200 px): Desk, Queue, Tasks and Runs with counts, then a "More"
  group. The active item takes `--action-soft` and `--action-text`.
- **Desk column**, `--desk-column` (400 px): the desk, as on the phone, without the capsule.
- **Detail**: the selected task or run on `--surface`.
- **Key hint bar** at the bottom, 40 px: `J` `K` move, `Enter` open, `1` to `9` answer, `W` write,
  `?` all shortcuts.

## Components

Every component below lists its states. Build no state that is not listed without adding it here
first. Icons come from [Lucide](https://lucide.dev) (ISC license), inlined as SVG.

### State tile

A state tile names a task or run state: a `--tile` (30 px) square with `--radius-tile` corners,
filled with the role's token, holding a 17 px icon in `--on-tile`, or `--on-tile-live` on the
live role (running, ready). Next to it, the row shows the daemon's exact word in lowercase, as
`priory tasks` and `priory runs` print it. In wide tables the tile becomes a 15 px icon in the
role's text color before the word.

Each role's text color: live `--action-text`, warn `--warn-text`, danger `--danger-text`, ok
`--ok-text`; wait, hold and muted `--text-2`.

An ask is not a state, but it takes a tile too: `--wait` with the `message-circle-question` icon
in `--on-wait`. A running tile holds a live dot in `--on-tile-live` instead of an icon; the dot
pulses once per 2 s and stays solid under reduced motion.

Task states (`crates/priory-core/src/state.rs`, `tasks.state`):

| State     | Role   | Icon            | Means for the person                        |
| --------- | ------ | --------------- | ------------------------------------------- |
| `queued`  | muted  | `circle-dashed` | Waits in the queue for its turn.            |
| `held`    | hold   | `pause`         | Held by a person or a condition; shows why. |
| `ready`   | live   | `circle-dot`    | Next to start when a runner has room.       |
| `running` | live   | live dot        | An agent is working on it now.              |
| `asleep`  | muted  | `moon`          | Waits on an event; holds no process.        |
| `blocked` | warn   | `hand`          | Needs a person. Always on the desk.         |
| `done`    | ok     | `check`         | Finished.                                   |
| `failed`  | danger | `circle-x`      | Ended in failure. Can be revived.           |
| `dropped` | muted  | `minus`         | Stopped on purpose. Can be revived.         |

An asleep task whose `retired` column is set shows `asleep` plus a muted tag `retired` with the
`archive` icon: it has no worktree and resurrects on its event.

Run states (`runs.ended_as`; a run with no `ended` is live):

| State       | Role   | Icon          | Means                                                            |
| ----------- | ------ | ------------- | ---------------------------------------------------------------- |
| `running`   | live   | live dot      | Live. Shows `idle` in `--text-3` when the agent waits for input. |
| `done`      | ok     | `check`       | The task reached done in this run.                               |
| `moved`     | muted  | `arrow-right` | The step ended; the task went to its next step.                  |
| `asleep`    | muted  | `moon`        | Ended by `wait` or `ask`.                                        |
| `blocked`   | warn   | `hand`        | Ended by a verdict that needs a person.                          |
| `lost`      | warn   | `unplug`      | Lost with its tmux server; resumes once.                         |
| `restarted` | muted  | `rotate-ccw`  | Restarted by a handler.                                          |
| `cancelled` | muted  | `x`           | Cancelled by a person.                                           |
| `dropped`   | muted  | `minus`       | The task was stopped.                                            |
| `exited`    | danger | `log-out`     | The agent process exited unexpectedly.                           |
| `failed`    | danger | `circle-x`    | The run failed.                                                  |

If the daemon reports a state not in these tables, show it as a muted tile with its raw word and
the `circle-help` icon. Add it here in the same change that adds it to the daemon.

Each role's tile fill in each theme, light / dark, from
[`design/tokens/themes.css`](design/tokens/themes.css):

| Role   | Token           | States                                               | Indigo                | Ink                   | Parchment             | Abbey                 |
| ------ | --------------- | ---------------------------------------------------- | --------------------- | --------------------- | --------------------- | --------------------- |
| wait   | `--wait`        | asks                                                 | `#5856D6` / `#5E5CE6` | `#0A0A0A` / `#EDEDED` | `#A4161A` / `#B9302A` | `#A31F1A` / `#B42A22` |
| live   | `--tile-live`   | running, ready                                       | `#5856D6` / `#5E5CE6` | `#0A0A0A` / `#EDEDED` | `#1F3F8F` / `#3F63C4` | `#24489C` / `#4466C8` |
| warn   | `--tile-warn`   | blocked, lost                                        | `#C96800` / `#C96800` | `#B45309` / `#B45309` | `#9A6400` / `#A36A00` | `#A86B00` / `#A86B00` |
| danger | `--tile-danger` | failed, exited                                       | `#D70015` / `#D70015` | `#CB1D26` / `#D92D35` | `#4A1A12` / `#6E2216` | `#5A1C17` / `#6E241E` |
| ok     | `--tile-ok`     | done                                                 | `#248A3D` / `#2E8B47` | `#18793A` / `#24873F` | `#3F6B2A` / `#4A7A33` | `#2F6B4F` / `#2F7A57` |
| hold   | `--tile-hold`   | held                                                 | `#8944AB` / `#9A55BF` | `#7340B8` / `#8550C8` | `#4B3F8F` / `#5C4FA8` | `#5B4A9C` / `#6A58B5` |
| muted  | `--tile-muted`  | queued, asleep, dropped, moved, restarted, cancelled | `#8E8E93` / `#636366` | `#8A8A8A` / `#555555` | `#8A7A5C` / `#6A5D48` | `#8C8170` / `#4C5468` |

In Indigo and Ink the ask tile and the live tile share a fill; the icon tells them apart.

### Desk

The desk lists, top to bottom:

1. **Count**: the number of open asks in `--type-count` and `--count`, beside "asks need you" in
   `--type-hero` and a line in `--text-3`: "Oldest waits 42 min. 2 stopped."
2. **Room banners** (R4.3): a `--warn-text` edge on `--surface`, the `hard-drive` or
   `memory-stick` icon, and the daemon's sentence: what is short, by how much, since when, and
   the waiting task.
3. **Next-ask card**: the oldest open ask, a button in solid `--emph` with `--radius-card`. Its
   context line (ask icon, task key, step, a "Command" tag for a command ask, age) sits above the
   question in `--type-next`, clamped at 3 lines, and "Oldest ask. Tap to answer." Tapping it
   opens answer mode on that ask.
4. **Then**: the other open asks, oldest first, as rows.
5. **Stopped**: blocked tasks, uncertain prompts (R5.4) and failed tasks, each with its reason in
   the role's text color.
6. **Asleep**: one row, "9 tasks asleep on waits", that opens the tasks panel filtered to asleep.
7. Reports waiting for review, when there are any.

On a phone, the **Answer capsule** floats above the tab bar: a full-width `--action` button,
`--capsule` (54 px) tall, `--capsule-gap` above the tab bar, with `--radius-full` and
`--shadow-float`, labelled "Answer 3 asks". It opens answer mode on the oldest ask. A fade from
transparent to `--bar` sits behind it.

- **The desk list ends with bottom padding of `--desk-end`**: the capsule's height plus its gap
  above and below, so the capsule never covers the last row when the list is scrolled to the end.
- The capsule hides when no ask is open. An empty desk shows: "No open asks. Agents ask here when
  they need you."

Rows on the desk are at least `--row-min` (64 px) tall with a hairline above each: a state tile
on the left, then line 1 (task key in mono, step, tags, and the age on the right in `--text-3`),
line 2 (the question or reason, clamped at 2 lines), and a chevron. The whole row is the touch
target.

### Answer mode

On compact screens, answering opens answer mode: one ask fills the screen.

1. **Header**: "Desk" with a back chevron on the left, the position "1/3" on the right in the
   display role, the total in `--text-3`.
2. **Context line**: the ask tile, task key, step, lane and age. It wraps. Tapping it opens the
   task.
3. **Question** in `--type-question`. A long question scrolls; the options never move.
4. **Options**: one full-width button per option, in the agent's order. Never reorder them. Each
   is at least `--option-min` (58 px) tall with `--radius-card` and a left-aligned label in
   `--type-option`. Ordinary options sit on `--surface` with a 1 px `--control-line` edge. The
   recommended option fills with `--recommended` and shows "Recommended" under its label. The
   options sit in the bottom half, inside the thumb's reach.
5. **Trade-off** under the options: "Trade-off." then the agent's sentence, in `--type-small`.
6. **Write an answer**: a ghost button that expands into a 16 px text field with a "Send"
   button. Free text is always possible.
7. **Bar**: "Skip" on the left, "Revision 2" in the middle in `--text-3`, "Next" on the right.

- After an answer, the next ask slides in and the position updates.
- "Skip", "Next" and a horizontal swipe move without answering. Nothing answers except a tap on
  an option or "Send".
- Leaving answer mode returns to the desk at the same scroll position.
- On wide screens the same ask card shows in the detail column; `1` to `9` pick an option.

Ask states:

| State     | What the person sees                                                                      |
| --------- | ----------------------------------------------------------------------------------------- |
| open      | As above.                                                                                 |
| sending   | The chosen option shows a check; the others dim. A bar at the bottom says `Sending "Merge"` with an "Undo" button for 4 s. The view sends after the 4 s. |
| answered  | The card collapses to the question, the answer, who answered and when. No buttons.        |
| refused   | A `--warn-text` banner: "The question changed. Read revision 3 and answer again." The card reloads the new revision; the old answer stays in the text field. |
| taken     | "Already answered 2 min ago: Merge." Shown when another answer came first.                |
| withdrawn | Muted: "Withdrawn: the task was revived." No buttons.                                     |
| offline   | Options stay visible but disabled, with "Reconnect to answer".                            |

A **command ask** (R6.8) also shows the command and its target in a mono block on `--raised`
above the options. Its answer runs the command, so:

- It has no undo bar. The first tap turns the option into "Tap again to run" for 4 s; the second
  tap sends.
- After sending it shows the outcome: `running`, `done`, `failed` with the error, or `uncertain`
  in `--warn-text`: "Priory stopped while this ran. Check the target before you act on it."

### Appearance screen

Settings, then Appearance. It holds:

1. A screen title, "Appearance", in `--type-screen`.
2. **Mode**: a segmented control, System, Light and Dark, `--control` (44 px) tall on
   `--raised`. The selected segment sits on `--surface`.
3. **Theme**: one card per theme, in the order Indigo, Ink, Parchment, Abbey. Each card shows the
   desk in that theme, light and dark side by side at one fifth scale, the theme's name in the
   display role, one sentence about it, and a radio. The selected card has a 2 px `--action` edge
   and a filled radio.
4. A note in `--text-3`: "A theme changes color and type only. Every screen keeps the same
   layout."

- Choosing applies at once, with no save button, and is stored on this device.
- The cards are a `radiogroup`; each card is a `radio` named by its theme.
- Each preview renders the real desk component with the theme's attributes, not a picture, so it
  can never drift from the theme.

### Task row and run row

- **Phone**: rows as on the desk. Line 2 is workflow and step, lane, band and age; a `blocked`,
  `held` or `failed` row shows its reason there in the role's text color.
- **Wide**: one line, `--row-wide` (38 px), with columns for state, key, workflow and step, lane,
  band and age (tasks) or run id, step, agent, started, duration and state (runs). Numbers are
  right aligned in `tabular-nums`.
- States: default, pressed (`--raised`), selected (bold text and a 3 px `--action` bar on the
  leading edge), new (fading `--action-soft`), focused (focus ring).
- A live run shows its duration counting up once per minute, not per second. Tapping a run opens
  its terminal.
- Actions live on the task page, not in the row: Resume, Cancel, Revive, Hold, Release, Move.

### Task detail and workflow grid

- Context line: task key in mono, the task's state tile and word, workflow, lane, band.
- Title in `--type-task`.
- **Workflow grid**: one cell per step in a row of equal columns with `--radius-md` and hairlines
  between cells. Each cell holds the step name and its decider with an icon: `cog` for model and
  code steps, `user` for human steps. Done steps show a check in `--ok-text`. The current step is
  solid `--emph` with a live dot. Future steps are in `--text-3`. An irreversible step has a
  1.5 px `--danger-text` edge and the `lock` icon.
- Then the runs table and the terminal of the selected run.
- The full workflow diagram (steps as nodes, outcome edges) keeps the same encodings, top to
  bottom on compact and left to right on wide, with pinch, drag and a "Fit" button.

### Panel chrome

- Header, 44 px, sticky: title in the display role, count in `--text-3`, then up to two icon
  buttons (filter, menu). Any further action goes in the menu.
- Body scrolls on its own with `overscroll-behavior: contain`.
- When the event stream drops, a `--warn-text` strip under the header says "Reconnecting. Last
  update 2 min ago." The panel keeps its rows.

### Terminal pane

- Always the `--term-*` tokens, dark in every theme and mode, with `--radius-md`. Mono at 12 / 16
  on compact, 12.5 / 19 on wide.
- tmux sets the width, so lines never wrap. The pane scrolls sideways inside its own box. A "Fit"
  toggle scales the text down to fit the width, never below 9 px.
- Read-only by default. "Type" opens a one-line input at the bottom that sends a line on Enter.
  The header shows `read-only` or `typing`.
- Header: the `terminal` icon, session name in mono, run id, "Copy screen", "Fit", "Attach".
- When the run ends, the pane keeps its last screen and shows the run's state tile.

### Reports and tables

- Numbers right-aligned in `tabular-nums`. The first column stays sticky when the table scrolls
  sideways on a phone.
- Headers in `--type-caption`, `--text-3`, sentence case.

### Buttons and inputs

| Kind      | Fill            | Text            | Edge             | Use                                  |
| --------- | --------------- | --------------- | ---------------- | ------------------------------------ |
| primary   | `--action`      | `--on-action`   | none             | One per region: the expected action. |
| secondary | `--surface`     | `--text`        | `--control-line` | Every other action.                  |
| ghost     | none            | `--action-text` | none             | Links and low-weight actions.        |
| danger    | `--tile-danger` | `--on-tile`     | none             | Cancel task, drop. Always confirms.  |

- Height `--control` (44 px) on a coarse pointer, `--control-fine` (32 px) on a fine one.
- States: pressed, focused, disabled (`--text-3` text, no fill change, `aria-disabled`), busy (an
  inline 12 px spinner replaces the icon; the label stays).
- Inputs: 16 px text, `--surface` fill, `--control-line` edge, `--radius-md`. Errors show under
  the field in `--danger-text` with the `circle-x` icon.

### Empty, loading and error states

- **Empty**: one sentence that says what appears here and what makes it appear, plus the action
  or CLI command. "No open asks. Agents ask here when they need you."
- **Loading**: show nothing for the first 300 ms, then skeleton rows on `--raised` at the real
  row height. Never a full-page spinner. A list that has data keeps it while it refreshes.
- **Error**: where it failed, why, and how to fix it, with a "Retry" button. "Could not load runs:
  the daemon refused the connection. Check that the daemon runs on its runner, then retry." Keep
  any stale data visible under the error.

### Motion

Motion shows what changed after a person acts. Nothing moves on its own except the dot of a live
run.

- A new row gets an `--action-soft` background that fades out over 1.2 s. With reduced motion it
  keeps a small dot until the person scrolls past it.
- Sheets slide up with `--dur-3`. With reduced motion they appear in place.
- Never animate the height of a list that updates live. Insert rows without moving the row under
  the person's thumb: if they scrolled, keep their position and show "3 new" at the top instead.

## Touch and gestures

- Touch targets are at least 44 by 44 px on coarse pointers and 24 by 24 px everywhere, with
  8 px between neighbours.
- Put the primary action of a screen in its bottom third on compact screens.
- Every gesture has a visible button that does the same thing.
- Allowed gestures: horizontal swipe between asks in answer mode and between tabs; pinch and drag
  on diagrams; the browser's own back swipe. Nothing else.
- No pull to refresh: panels update live. Set `overscroll-behavior: contain` on scrolling panels.
- No long press, no hover-only controls, no double tap except the command ask's second tap.
- Respect the safe-area insets on all four edges.

## Accessibility

- Meet WCAG 2.2 AA in every theme and mode. The tokens already meet the contrast part; keep it so.
- Focus ring: 2 px solid `--action-text` with a 2 px offset, on every focusable element, using
  `:focus-visible`.
- Every control works by keyboard. Desktop shortcuts: `j` and `k` move, `Enter` opens, `1` to
  `9` pick an option, `w` writes an answer, `Esc` closes, `?` lists shortcuts. Single-key
  shortcuts never fire inside an input, and a setting turns them off.
- Each panel is a `region` labelled by its title. The tab bar is a `nav`.
- A new ask is announced once through a polite live region: "New ask from task 61." Never
  assertive.
- A state tile has the state word as its accessible name when the word is not shown beside it.
  Decorative icons are `aria-hidden`.
- The terminal is a `pre` with an accessible name ("Terminal of run 18") and a "Copy screen"
  button, so its text can be read elsewhere.
- Text reflows at 320 px wide and at 200 % zoom. Only the terminal, diagrams and wide tables
  scroll in two directions.
- Under `prefers-contrast: more`, `--line` takes the value of `--control-line`.
- Under `forced-colors: active`, tiles, cards and option buttons keep a 1 px `CanvasText` edge.

## Copy

- Use the terms of the PRD's Concepts table: task, run, ask, desk, inbox, queue, wait, result,
  lane, band, runner. Never "job", "ticket", "worker" or "prompt" for these.
- Show state names exactly as the daemon writes them, in lowercase.
- Name actions with the CLI verb: Answer, Resume, Cancel, Revive, Hold, Release, Move, Attach.
  Confirmations reuse the verb: "Answered", "Resumed".
- Errors say what failed, why, and what to do. No apologies, no "Oops", no exclamation marks.
- Times: relative under 24 h ("12 min", "3 h"), then a date ("Nov 18, 14:02"). The full
  timestamp is in the element's `title` and in `<time datetime>`.
- Sentence case for everything. No uppercase labels.
- Plain English, short sentences, active voice, no em dashes.
- Agent text appears as the agent wrote it, as text. Links in it show their host.

## When to change this file

Change `DESIGN.md` and the token files in the same pull request as the code that needs the
change, and update the artifact to match. A new component, state, token or theme starts here, not
in the code. A new theme defines every token in `themes.css`, passes the contrast checks above in
both modes, and adds a card to the Appearance screen.

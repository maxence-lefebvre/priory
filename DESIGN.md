# DESIGN.md

The design system for Priory's web view (`docs/prd.md`, R8). Follow it for every panel, view and
component. The live reference is the
[Priory design system artifact](https://claude.ai/artifact/3YYj2s5hmKksarCCC3VgHb): tokens, the
main components and their states in every theme. The token files in
[`design/tokens/`](design/tokens/) are the source of truth for values. When the artifact, this
file and the token files disagree, the token files win for values and this file wins for rules.
Fix the others. The artifact still shows the older Ledger layout and four themes; redraw it for
Explorer.

The M2 test is "one working day of asks answered from a phone only". Every decision below serves
that first: a person on a phone, often one-handed, reads an ask and answers it correctly.

The view has one structure, called Explorer, and four themes: Indigo (the default), Ink,
Parchment and Abbey, each in light and dark. A person picks a theme and a mode on the
Appearance screen.

Explorer treats every list as one query over the factory. Saved views and facet chips narrow it,
so nobody has to learn a syntax. The desk is the "Needs me" view. An ask opens into a detail that
puts the decision first and everything to read after it.

## Principles

1. **The desk comes first.** What waits on a person (asks, blocked tasks, uncertain prompts, room
   banners) is the default view and the first tab. Everything else is context for an answer.
2. **Ink for attention, the action color for actions, color for state.** The active view and the
   active tab are printed in solid ink. Buttons a person taps take the theme's action color. Every
   other color belongs to a state tile. A screen with nothing to do looks quiet.
3. **Decide first, then read.** An ask shows its question, recommendation and options before any
   context. The context follows for the person who doubts, and answering stays one tap away while
   they read.
4. **Dense, not cramped.** Rows show state, key, step and age in one glance. Density comes from
   tight type and hairlines, never from small touch targets.
5. **Phone first, desktop extends.** Design each panel at 360 px wide first. Wider screens show
   more panels side by side; they do not get different components.
6. **State is a tile: shape and word, then color.** Every state has an icon and its daemon name.
   Color only reinforces it.
7. **One structure, many themes.** A theme changes colors and the three font roles, nothing else.
8. **Never answer by accident.** No gesture answers an ask. Answers to command asks need a second,
   deliberate tap.
9. **Agent text is untrusted.** Render questions, options, summaries and terminal output as text
   or sanitized Markdown. Never as HTML.
10. **Offline is normal.** Show the last known state with its age when the connection drops.
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

- `data-theme` is `indigo`, `ink`, `parchment` or `abbey`. On `<html>`, a missing or
  unknown value gives Indigo.
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
  terminal, `--radius-card` (14 px) for option buttons, the answer bar's buttons, sheets and theme
  cards, `--radius-full` for the Answer capsule, saved views, live dots and badges. Facet chips
  take `--radius-sm`. Rows have no radius.
- **Elevation:** hairlines (`--line`) separate. Only the Answer capsule (`--shadow-float`) and
  sheets (`--shadow-sheet`) cast a shadow. A sheet on a phone sits over `--scrim`, which dims
  and disables everything behind it.
- **Sizes:** `--rail` (72 px) and `--drawer` (640 px) on wide screens, `--chip` (34 px) for views
  and chips, `--answer-bar` (64 px) for the sticky answer bar, `--measure` (68ch) for Markdown.
- **Motion:** `--dur-1` to `--dur-3` with `--ease-out` and `--ease-in`, all zero under
  `prefers-reduced-motion`.

### Type scale

| Token             | Size / line | Role    | Weight   | Use                                            |
| ----------------- | ----------- | ------- | -------- | ---------------------------------------------- |
| `--type-count`    | 64 / 0.85   | display | theme    | the desk's ask count on a phone                |
| `--type-count-wide` | 44 / 1    | display | theme    | the ask count in the wide list header          |
| `--type-screen`   | 34 / 40     | display | 700      | screen titles, such as Appearance              |
| `--type-task`     | 32 / 38     | display | 700      | the task title on wide screens                 |
| `--type-question` | 25 / 31     | display | 700      | the question in an ask's detail on a phone     |
| `--type-question-wide` | 22 / 28 | display | 700     | the question in the ask drawer                 |
| `--type-hero`     | 20 / 24     | display | 700      | "asks need you" beside the count               |
| `--type-read-h1`  | 20 / 26     | display | 700      | Markdown h1                                    |
| `--type-option`   | 17 / 22     | UI      | 600      | option labels, the Answer capsule, answer bar  |
| `--type-read-h2`  | 16 / 22     | UI      | 650      | Markdown h2 and h3, collapsible section titles |
| `--type-read`     | 15 / 23     | UI      | 400      | Markdown body: ask text, briefs and plans      |
| `--type-body`     | 15 / 20     | UI      | 400, 500 | row text, default text                         |
| `--type-small`    | 14 / 20     | UI      | 400, 500 | reasons, trade-off, banner text                |
| `--type-caption`  | 13 / 18     | UI      | 400, 600 | row metadata, section heads, ages              |
| `--type-mono`     | 12.5 / 19   | mono    | 400, 500 | task keys, ids, shas, commands, wide terminal  |
| `--type-mono-compact` | 12 / 16 | mono    | 400      | the terminal on compact screens                |
| `--type-tab`      | 11 / 14     | UI      | 600      | tab bar labels                                 |

- The display role takes `--display-stretch`; the count also takes `--count-stretch`,
  `--count-weight` and `--count-tracking`. A grotesk theme uses them for its wide cut; a serif
  theme sets them to plain values. The wide count halves `--count-tracking`.
- Set inputs at 16 px at every width. Smaller inputs make iOS Safari zoom the page.
- Use `font-variant-numeric: tabular-nums` for every column of numbers, durations and times.
- Keep reading text within `--measure`.

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
  in an Appearance preview. The `woff2` files and their licenses are in
  [`design/fonts/`](design/fonts/README.md).

### Theme tokens

| Token                                       | Meaning                                                                |
| ------------------------------------------- | ---------------------------------------------------------------------- |
| `--bg`                                      | page ground                                                            |
| `--surface`                                 | the drawer, sheets, option buttons, chips, inputs                      |
| `--raised`                                  | pressed rows, the segmented control track, skeletons, code and commands |
| `--line`                                    | hairlines between rows and around panels; never the only edge of a control |
| `--control-line`                            | edges of inputs, option buttons and secondary buttons, at least 3:1    |
| `--bar`                                     | the tab bar, the rail, the answer bar and the fade behind the capsule  |
| `--scrim`                                   | the dimmed backdrop behind a sheet; alpha over the page                |
| `--text`, `--text-2`, `--text-3`            | primary, secondary and metadata text                                   |
| `--emph`, `--on-emph`, `--on-emph-2`        | solid ink: the active view, the active tab, the current workflow step, an armed command option |
| `--action`, `--on-action`                   | primary buttons, the Answer capsule, the recommended option            |
| `--action-soft`, `--action-text`            | selected rows and rail items, set facet chips, the recommendation, links, ghost buttons, the focus ring |
| `--wait`, `--on-wait`                       | the ask tile: what waits on a person                                   |
| `--count`                                   | the desk's ask count                                                   |
| `--badge`, `--on-badge`                     | the count of open asks on the Desk tab                                 |
| `--tile-live`, `--tile-warn`, `--tile-danger`, `--tile-ok`, `--tile-hold`, `--tile-muted` | state tile fills |
| `--on-tile`, `--on-tile-live`               | the icon on a state tile; `--on-tile-live` on the live tile            |
| `--warn-text`, `--danger-text`, `--ok-text` | reasons and state words in text                                        |
| `--term-bg`, `--term-fg`, `--term-dim`      | the terminal, dark in every theme and mode                             |
| `--recommended`, `--on-recommended`         | derived: equal to `--action` and `--on-action`                         |

Contrast, checked for every theme in both modes: text tokens, state text tokens and
`--action-text` reach 4.5:1 on `--bg` and `--surface`, and `--action-text` also on
`--action-soft`; `--on-action`, `--on-emph`, `--on-emph-2`, `--on-badge` and the terminal text
reach 4.5:1 on their fill, and so does `--on-tile` on
`--tile-danger`, which is also the danger button; tile icons, `--on-wait` and `--control-line`
reach 3:1; the count reaches 3:1 on `--bg`. Re-check every pair with a contrast
tool after any change to a theme.

### The illuminated initial is dropped

An illuminated initial, the count in a gilded square, was tried for Parchment and Abbey. It is
dropped: it would be the one place a theme changes layout, breaking principle 7, and its 84 px
square shrinks the count. Those themes keep their character through their serif display role
and the rubric red `--count`.

## Layout

### Breakpoints

| Name    | Width         | Layout                                                                |
| ------- | ------------- | --------------------------------------------------------------------- |
| compact | below 600 px  | One panel at a time. Bottom tab bar. Detail pages push over the list. |
| medium  | 600 to 1023   | List and drawer side by side, list 320 to 400 px wide. Bottom tab bar. |
| wide    | 1024 and up   | Rail, list and drawer, or the view file's tree as written.            |

- A view file (R8.2) describes the wide layout. On compact screens, each leaf panel becomes a tab
  in the tree's order. The bottom bar shows the first four and a "More" tab for the rest.
- The default view puts the desk first, then asks, tasks and runs. The queue is a saved view of
  tasks. Inbox, workflows, reports and terminals sit under "More".
- "Asks" names the list of every ask. The PRD's inbox stays the log of events (R3).
- Panels lay out by their own width with container queries, not by the viewport.
- Every task, run and ask has its own URL, such as `/asks/42`. A link from a notification opens
  that ask directly, on any screen size.

### Frame on a phone

- **Top bar**, `--top-bar` (52 px), with a hairline under it: the factory name in the display
  role, the connection state as a dot and a word ("live", "reconnecting"), a filter button and a
  search button. A list panel shows its title in the display role instead of the factory name.
  Add lives beside the Answer capsule instead, in the thumb's reach (see [Add](#add)).
  It stays sticky below the safe-area inset.
- **Tab bar**, `--tab-bar` (60 px plus the bottom safe area), on `--bar`: Desk, Asks, Tasks,
  Runs, More, each an icon and a label. The active tab is a solid `--emph` pill. The Desk tab
  carries the open-ask count in `--badge`.
- The page never scrolls sideways. Only tables, diagrams and the terminal scroll inside their own
  box. Use `height: 100dvh` so mobile browser bars do not hide the tab bar.

### Frame on a wide screen

- **Top bar**: the factory name and connection state, a command bar in the middle ("Search or run
  a command", `Ctrl K`), the runner name, and the [Add](#add) button on the right.
- **Rail**, `--rail` (72 px) on `--bar`: Desk, Asks, Tasks, Runs, Inbox and Reports, each an
  icon over a label, then Settings at the bottom. The active item takes `--action-soft` and
  `--action-text`. The Desk item carries the open-ask badge.
- **List**: the selected view as an explorer list (below), at least 600 px wide at 1440 px. It
  holds no facet column and no histogram: facets are chips, and the histogram lives in Runs.
- **Drawer**, `--drawer` (640 px) on `--surface`: the selected ask, task or run. It opens beside
  the list and never covers it.
- **Key hint bar** at the bottom of the list, 40 px: `J` `K` move, `Enter` open, `1` to `9`
  answer, `W` write, `N` add, `[` `]` sections, `?` all shortcuts.

## Iconography

- **The mark** is a closed book with a P cut out of its cover and its ribbon hanging out, the
  ribbon marking the page where a person is needed. It is provisional. [`design/logo/`](design/logo/) holds the files and their
  rules: clear space, minimum sizes and colors.
- **Where the mark appears:** the favicon, the PWA and home screen icons, the README header and
  the Priory row of the Herdr sidebar. The view's top bar keeps the factory name in the display
  role and shows no mark.
- **Flat or two-tone:** below 24 px, and in Ink, the mark is flat in the row's text color. From
  24 px, the book takes `--text` and the ribbon `--action`. The favicon alone drops the P, whose
  counter closes at 16 px.
- **Interface icons** come from [Lucide](https://lucide.dev) (ISC license), inlined as SVG. The
  mark never stands for a panel, a state or an action.

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

An answered or withdrawn ask is not a task state either. In the asks list it takes a muted tile
with the `check` icon (answered) or the `minus` icon (withdrawn).

### Explorer list

Every list panel (desk, asks, tasks, runs, inbox) is the same explorer over one query.

1. **Saved views**: a row of chips, `--chip` tall with `--radius-full`, each a name and a count
   in `--text-3`. The active view is solid `--emph`. Views name a whole query, such as "Needs
   me", "Running", "Asleep" or "All". On a phone the row fits the width; a view that does not fit
   moves to the view's menu, and the row never scrolls sideways.
2. **Facet chips**: one chip per facet the panel offers (lane, band, risk, step, runner, age, task, command),
   `--chip` tall like the views, with `--radius-sm` and a `--control-line` edge. Tapping one
   opens a picker sheet with values and counts. A set chip takes `--action-soft` and `--action-text`, shows its value
   and an `x` that clears it. A last "Filter" ghost chip adds a facet. On a phone the facet row
   sits behind the top bar's filter button, except in the asks list.
3. **Query bar**, optional: hidden by default. Search on a phone, or the "Query" button on wide
   screens, opens it. It shows the current views and chips as text, such as `needs:me
   lane:infra/*`, so a person who knows the syntax can edit it. Nobody needs it to filter.
4. **Rows**, grouped by the view's sections, as described for each panel below.

On wide screens the list is a table: one row per item, `--row-wide` (38 px) at least. A long
question wraps to two lines instead of truncating.

### Desk

The desk is the "Needs me" view. It lists, top to bottom:

1. **Count**: the number of open asks in `--type-count` and `--count`, beside "asks need you" in
   `--type-hero` and a line in `--text-3`: "Oldest waits 42 min. 2 stopped." On wide screens
   the count sits at the right of the list header in `--type-count-wide`.
2. **Saved views**: Needs me, Running, Asleep, All on a phone; Queue joins them on wide screens.
3. **Room banners** (R4.3): a `--warn-text` edge on `--surface`, the `hard-drive` or
   `memory-stick` icon, and the daemon's sentence: what is short, by how much, since when, and
   the waiting task. The intake banner looks the same with the `unplug` icon: which source fails
   to poll, since when and why, or since when the intake has not polled.
4. **Asks**: the open asks, oldest first, as ask rows.
5. **Stopped**: blocked tasks, uncertain prompts (R5.4) and failed tasks, each with its reason in
   the role's text color. A blocked row carries Resume and Cancel buttons, a failed row Revive, in
   `.btn secondary` under its reason; its key opens the task page. On wide screens the Recommends
   column holds the same buttons, compact. Then **Also waiting**: the desk's other items (held tasks, sweeps, memory waits,
   retired tasks and worktrees that stay), each with the daemon's sentence.
6. **Up next**: a strip with the next 3 runs, queued or scheduled, as in the Herdr Queue panel.
   Each line shows when, the task and why, and opens the task. With none, the strip says "Nothing
   is queued or scheduled to run next."
7. **Asleep**: one row, "9 tasks asleep on waits", that opens the Asleep view. Each of its rows
   says what the task waits on and since when: "Waits on review github:o/r#7, 2 h", and opens
   the task page.
8. Reports waiting for review, when there are any.

On a phone, the **Answer capsule** floats above the tab bar: a full-width `--action` button,
`--capsule` (54 px) tall, `--capsule-gap` above the tab bar, with `--radius-full` and
`--shadow-float`, labelled "Answer 3 asks". It opens the oldest ask's detail. A fade from
transparent to `--bar` sits behind it. Wide screens have no capsule: `Enter` opens the
selected row.

- **The desk list ends with bottom padding of `--desk-end`**: the capsule's height plus its gap
  above and below, so the capsule never covers the last row when the list is scrolled to the end.
- The capsule hides when no ask is open. An empty desk shows: "No open asks. Agents ask here when
  they need you."
- A round [Add](#add) button, `--capsule` wide and tall, sits at the right of the capsule with
  `--space-2` between them. It stays where the capsule hides. It takes `--surface` with a
  `--control-line` edge and `--action-text`, so the capsule stays the one primary action.

### Add

Add queues a task by hand, as `priory add` does. It opens a sheet with one text field and three
chips, and nothing else. `N` opens it on a keyboard, the Add button in the wide top bar opens it
with a pointer, and the `+` beside the Answer capsule opens it on a phone.

- **The sheet** is a modal `dialog` over `--scrim`: "Add a task" in the display role and a close
  button, then the field, the chips, and the primary Add button at the bottom right. On a phone it
  slides up from the bottom edge with `--dur-3`; from 600 px it sits centered, 560 px wide. `Esc`, the
  close button and a tap on the scrim close it, and focus returns to the Add button.
- **The field** is 16 px text, "What should the factory do?" as its placeholder, with focus on open.
  `Enter` queues. A GitHub or Linear link in the text sets the repository and the key.
- **The chips** read "Workflow implement", "Band today" and "Lane routed": the name in `--text-3`,
  the value in `--text` at 500, and `chevron-down`. Each opens the platform's own picker. Workflow
  and Band start on the `[request]` defaults; Lane starts on "routed", which leaves the lane to the
  factory's lane rules. Workflow lists the factory's workflows, Band its bands in order, and Lane
  each lane a listed task carries. Chips are `--chip` tall with a `--control` target on a coarse
  pointer.
- **Keys and targets.** `N` never fires inside a field or while the sheet is open. Every control is
  at least `--control` tall on a coarse pointer and `--control-fine` on a fine one.

| State   | What the person sees |
| ------- | -------------------- |
| empty   | The field and the chips; Add is disabled (`aria-disabled`, `--text-3` on `--raised`). |
| sending | The field turns read-only and Add reads "Adding…". |
| refused | Under the field, in `--danger-text` with `circle-x`: "Could not add: " and the daemon's sentence. The text and chips stay. "Discard draft" appears. |
| added   | The sheet shows the new task in place: its state tile, key in mono, workflow and state, the request, band and lane, with "Added. The task waits in the queue for its turn." Then "Add another" (secondary) and "Close" (primary). |
| offline | A `--warn-text` line with `unplug`: "Offline. The draft stays on this device, and nothing is sent until you confirm it after you reconnect." Add reads "Keep draft" and sends nothing. |
| back    | Once the page reconnects, the line reads "Reconnected. This draft has not been sent. Send it now?" and the button reads "Send". Only that tap sends: the view never resends a draft by itself. |
| unsure  | A send lost its connection, or the daemon did not confirm it, so it may have queued the task. The line says so and keeps the draft. Once back, it says to check the Desk's All view, then send if the task is missing. |

The draft and its chips stay in the browser's local storage until the daemon takes the task, so a
reload or a closed tab keeps them.

Rows on the desk are at least `--row-min` (64 px) tall with a hairline above each: a state tile
on the left, then line 1 (task key in mono, step, tags, and the age on the right in `--text-3`),
line 2 (the question or reason, clamped at 2 lines), and a chevron. An ask row adds line 3 in
`--type-caption`: "Recommends" and the recommended option in `--text-2` at 600, then icons for
what the ask carries (`file-text` brief, `list-checks` plan, `git-pull-request` and the pull
request number). The whole row is the touch target.

### Asks list

The Asks tab lists every ask. Saved views: Open, Answered and All, each with a count. Facet
chips: Command, Lane, Band, Risk and Task, shown on a phone too. Band picks the task's band, with
its values in the factory's band order, then `none` for an ask with no band, such as an outside
ask. Risk picks what answering risks, with its values in this order: `irreversible`, when the
answer lets an irreversible step run or runs an agent's command; the task's grade `red`, `yellow`
or `green`; then `none` for an ask with neither. An ask can carry `irreversible` and a grade, and
counts under both. Risk is a facet chip like Band, with the same tokens, and its values are words,
so it adds no token. Each view shows these sections, in order, and leaves out an empty one:

| Section          | Open | Answered | All | Order        | Line 3                                       |
| ---------------- | ---- | -------- | --- | ------------ | -------------------------------------------- |
| Open             | yes  |          | yes | oldest first | "Recommends" and the recommended option      |
| Snoozed          |      |          | yes | oldest first | the `clock` icon and "Snoozed until 12:05"   |
| Answered today   |      | yes      | yes | newest first | "Answered", the answer, then who answered    |
| Answered earlier |      | yes      | yes | newest first | as above, with a date instead of a time      |
| Withdrawn        |      |          | yes | newest first | "Withdrawn" and why, such as "task revived"  |

Answered and withdrawn rows take the muted tiles from the state tile section. A long section
loads more rows as the person scrolls; its header keeps the total.

A snoozed ask leaves the Open section and the Open count until its snooze ends. The list reads the
asks of the last 7 days first; scrolling to the end of Answered or All loads the older ones.

### Ask detail

An ask opens into its detail: a full screen on a phone, the drawer on wide screens. It is ordered
for a decision. Every ask has its own URL, such as `/asks/42`.

**Decide** (the top, inside the thumb's reach on a phone):

1. **Header**: a back chevron with the list's name, the position "1/3" in the display role, and
   previous and next buttons.
2. **Context line**: the ask tile, task key, step, lane, a "Command" tag for a command ask, age
   and revision. It wraps. Tapping it opens the task.
3. **Question** in `--type-question`, or `--type-question-wide` in the drawer.
4. **Recommendation** on `--action-soft` with `--radius-md`: "Recommended: Merge." in
   `--action-text`, the agent's reason, then a `corner-down-right` line with what the agent does
   next after that answer.
5. **Options**: one full-width button per option, in the agent's order. Never reorder them. Each
   is at least `--option-min` (58 px) tall with `--radius-card` and a left-aligned label in
   `--type-option`. Ordinary options sit on `--surface` with a 1 px `--control-line` edge. The
   recommended option fills with `--recommended` and shows "Recommended" under its label, plus
   "Runs a command" when its answer runs one. In the drawer the options sit in one row, each with
   its key `1` to `9`.
6. **Write an answer**: a ghost button that expands into a 16 px text field with a "Send"
   button. Free text is always possible.
7. **Dismiss** and the [snooze](#snooze) buttons: ghost buttons on the same line as "Write an
   answer". Dismiss closes the ask with no answer and sends at once, with no undo bar, as
   `priory dismiss` does. Snooze shows only on an ask that belongs to a task.

**Read** (below, in the order a doubt follows): a row of section chips (Why, Brief, Plan, PR,
Trace, History, and Command in the drawer) jumps to each part and marks the one in view.

1. **Why**: what the agent says, as Markdown, with the run that asked; then "Trade-off." and
   "What waits." as two facts with the `scale` and `hourglass` icons.
2. **Brief**: the task's brief file as Markdown, with its file name and an open link.
3. **Plan**: the task's plan as Markdown; a checklist shows done items with `check` in
   `--ok-text`.
4. **PR**: the pull request summary (below).
5. **Trace**: the runs that led to the ask, as a waterfall (below).
6. **History**: the answer history (below).
7. **Command**: for a command ask, the command and its target (below).

A section the task lacks, such as a plan, is left out with its chip.

**Answer while reading.** On a phone, once the options scroll out of view:

- The header shrinks to one line: the question, clamped, over the task key, step and revision.
- The section chips stick under the header.
- A **sticky answer bar** on `--bar`, `--answer-bar` tall, holds two buttons with
  `--radius-card`: a secondary "3 options" with `chevron-up`, and the recommended option as a
  primary button with "Recommended" under its label. The recommended button answers in one tap,
  or arms a command ask. "3 options" opens the answer sheet.
- The **answer sheet** slides up over `--scrim`: "Answer ask #42" and the revision, every
  option, the command block for a command ask, and a close button. Tapping the scrim closes it.

In the drawer, the decide part stays pinned at the top and the reading part scrolls under it.
The pinned part takes at most 50 % of the drawer's height and scrolls on its own past that. In
it, the question clamps at 3 lines and the recommendation at 4, each with a "More" ghost button
that expands it. Options sit in one row while each label fits on two lines, and wrap to a column
otherwise. `[` and `]` move between sections.

- After an answer, the next ask slides in and the position updates.
- Previous, next and a horizontal swipe move without answering. Nothing answers except a tap on
  an option or "Send".
- Leaving the detail returns to the list at the same scroll position.

Ask states:

| State     | What the person sees                                                                      |
| --------- | ----------------------------------------------------------------------------------------- |
| open      | As above.                                                                                 |
| sending   | The chosen option shows a check; the others dim. A bar at the bottom says `Sending "Merge"` with an "Undo" button for 4 s. The view sends after the 4 s. |
| answered  | The card collapses to the question, the answer, who answered and when. No buttons.        |
| refused   | A `--warn-text` banner: "The question changed. Read revision 3 and answer again." The detail reloads the new revision; the old answer stays in the text field and in the history. |
| taken     | "Already answered 2 min ago: Merge." Shown when another answer came first.                |
| withdrawn | Muted: "Withdrawn: the task was revived." No buttons.                                     |
| offline   | Options stay visible but disabled, with "Reconnect to answer".                            |
| unsent    | A `--warn-text` line says why the answer did not go, such as "no priory daemon runs beside this server". The draft stays in the text field. |

A **command ask** (R6.8) binds one option to a command. Its answer runs the command, so:

- The option says so before any tap: "Recommended. Runs a command", or "Runs a command".
- The Command section and the answer sheet show the command in a mono block on `--raised`: each
  word of its argv in its own boxed chip, so a space, a quote or an empty word shows as it is, its
  target folder, the revision it is bound to, and "Runs at most once."
- It has no undo bar. The first tap arms the option: it turns solid `--emph` with a 3 px
  `--action` ring, reads "Tap again to run" over the option's name and "Cancels in 4 s", and a
  ring counts the 4 s down. The other options dim. The second tap sends. In the drawer, the
  option's key pressed twice does the same.
- While armed, the command block sits right under the armed option, in the options and in the
  sheet, or right above the answer bar when armed there: the command line, its target and
  "Bound to revision 2". The person sees exactly what the second tap runs without scrolling.
- After sending it shows the outcome: `running`, `done`, `failed` with the error, or `uncertain`
  in `--warn-text`: "Priory stopped while this ran. Check the target before you act on it."

### Markdown

Ask text, briefs and plans render as sanitized Markdown, never as HTML (principle 9).

- Body in `--type-read`, at most `--measure` wide. Headings: h1 in `--type-read-h1`, h2 and h3 in
  `--type-read-h2`. Lists indent 22 px with markers in `--text-3`.
- Inline code in mono 13 px on `--raised` with `--radius-sm`. Code blocks in `--type-mono` on
  `--raised` with `--radius-md`; a long line scrolls inside its block, never the page.
- A heading section longer than a screen can collapse: a `details` row, `--control` tall, with a
  `chevron-right` that turns 90 degrees when open, the heading, and a count in `--text-3`, such
  as "4 criteria". A brief opens with Goal and Scope expanded and the rest collapsed.
- Links show their host after the text. Images do not load; they show their alt text and host.

### Pull request summary

The pull request's title, then a line of branch, base, additions in `--ok-text`, deletions in
`--danger-text`, file count and head sha. Then the largest files, up to 4 on a phone and 5 in
the drawer: path in mono (ellipsized from the left), counts, and five squares of added and
deleted share in `--tile-ok` and `--tile-danger`. Then the checks, each a `check`, `x` or live
dot with its name and duration, and the review state.

### Trace waterfall and run blocks

The trace is the task's runs and waits on a time axis, ending at now.

- One row per run or wait, 28 px tall: a label (state icon in the role's text color, step and
  run id) and a bar on the axis. Bars: `--tile-muted` for moved and asleep runs, `--tile-warn`
  for lost ones, `--tile-ok` for done, a dashed outline for a wait, and stripes of `--wait` for
  the open ask. The duration sits after the bar, or before it near the right edge.
- The ask's row is selected: `--action-soft` with a 3 px `--action` bar.
- On a phone the label column is 138 px and the axis shows two ticks; in the drawer, 190 px and
  every half hour.
- **Run blocks** are the same bars without labels, 10 px tall in one strip: a task's runs at a
  glance in a row or a card.

### Answer history

Every revision and answer of the ask, newest first: a small tile, a sentence and a time in mono.
A refused answer shows on a warn tile with "Refused:" and the answer in `--warn-text`, and why.

### Appearance screen

Settings, then Appearance. It holds:

1. A screen title, "Appearance", in `--type-screen`.
2. **Mode**: a segmented control, System, Light and Dark, `--control` (44 px) tall on
   `--raised`. The selected segment sits on `--surface`.
3. **Theme**: one card per theme, in the order Indigo, Ink, Parchment, Abbey. Each card
   shows the desk in that theme, light and dark side by side at two fifths scale, the theme's
   name in the display role, one sentence about it, and a radio. The selected card has a 2 px
   `--action` edge and a filled radio.
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
- Actions live on the task page, not in the row: Resume, Cancel, Revive, Hold, Release, Move. The
  desk's stopped rows are the one exception, since acting on them is why they are there.

### Task detail and workflow grid

- Context line: task key in mono, the task's state tile and word, workflow, lane, band.
- Title in `--type-task`, then the task's reason in its role's text color.
- **Actions**, in `.btn secondary`: Resume and Cancel for a blocked task, Revive for a failed or
  dropped one, Cancel for any other active task. Cancel and Revive arm on the first tap, as a
  command option does: the button turns `--emph`, reads "Tap again to cancel" over its verb, and a
  ring counts 4 s down. The second tap sends. After it, a `--text-2` note says the verb and the
  daemon's sentence: "Cancelled: task 3 dropped."
- **Say**: for an active task, a text box "Say to the agent" and a primary Say button under the
  actions. The draft stays on this device until the daemon takes it.
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
- Allowed gestures: horizontal swipe between asks in an ask's detail and between tabs; pinch and
  drag on diagrams; the browser's own back swipe. Nothing else.
- No pull to refresh: panels update live. Set `overscroll-behavior: contain` on scrolling panels.
- No long press, no hover-only controls, no double tap except the second tap of a command ask, a
  Cancel or a Revive.
- Respect the safe-area insets on all four edges.

## Accessibility

- Meet WCAG 2.2 AA in every theme and mode. The tokens already meet the contrast part; keep it so.
- Focus ring: 2 px solid `--action-text` with a 2 px offset, on every focusable element, using
  `:focus-visible`.
- Every control works by keyboard. Desktop shortcuts: `j` and `k` move, `Enter` opens, `1` to
  `9` pick an option (twice for a command), `w` writes an answer, `n` adds a task, `[` and `]` move between
  sections, `Esc` closes, `?` lists shortcuts. Single-key shortcuts never fire inside an input,
  and a setting turns them off.
- The sheet over `--scrim` is a modal `dialog`: focus moves into it, stays there, and returns to
  the answer bar when it closes. The section chips are a `nav` of in-page links.
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

## Herdr adapter

Priory's panels also ship as Herdr mods (R8.6). A mod renders natively with gpui-kit, in a pane
beside other panes, with a fine pointer. It keeps this file's structure, tokens, state tiles, copy
and ask rules. This section lists only what differs, and the four panels the adapter ships first:
Desk, Asks, Queue and Bonjour. [`docs/adapters/herdr.md`](docs/adapters/herdr.md) explains where
the adapter lives, how it reads and acts, and the data each panel needs. The approved boards are in
`~/shared/priory-round-6/` on the author's machine.

### What differs from the web view

- **Density.** A pane takes the `--pane-*` tokens from
  [`design/tokens/structure.css`](design/tokens/structure.css): rows of at least `--pane-row`
  (52 px) with a `--pane-tile` (24 px) tile, views and chips `--pane-chip` (28 px) tall, options
  `--pane-option` (40 px) tall, the question in `--type-pane-question` (18 / 24), the desk count in
  `--type-pane-count` (40 / 1), and buttons at `--control-fine`.
- **Width.** Every panel works at `--pane-narrow` (420 px), the side panel's default width. Below
  `--pane-split` (720 px) a selected row opens its detail in place, under the row. From
  `--pane-split`, the list and the detail sit side by side.
- **Selection.** A selected row takes the `--action-soft` fill on its own rounded surface. It has
  no 3 px leading bar: gpui-kit's design guide forbids leading-edge selection bars.
- **Navigation.** Herdr owns tabs and panes, so a panel has no tab bar, rail or Answer capsule.
  `Enter` opens the selected row.
- **Add.** The Desk and Queue panels open [Add](#add) as a card under their header, not as a
  sheet: the field with Add and close buttons, then a row of chips per value, workflow, band and
  lane, with a line that says what happened. `N` opens it.
- **Overlays.** The answer sheet becomes a popover from the answer bar's "3 options". Secondary
  commands sit in a menu, opened by a `more-horizontal` button, and in a context menu. No command
  appears only on hover.
- **The armed command option** counts its 4 s down with a bar that drains under its label,
  instead of a ring.
- **Pointer.** Buttons keep the arrow cursor. Only links that leave Priory take the pointing hand.
- **Mode.** A mod draws with Priory's tokens, not Herdr's theme. The adapter resolves each
  `light-dark()` pair from Herdr's appearance.

gpui-kit lacks `light-dark()`, conic gradients, repeating stripes, `position: sticky`, container
queries and a line clamp with an ellipsis. So the answer bar and section chips sit outside the
scroll region, the open ask's trace bar is solid `--wait`, and a panel picks its layout from its
measured width. Check in a real mod whether variable font widths work; if not, the wide cuts of
Mona Sans need static font files.

### Snooze

A person can snooze any ask or Desk item. Snoozing hides the item for a while; the agent keeps
waiting, and the daemon does nothing different.

- **Snooze 1 h** and **Snooze until tomorrow 09:00** sit in the ask's overflow menu next to the
  answer bar, and beside "Write an answer". `S` snoozes the selected row for 1 h. The Desk's next
  thing card has a "Snooze 1 h" button.
- A snoozed item leaves the Needs me count, the Open count and the Desk badge. It moves to the
  saved view "Snoozed", with its count, on the Desk and in Asks.
- A snoozed row keeps its tile and shows "Snoozed until 12:05" in `--text-3`, with the `clock`
  icon. Its detail offers Unsnooze and the other snooze.
- A meaningful change brings the item back early: a new ask revision, a change of task state or a
  new failure. The row then shows the `rotate-ccw` icon and "Back early: new revision 2" in
  `--action-text`, and Since you looked lists it.

### Desk

The calm count, then the one next thing, then the explorer list. It opens as a tab in the center
stack.

1. **Count** in `--type-pane-count`, as the web desk's: "2 asks need you. Oldest waits 42 min. 1
   stopped. 2 snoozed."
2. **The next thing**: a card on `--surface` with the oldest open ask that is not snoozed, its
   recommendation, and the buttons Answer… (primary), Open task and Snooze 1 h. Answer… opens the
   ask in the Asks panel, so a person answers in one place.
3. **Saved views**: Needs me, Running, Asleep, Snoozed and All.
4. The room banner, Asks, Stopped and Later, as on the web desk.
5. **Since you looked**, from 1024 px: the events since the person's last visit.

From 1024 px the count, next thing, banner and Since you looked form a 440 px column; the rest is
the wide table with Recommends and the key hint bar (`J` `K`, `Enter`, `S`, `R`, `?`).

### Asks

Every ask, with the web's ask detail fitted to the pane. It sits in the side panel stack.

- Saved views: Open, Snoozed, Answered and All. Facet chips: Command, Lane, Band, Risk and Task.
- The detail keeps the web's order: context line, question, recommendation with what happens next,
  options with keys `1` to `9`, write an answer, then the section chips Why, Brief, Plan, PR,
  Trace and History.
- Once the options scroll out of view, the sticky answer bar shows "3 options", the recommended
  option and the overflow menu.

### Queue

What runs next and why. It sits in the side panel stack, under the same activity bar as Asks.

- **Up next**: a strip with the next 3 runs, queued or scheduled. Each line shows when ("now, on
  room", "about 11:50", "08:30 Fri"), the task or schedule, and why. It opens the task.
- Then Running, the bands in order (Blocker, Now, Today, Later, or the factory's names), Held with
  each hold's reason, and Last day, folded.
- Each waiting row shows its position in its band and why it has not started, in its role's text
  color. A grip in a fixed trailing lane, and `Shift J` and `Shift K`, reorder a band.
- An opened row offers Move…, Hold… and Cancel…, and Open task.

### Bonjour

The morning dossier. It opens as a tab in the center stack.

- A toolbar with the date and older and newer arrows, "n of m agreed" with a progress bar, Next
  pick and search.
- The greeting in the display role, then the report's sections **in the order the factory gives
  them**. The panel knows no section by name. Each section has a title, an optional count and
  rows; a row may carry a pick.
- A pick is a segmented control of its options. The recommended option shows a `check` in
  `--action-text` until a person chooses. An agreed pick folds to one line with Change.
- From 1024 px a 200 px list of the sections, with counts and "Picks", sits on the left. Below,
  the same list becomes section chips under the toolbar.
- **Threads** sit under their row, indented to the row's text, oldest first. An open thread is a
  `--surface` card: each comment's author in `--text-2` weight 600 and its time in `--text-3`, on
  one caption line, then its text in `--type-small`; replies follow in the same card. Reply and
  Resolve close the card. A resolved thread folds to one `--text-3` caption line, a `check` in
  `--ok-text`, "Resolved: " and its text, with Reopen.
- The selected row, when it has an id, ends with the comment field and a Comment button. A pick
  card, which a press does not select, shows a Comment button that selects it. Reply
  turns the button into Reply, with "Replying in thread 3" above the field and Cancel beside it.
  A refused comment says why under the row in `--warn-text`. While the field has focus it keeps
  every letter; Escape returns to the panel.

### Open questions

- **The same ask three times.** In the full layout the oldest ask shows as the Desk's next thing,
  as the Desk's first row and in the Asks panel. Max will tweak this next week.
- **Snooze in the web view.** Snooze is drawn for the Herdr panels. The web asks list shows
  snoozed asks as a section of All; a Snoozed saved view may follow when its slice lands.

## When to change this file

Change `DESIGN.md` and the token files in the same pull request as the code that needs the
change, and update the artifact to match. A new component, state, token or theme starts here, not
in the code. A new theme defines every token in `themes.css`, passes the contrast checks above in
both modes, and adds a card to the Appearance screen.

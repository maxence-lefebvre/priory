# DESIGN.md

The design system for Priory's web view (`docs/prd.md`, R8). Follow it for every panel, view and
component. The
[Priory design system artifact](https://claude.ai/artifact/3YYj2s5hmKksarCCC3VgHb) is an older
reference: tokens, the main components and their states in every theme. The token files in
[`design/tokens/`](design/tokens/) are the source of truth for values. When the artifact, this
file and the token files disagree, the token files win for values and this file wins for rules.
Fix the others. The artifact still shows the older Ledger layout and four themes; redraw it for
Explorer.

The M2 test is "one working day of asks answered from a phone only". Every decision below serves
that first: a person on a phone, often one-handed, reads an ask and answers it correctly.

The view has one structure, called Explorer, and five themes: Graphite (the default), Indigo,
Ink, Parchment and Abbey, each in light and dark. The
[Graphite board](https://claude.ai/artifact/PJWYYqCiEBRFnpVBE3HYRY) is the reviewed design for the
default theme and the components below. A person picks a theme and a mode on the
Appearance screen.

Explorer treats every list as one query over the factory. Saved views and facet chips narrow it,
so nobody has to learn a syntax. The desk is the "Needs me" view. An ask opens into a detail that
puts the decision first and everything to read after it.

## Principles

1. **The desk comes first.** What waits on a person (asks, blocked tasks, uncertain prompts, room
   banners) is the default view and the first tab. Everything else is context for an answer.
2. **Ink for actions, one accent for what waits on you, color for state.** Buttons a person taps
   take the theme's action color, ink in Graphite. The wait color marks only what waits on a
   person: the ask icon, the ask count and the Desk count. Every other color belongs to a state
   icon. Few shapes are filled: primary buttons, the active saved view and an armed option. A
   screen with nothing to do looks quiet.
3. **Decide first, then read.** An ask shows its question, recommendation and options before any
   context. The context follows for the person who doubts, and answering stays one tap away while
   they read.
4. **Dense, not cramped.** Rows show state, key, step and age in one glance. Density comes from
   tight type and hairlines, never from small touch targets.
5. **Phone first, desktop extends.** Design each panel at 360 px wide first. Wider screens show
   more panels side by side; they do not get different components.
6. **State is a stroke icon and a word, then color.** Every state has an icon and its daemon
   name. Color only reinforces it, and no state draws a filled square.
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
  carry its tuning: `--display-stretch`, `--display-weight`, `--count-stretch`, `--count-weight`
  and `--count-tracking`.

A theme never changes layout, spacing, radii, sizes, the type scale, motion, icons, copy or which
components appear. A design idea that needs any of these is a change to the structure, for every
theme, in this file.

### Choosing a theme and a mode

The view sets two attributes on `<html>`:

```html
<html data-theme="graphite" data-mode="system">
```

- `data-theme` is `graphite`, `indigo`, `ink`, `parchment` or `abbey`. On `<html>`, a missing
  or unknown value gives Graphite. A person who picked a theme before keeps it.
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
  `gap`, not margins. The side gutter is `--gutter`, 16 px or the safe-area inset, and 20 px
  in the wide layout.
- **Radii:** two sizes. `--radius-md` (6 px) for buttons, saved views, inputs, the segmented
  control, tab and rail items and the terminal. `--radius-card` (10 px) for panels: banners, the
  next-thing card, option buttons, the answer bar's buttons, the Answer capsule, sheets and theme
  cards. `--radius-sm` (4 px) for tags, key hints and facet chips. Progress and stage bars take
  `--radius-sm` too. `--radius-full` only for dots and radios, never for a button, a chip or a
  bar. `--radius-tile` (8 px) is the Herdr pane tile's. Rows have no radius.
- **Elevation:** hairlines (`--line`) separate. Only sheets and menus (`--shadow-sheet`) cast a
  shadow. The Answer capsule floats with a 1 px ring in `--bg` instead. A sheet on a phone sits
  over `--scrim`, which dims and disables everything behind it.
- **Sizes:** `--rail` (72 px) and `--drawer` (640 px) on wide screens, `--chip` (34 px) for views
  and chips, `--answer-bar` (64 px) for the sticky answer bar, `--measure` (68ch) for Markdown.
- **Motion:** `--dur-1` to `--dur-3` with `--ease-out` and `--ease-in`, all zero under
  `prefers-reduced-motion`.

### Type scale

| Token             | Size / line | Role    | Weight   | Use                                            |
| ----------------- | ----------- | ------- | -------- | ---------------------------------------------- |
| `--type-count`    | 28 / 32     | display | theme    | the desk's ask count on a phone                |
| `--type-count-wide` | 22 / 28   | display | theme    | the ask count in the wide list header          |
| `--type-screen`   | 34 / 40     | display | theme    | screen titles, such as Appearance              |
| `--type-task`     | 32 / 38     | display | theme    | the task title on wide screens                 |
| `--type-question` | 22 / 28     | display | theme    | the question in an ask's detail on a phone     |
| `--type-question-wide` | 22 / 28 | display | theme   | the question in the ask drawer                 |
| `--type-hero`     | 20 / 24     | display | theme    | "asks need you" beside the count               |
| `--type-read-h1`  | 20 / 26     | display | theme    | Markdown h1                                    |
| `--type-option`   | 17 / 22     | UI      | 600      | option labels, the Answer capsule, answer bar  |
| `--type-read-h2`  | 16 / 22     | UI      | 650      | Markdown h2 and h3, collapsible section titles |
| `--type-read`     | 15 / 23     | UI      | 400      | Markdown body: ask text, briefs and plans      |
| `--type-body`     | 15 / 20     | UI      | 400, 500 | row text, default text                         |
| `--type-small`    | 14 / 20     | UI      | 400, 500 | reasons, trade-off, banner text                |
| `--type-caption`  | 13 / 18     | UI      | 400, 600 | row metadata, section heads, ages              |
| `--type-mono`     | 12.5 / 19   | mono    | 400, 500 | task keys, ids, shas, commands, wide terminal  |
| `--type-mono-compact` | 12 / 16 | mono    | 400      | the terminal on compact screens                |
| `--type-tab`      | 11 / 14     | UI      | 600      | tab bar labels                                 |

- The display role takes `--display-weight` (600 in Graphite, 700 elsewhere) and
  `--display-stretch`; the count takes `--count-weight`, `--count-stretch` and `--count-tracking`
  instead. Graphite sets all of them to plain values: normal width, no tracking. Indigo and Ink
  use them for Mona Sans's wide cut. The wide count halves `--count-tracking`.
- Set inputs at 16 px at every width. Smaller inputs make iOS Safari zoom the page.
- Use `font-variant-numeric: tabular-nums` for every column of numbers, durations and times.
- Keep reading text within `--measure`.

## Themes

| Theme     | Character                                           | Display / UI / mono                         | Action | Waits on a person | Emphasis   |
| --------- | --------------------------------------------------- | ------------------------------------------- | ------ | ----------------- | ---------- |
| Graphite  | The default. A grey ramp, ink actions and one amber for what waits on you. | Instrument Sans / Instrument Sans / JetBrains Mono | ink | amber | ink |
| Indigo    | Ink emphasis with an indigo action.                 | Mona Sans / Mona Sans / JetBrains Mono      | indigo | indigo            | ink        |
| Ink       | Monochrome. What needs you is printed in solid ink. | Mona Sans / Mona Sans / JetBrains Mono      | ink    | ink               | ink        |
| Parchment | Parchment and iron-gall ink, rubric red and lapis.  | EB Garamond / IBM Plex Sans / IBM Plex Mono | lapis  | rubric red        | ink        |
| Abbey     | Vellum, lapis and gold, rubric red.                 | Alegreya / Alegreya Sans / IBM Plex Mono    | lapis  | rubric red        | deep lapis |

- **Amber marks only what waits on a person.** In Graphite, `--wait` and `--count` are one amber,
  `#A84D06` light and `#F0A04B` dark: the ask icon, the ask count, the Desk count and the `ask`
  state word. Warnings take a darker ochre, `--warn-text`. The grounds are a neutral grey ramp
  with no hue, and no value comes from a platform's system palette.
- **Rubric red marks only what waits on a person.** In Parchment and Abbey, red appears on ask
  tiles, the ask count and the Desk badge, and nowhere else. Actions, including the recommended
  option and the Answer capsule, take the lapis action color. Failure takes dark oxblood, and
  danger text a pale rose in dark mode, both apart from rubric red in lightness.
- **The recommended option is outlined in `--recommended`, which is the theme's action color in
  every theme.** It is never `--wait`, and it is filled only while armed.
- **Fonts:** all eight families are under the SIL Open Font License 1.1. Ship their `woff2` files
  with the daemon and serve them from the view. Never load fonts from a CDN: the view must work on
  a factory with no internet. Load a theme's families only when that theme is active or shown
  in an Appearance preview. The `woff2` files and their licenses are in
  [`design/fonts/`](design/fonts/README.md).

### Theme tokens

| Token                                       | Meaning                                                                |
| ------------------------------------------- | ---------------------------------------------------------------------- |
| `--bg`                                      | page ground                                                            |
| `--surface`                                 | the drawer, sheets, option buttons, chips, inputs, native select lists |
| `--raised`                                  | pressed rows, the segmented control track, skeletons, code and commands |
| `--line`                                    | hairlines between rows and around panels; never the only edge of a control |
| `--control-line`                            | edges of inputs, option buttons and secondary buttons, at least 3:1    |
| `--bar`                                     | the tab bar, the rail, the answer bar and the fade behind the capsule  |
| `--scrim`                                   | the dimmed backdrop behind a sheet; alpha over the page                |
| `--text`, `--text-2`, `--text-3`            | primary, secondary and metadata text                                   |
| `--emph`, `--on-emph`, `--on-emph-2`        | solid ink: the active saved view, the current workflow step, an armed command option |
| `--action`, `--on-action`                   | primary buttons, the Answer capsule, the recommended option's outline  |
| `--action-soft`, `--action-text`            | selected rows, set facet chips, links, ghost buttons, the focus ring   |
| `--wait`, `--on-wait`                       | what waits on a person: the ask tile in a Herdr pane, stripes in charts |
| `--count`                                   | the desk's ask count, the Desk count on the tab bar and rail, the ask icon and the `ask` state word |
| `--badge`, `--on-badge`                     | the Desk badge in a Herdr pane                                         |
| `--tile-live`, `--tile-warn`, `--tile-danger`, `--tile-ok`, `--tile-hold`, `--tile-muted` | chart and stage bar fills, and Herdr pane tiles |
| `--on-tile`, `--on-tile-live`               | the danger button's text, and the icon on a Herdr pane tile            |
| `--warn-text`, `--danger-text`, `--ok-text` | reasons and state words in text                                        |
| `--term-bg`, `--term-fg`, `--term-dim`      | the terminal, dark in every theme and mode                             |
| `--term-black`, `--term-red`, `--term-green`, `--term-yellow`, `--term-blue`, `--term-magenta`, `--term-cyan`, `--term-white` | the terminal's eight SGR colours, dark in every theme and mode |
| `--recommended`, `--on-recommended`         | derived: equal to `--action` and `--on-action`                         |

Contrast, checked for every theme in both modes: text tokens, state text tokens and
`--action-text` reach 4.5:1 on `--bg` and `--surface`, and `--action-text` also on
`--action-soft`; `--on-action`, `--on-emph`, `--on-emph-2`, `--on-badge` and the terminal text,
each of its eight colours included, reach 4.5:1 on their fill, and so does `--on-tile` on
`--tile-danger`, which is also the danger button; state icons, `--on-wait` and `--control-line`
reach 3:1; the count reaches 3:1 on `--bg`, and 4.5:1 in Graphite, where it is also a word. Re-check every pair with a contrast
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
- The default view puts the desk first, then asks, tasks and runs, then the inbox, the queue,
  reports, flows and costs. On a phone, these last five sit under "More". The queue is also a
  saved view of the desk on wide screens, and the Desk's Up next strip links to its panel.
  A view file that names the `terminal` panel adds Terminal, which lists the running tasks, each a
  link to its task page's terminal. The default view leaves it out.
- More lists those panels, then Appearance and Notifications, as rows with a chevron. "Sign out of
  this device" is a `.btn secondary` in its own group under them, with no chevron: it acts at
  once, and it opens no page.
- "Asks" names the list of every ask. The PRD's inbox stays the log of events (R3).
- Panels lay out by their own width with container queries, not by the viewport.
- Every task, run and ask has its own URL, such as `/asks/42`. A link from a notification opens
  that ask directly, on any screen size.

### Frame on a phone

- **Top bar**, `--top-bar` (52 px), with a hairline under it: the factory name in the display
  role and the connection state as a dot and a word ("live", "reconnecting"). A list panel shows
  its title in the display role instead of the factory name.
  Add lives beside the Answer capsule instead, in the thumb's reach (see [Add](#add)).
  It stays sticky below the safe-area inset.
- **Tab bar**, `--tab-bar` (60 px plus the bottom safe area), on `--bar`: Desk, Asks, Tasks,
  Runs, More, each an icon and a label. The active tab sits on `--raised` in `--text`, with
  `--radius-md`. The Desk tab carries the open-ask count as an 11 px mono numeral in `--count`,
  with no fill.
- The page never scrolls sideways. Only tables, diagrams and the terminal scroll inside their own
  box. Use `height: 100dvh` so mobile browser bars do not hide the tab bar.

### Frame on a wide screen

- **Top bar**: the factory name and connection state, and the [Add](#add) button on the right.
- **Rail**, `--rail` (72 px) on `--bar`: the default view's panels (Desk, Asks, Tasks, Runs,
  Inbox, Queue, Reports, Flows and Costs), each an icon over a label, then Settings at the
  bottom. The active item sits on `--raised` in `--text`. The Desk item carries the open-ask
  count as on the tab bar.
- **List**: the selected view as an explorer list (below), at least 600 px wide at 1440 px. It
  holds no facet column and no histogram: facets are chips, and the histogram lives in Runs.
- **Drawer**, `--drawer` (640 px) on `--surface`: the selected ask, task or run. It opens beside
  the list and never covers it. A person who arrives on the Desk's Needs me view finds its oldest
  ask open in the drawer, its row marked; once they close it, the drawer stays empty until they
  leave the Desk. Tasks, Runs and Queue have no drawer, nor has the Inbox in a plain tabs
  layout: the list takes the width.
- **Page width**: the task, batch and memories pages, and the Inbox's groups, are at most 960 px
  wide, against the rail. The Tasks, Runs and Queue tables keep the full width.
- **Back link**: its chevron lines up with the context line under it.
- **Key hint bar** at the bottom of the Desk, Asks and Inbox lists, 40 px: `J` `K` move, `Enter`
  open, `Esc` close, `N` add. It sits at `--gutter`, as the list's rows do.

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

A state tile names a task or run state: a 17 px Lucide stroke icon (1.75 px stroke) in the role's
color, centred in a `--tile` (20 px) column, with no fill and no edge. Next to it, the row shows
the daemon's exact word in lowercase, as `priory tasks` and `priory runs` print it. In wide
tables the icon is 15 px, before the word. A small tile, in history rows and context lines, is
an 18 px slot with a 15 px icon.

The icon and the word share the role's text color, which reaches 3:1 for an icon in every theme:
wait `--count`, live `--action-text`, warn `--warn-text`, danger `--danger-text`, ok
`--ok-text`, hold `--text-2`. A muted word takes `--text-2` and a muted icon `--text-3`.

An ask is not a state, but it takes a tile too: the `message-circle-question` icon in `--count`. A
running tile holds a live dot in `--action-text` instead of an icon; the dot pulses once per 2 s
and stays solid under reduced motion.

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

A scorer's verdict on a run (`factory.run.scored`) takes a tile too, with the verdict word as the
trace has it. An `error` means the scorer broke, not the run, so it is warn, never danger:

| Verdict          | Role   | Icon           |
| ---------------- | ------ | -------------- |
| `pass`           | ok     | `check`        |
| `fail`           | danger | `circle-x`     |
| `error`          | warn   | `circle-alert` |
| `not applicable` | muted  | `minus`        |

If the daemon reports a state not in these tables, show it as a muted tile with its raw word and
the `circle-help` icon. Add it here in the same change that adds it to the daemon.

Each role's fill in each theme, light / dark, from
[`design/tokens/themes.css`](design/tokens/themes.css). The web view and the Herdr panes draw
these only in charts, the stage bar and the danger button, never in a tile. The table leaves
Graphite out; `themes.css` holds its values.

| Role   | Token           | States                                               | Indigo                | Ink                   | Parchment             | Abbey                 |
| ------ | --------------- | ---------------------------------------------------- | --------------------- | --------------------- | --------------------- | --------------------- |
| wait   | `--wait`        | asks                                                 | `#5856D6` / `#5E5CE6` | `#0A0A0A` / `#EDEDED` | `#A4161A` / `#B9302A` | `#A31F1A` / `#B42A22` |
| live   | `--tile-live`   | running, ready                                       | `#5856D6` / `#5E5CE6` | `#0A0A0A` / `#EDEDED` | `#1F3F8F` / `#3F63C4` | `#24489C` / `#4466C8` |
| warn   | `--tile-warn`   | blocked, lost                                        | `#C96800` / `#C96800` | `#B45309` / `#B45309` | `#9A6400` / `#A36A00` | `#A86B00` / `#A86B00` |
| danger | `--tile-danger` | failed, exited                                       | `#D70015` / `#D70015` | `#CB1D26` / `#D92D35` | `#4A1A12` / `#6E2216` | `#5A1C17` / `#6E241E` |
| ok     | `--tile-ok`     | done                                                 | `#248A3D` / `#2E8B47` | `#18793A` / `#24873F` | `#3F6B2A` / `#4A7A33` | `#2F6B4F` / `#2F7A57` |
| hold   | `--tile-hold`   | held                                                 | `#8944AB` / `#9A55BF` | `#7340B8` / `#8550C8` | `#4B3F8F` / `#5C4FA8` | `#5B4A9C` / `#6A58B5` |
| muted  | `--tile-muted`  | queued, asleep, dropped, moved, restarted, cancelled | `#8E8E93` / `#636366` | `#8A8A8A` / `#555555` | `#8A7A5C` / `#6A5D48` | `#8C8170` / `#4C5468` |

An answered or withdrawn ask is not a task state either. In the asks list it takes a muted tile
with the `check` icon (answered) or the `minus` icon (withdrawn).

A batch is not a task state either. Its desk row takes a muted tile with the `layers` icon.

Memory candidates are not a task state either. Their desk row takes the ask role's `--wait` tile
with the `brain` icon, since they wait on a person.

Another runner's task shows its hold instead of a state: `kept` or `waiting`, beside a muted tile
with the `server` icon. `waiting` is a hold's word, never a task state of this runner.

### Explorer list

Every list panel (desk, asks, tasks, runs, inbox) is the same explorer over one query.

1. **Saved views**: a row of chips, `--chip` tall with `--radius-md` and a `--line` edge, each a
   name and a tabular count in `--text-3`. The active view is solid `--emph`, its count in
   `--on-emph-2`. Views name a whole query, such as "Needs
   me", "Running", "Asleep" or "All". On a phone the row fits the width; a view that does not fit
   moves to the view's menu, and the row never scrolls sideways.
2. **Facet chips**: one chip per facet the panel offers (lane, band, risk, step, runner, age, task, command),
   `--chip` tall like the views, with `--radius-sm` and a `--control-line` edge. Tapping one
   opens a menu under it with values and counts, at every width. A set chip takes `--action-soft` and `--action-text`, shows its value
   and an `x` that clears it. The facet row shows on a phone too.
3. **Rows**, grouped by the view's sections, as described for each panel below.

On wide screens the list is a table: one row per item, `--row-wide` (38 px) at least. A long
question wraps to two lines instead of truncating.

### Desk

The desk is the "Needs me" view. It lists, top to bottom:

1. **Count**: the number of open asks in `--type-count` and `--count`, beside "asks need you" in
   `--type-hero` and a line in `--text-3`: "Oldest waits 42 min. Also 2 stopped and 1 waiting."
   The line names every other item the Needs me view counts, stopped, to-dos, reports and also
   waiting, so the count and the Needs me chip add up. On wide screens the count sits at the
   right of the list header in `--type-count-wide`. With no open ask the desk stays quiet: no
   number, and "No asks need you" in `--type-body` and `--text-2`, over the same line, which
   still names any stopped work.
2. **The next thing**: a card on `--surface` with `--radius-card` and a `--line` edge, in every
   saved view, with the desk's `next` whatever its kind: an alarm (a blocked or failed task, an
   uncertain prompt or command, a failing intake) or the ask that ranks first. It shows the tile,
   key, step and age, the question or the reason on at most three lines, then `why` in the role's text color, and an
   ask's recommendation on at most two lines. One button opens what answers it: Answer… (primary) for an ask, Open
   task (secondary) for a task's item, and See the banner for a failing intake, which scrolls to
   the intake banner. On wide screens the card is at most 440 px wide. With no `next` it hides.
3. **Saved views**: Needs me, Running, Asleep, All on a phone; Queue joins them on wide screens.
4. **Room banners** (R4.3): a `--line` edge on `--surface` with `--radius-card`, the `hard-drive` or
   `memory-stick` icon, and the daemon's sentence: what is short, by how much, since when, and
   the waiting task. The intake banner looks the same with the `unplug` icon: which source fails
   to poll, since when and why, or since when the intake has not polled. So do the pause banner,
   with the `pause` icon and the desk's `paused` line, and one banner per day or week spend cap
   reached, with the `hourglass` icon and its line of `caps`. The order is autopilot, pause, room,
   intake, then the caps. The autopilot banner, with the `bot` icon in `--text`, says "Autopilot is
   on until 08:00 (high, every lane). It answered 3 and left 1." over Turn off in `.btn secondary`.
   A tap reads "Turning off" until the reply, and the banner goes once off. When no other runner was
   told, its sentence turns `--danger-text`, "Autopilot is off here, and no other runner was told:
   <why>.", over Tell them again. A refused off keeps the banner, with why in its note. Offline it
   says "Autopilot was on 4 min ago, until 08:00 (high, every lane)." and its button is disabled.
5. **Asks**: the open asks, oldest first, as ask rows. An ask routed to a lead carries the
   [routed mark](#routed-ask) in its row and in the next thing's card.
6. **Stopped**: blocked tasks, uncertain prompts (R5.4) and failed tasks, each with its reason in
   the role's text color. A blocked row carries Resume and Cancel buttons, a failed row Revive, in
   `.btn secondary` under its reason; its key opens the task page. On wide screens the Recommends
   column holds the same buttons, compact. When the list is narrower than 760 px, as beside the
   drawer at 1440 px, that column folds: its buttons, the recommendation ("Recommends open") and
   an ask's status, such as "Sent to desktop: approved", move under the question in the same
   cell, in `--text-2`, as on the phone row. Then **To-dos**: the person's own items, each with
   the `square` tile in the ask role, its text and due time, and a Done button in `.btn
   secondary` that marks it done (`POST /api/todos/{id}/done`). The next thing card offers Done
   for a to-do. Then **Also waiting**: the desk's other items (held tasks, going batches, sweeps,
   memory waits, retired tasks and worktrees that stay), each with the daemon's sentence. The
   [memory candidates](#memory-candidates) row names the candidates instead and opens their
   review list; on wide screens its Recommends column holds a Review button. A
   batch row stands for its entries' held rows. On the phone list, its title and counts are the
   main line and the `Next: ...` sentence a caption below; the wide table and the Herdr Desk keep the
   line whole, as board 3 shows. Its key opens the [batch page](#batch-page-and-lane-graph). A held task's row carries a
   Release button in `.btn secondary`, and a report row under **Reports** a Read button. Release
   sends the revision the page drew (`POST /api/tasks/{id}/release`), Read sends `POST /api/inbox/{seq}/read`.
   A refused action keeps the row and shows why under it, in the row's note.
7. **Up next**: a strip with the next 3 runs, queued or scheduled, as in the Herdr Queue panel.
   Each line shows when, the task and why, and opens the task. A queued task waiting for its place
   shows "now, when a runner has room"; one with its own wait reason shows "now" and that reason. With none, a
   plain sentence with no strip says "Nothing is queued or scheduled to run next." An "Open the
   queue" link ends the strip, or follows that sentence, and opens the Queue panel, where Move
   and Held live.
8. **Since you looked**: a strip with the events since the person's last visit, one group per
   task, the task with the newest event first. Each group opens with the task's key, which opens
   the task page, then each event's time and what: the event type in plain words, as the Inbox
   panel says it, then its subject ("GitHub pull request review submitted acme/shop#245: kept").
   Repeats do not fold: each event keeps its own line. The heading's note says "since 09:12", or
   "latest 50" before a first visit. With none, the strip says "Nothing happened since you
   looked." Once the strip is drawn, the page records the visit (`POST /api/seen`) and keeps the
   events it opened on until it reloads.
9. **Asleep**: one row, "9 tasks asleep on waits", that opens the Asleep view. Each of its rows
   says what the task waits on and since when: "Waits on review github:o/r#7, 2 h", and opens
   the task page.
10. **Unrouted**: when events reached no task in the last 24 hours, one calm row under Asleep,
   "3 events reached no task in the last 24 hours", with the `queued` tile and the count, that
   opens the [Inbox](#inbox) panel's Unrouted view. It never ranks and never takes an alarm color.
   Under it, when a task kept events no agent read (`counts.unheard`), one more calm row, "2 events
   kept by a sleeping task reached no agent", with the `asleep` tile and the count, that opens the
   same view at its Unheard section. Under that, when a pre script dropped requests
   (`counts.dropped`), one more calm row, "1 request a pre script dropped in the last 24 hours",
   with the `dropped` tile and the count, that opens the same view at its Dropped section.
11. Reports waiting for review, when there are any.
On a phone, the **Answer capsule** floats above the tab bar: a full-width `--action` button,
`--capsule` (54 px) tall, `--capsule-gap` above the tab bar, with `--radius-card`, no shadow and
a 1 px ring in `--bg`, labelled "Answer 3 asks". It opens the detail of the ask that ranks first (see `docs/daemon/desk.md`, The next thing). A fade from
transparent to `--bar` sits behind it. Wide screens have no capsule: `Enter` opens the
selected row.

- **The desk list ends with bottom padding of `--desk-end`**: the capsule's height plus its gap
  above and below, so the capsule never covers the last row when the list is scrolled to the end.
- Offline, the capsule leaves the primary look: `--surface` with a `--control-line` edge, `unplug`
  and `--warn-text`, reading "Read 3 asks, offline". It still opens the ask, whose options stay
  disabled until the view reconnects.
- The capsule hides when no ask is open. An empty desk shows: "No open asks. Agents ask here when
  they need you."
- A square [Add](#add) button with `--radius-card`, `--capsule` wide and tall, sits at the right of the capsule with
  `--space-2` between them. It stays where the capsule hides. It takes `--surface` with a
  `--control-line` edge and `--action-text`, so the capsule stays the one primary action.

### Memory candidates

`/memories` lists the memory candidates that wait for a person, oldest first, as a page of its own
with a back link to the Desk, like the batch page. Accepting one gives it to every task's runs, so
the page shows everything a person needs before they tap:

- A heading with the count, and one note: an agent wrote each one; Accept gives it to every task's
  runs, on every runner; Drop discards it.
- Per candidate, in a section: a small memories tile, the name in mono, a tag, and the age. The tag
  is `new`, `forget`, or in `--warn-text` `replaces a memory runs get`, `forgets a memory runs
  get` or `may replace a memory runs get`, when this machine cannot tell, such as one with no kit
  checkout. It never says `new` unless the server confirmed it. Then the description in `--text-2`, the full text on `--raised` with `--radius-md`, never
  cut, and for a candidate that replaces a memory, the current text behind a "Current text"
  disclosure, closed by default, which stays as the person left it across redraws. Then "Written by KEY on RUNNER, candidate ID" in `--text-3`.
- Accept in `.btn primary` and Drop in `.btn secondary`. Accept arms on its first tap, as Cancel
  does, and sends on the second within 4 seconds. Drop sends at once: a dropped candidate reaches
  no other task.
- Once sent, both buttons turn off and a note under them says what the server answered. The
  candidate leaves the list once the daemon reads the verdict from the log. A refusal says why in
  the same note.
- With none waiting: "No memory candidate waits. Agents write them with the remember and forget
  tools."

### Inbox

The Inbox panel, in the rail and under More on a phone, is an [explorer list](#explorer-list) with
one saved view, **Unrouted**, which counts the events of a task's resources it lists. It lists the
`owned` groups of `/api/inbox?unrouted=1`, one per resource, newest first. A group shows the
resource in mono with its event count, then its task as a caption line: state tile, "task 12"
linking to the task page, state and key. Then its 5 newest events as history rows (`queued` tile,
type in plain words in bold, such as "Linear issue updated" for `linear.issue.updated`, subject, the event's first line in `--text-2` under them, time), and "and 3 older"
past that. Under them, the one action that routes the next event, by the task's state: the task
page's **Revive** (armed, as there) for a failed or dropped task; **Request a task** for a done
one, which opens the [Add](#add) sheet with the task's workflow, or the default when the factory
opts it out (one the factory lacks stays, and the daemon refuses it), and the group's `request_text`,
whose GitHub or Linear URL makes the new task's key that resource (a Slack thread gets a
`--text-2` line instead: "No new task can take this resource from here: start one from its
thread."); **Subscribe task 12** for an active one (`POST /api/subscribe` with the revision the
Inbox read was at). The daemon's sentence shows under the row, as a
desk action's note does. A last `--text-2` line counts the events no task ever owned: "No task
ever owned: 41 github events, 6 slack events." With no owned group: "No event of a task's resource
went unrouted. An event about an ended task's pull request or issue shows here."

When `unrouted.unheard` has groups, an **Unheard** section, "no agent read them", comes first, then
an **Unrouted** heading, "reached no task", over the owned groups. Each unheard group is the owned
group's card for one task: its key in mono with its event count, the caption line with its
workflow, then its 5 newest events with the `asleep` tile. Under them, by the task's state:
**Say to task 12** (`.btn secondary`, one tap, not armed) for an asleep task, which sends the
group's `say_text` through the task's say verb, with "Say sends “...”" in `--text-2`; Say and the
task page's **Resume** for a blocked one, whose line adds "It reads them when it resumes."; the
owned group's Revive or Request a task for an ended one; "Its next run reads them." otherwise.

When `unrouted.dropped` has entries, a **Dropped** section, "a script judged each once", comes
after Unheard and before Unrouted. Each entry is a group card with no action: its key in mono with
its event count, a caption line with the `dropped` tile, "trigger linear-view,
scripts/has-estimate", then its source event as a history row with the `dropped` tile, then a
`--text-2` line with why and the way back: "Dropped: no estimate yet. A new event of it that a
trigger wants runs the script again." A script that failed reads "Dropped: the script failed:
exit status 2."
Events open no detail, so the Inbox has no drawer on wide screens.

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
  and Band start on the `[request]` defaults, and follow them until the person picks; Lane starts on "routed", which leaves the lane to the
  factory's lane rules. Workflow lists the factory's requestable workflows, read each time the sheet opens, Band its bands in order, and Lane
  each lane a listed task carries. Chips are `--chip` tall with a `--control` target on a coarse
  pointer.
- **Keys and targets.** `N` never fires inside a field or while the sheet is open. Every control is
  at least `--control` tall on a coarse pointer and `--control-fine` on a fine one.

| State   | What the person sees |
| ------- | -------------------- |
| empty   | The field and the chips; Add is disabled (`aria-disabled`, `--text-3` on `--raised`). |
| swapped | Opened by **Request a task** for a workflow that is not requestable: a `--warn-text` line with `circle-alert`, "Workflow sweep is not requestable, so this starts the default, implement.", or "Workflow sweep is not requestable. Pick one." with no `[request]` default. It leaves once the person picks a workflow, discards the draft or adds the task. |
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

The Asks tab lists every ask. Saved views: Open, Snoozed, Answered and All, each with a count. Facet
chips: Command, Lane, Band, Risk and Task, shown on a phone too, and Runner in a factory of several
runners. Band picks the task's band, with
its values in the factory's band order, then `none` for an ask with no band, such as an outside
ask. Risk picks what answering risks, with its values in this order: `irreversible`, when the
answer lets an irreversible step run or runs an agent's command; the task's merge risk `high`,
`medium` or `low`; the task's grade `red`, `yellow` or `green`; then `none` for an ask with none of
them. An ask can carry `irreversible`, a merge risk and a grade, and counts under each. Risk is a facet chip like Band, with the same tokens, and its values are words,
so it adds no token. Each view shows these sections, in order, and leaves out an empty one:

| Section          | Open | Snoozed | Answered | All | Order        | Line 3                                       |
| ---------------- | ---- | ------- | -------- | --- | ------------ | -------------------------------------------- |
| Open             | yes  |         |          | yes | oldest first | "Recommends" and the recommended option      |
| Snoozed          |      | yes     |          | yes | oldest first | the `clock` icon and "Snoozed until 12:05"   |
| Answered today   |      |         | yes      | yes | newest first | "Answered", the answer, then who answered    |
| Answered earlier |      |         | yes      | yes | newest first | as above, with a date instead of a time      |
| Withdrawn        |      |         |          | yes | newest first | "Withdrawn" and why, such as "task revived"  |

Answered and withdrawn rows take the muted tiles from the state tile section. A long section
loads more rows as the person scrolls; its header keeps the total.

A snoozed ask leaves the Open section and the Open count until its snooze ends. The list reads the
asks of the last 7 days first; scrolling to the end of Answered or All loads the older ones.

### Routed ask

An open ask of a `hands-off` task can be routed to a lead agent (`routed_to`). It shows a **routed
mark**, "with lead", in the lead's name:

- **Rows** (Desk, Asks list): a `.tag` beside "Command" on line 1, or after the question in the
  wide table. It shrinks with an ellipsis when the row is narrow. The next thing's card shows the
  mark as a line in `--action-text` under the question. In Herdr it is a line of text. An answered
  or withdrawn ask has no mark.
- **Ask detail**: under the question, a line in `--action-text`, with the `corner-down-right` icon
  on the web: "lead is answering. You can still answer first: the first answer stands." The options stay enabled.
- The mark adds no token and no color: it reuses `.tag` and `--action-text`. An unrouted ask looks
  as before.

### Another runner's ask

In a factory of several runners, the lists mix every runner's asks in the usual order
([runners.md](docs/daemon/runners.md#every-runners-asks)). An ask of this runner looks as before.
Another runner's ask shows:

- **Rows** (Desk, Asks list): a `.tag` with the runner's name on line 1, which never shrinks. Once
  the runner's last alive message is over 3 minutes old, the tag takes `--warn-text` and line 3
  starts with "desktop last seen 40 min ago" in `--warn-text`. The wide table shows the whole tag,
  "desktop, last seen 40 min ago", after the question.
- **Runner facet**: a facet chip like Band, offering each runner with its count. This runner reads
  "workstation (here)". It shows only when the asks name runners.
- **Sent**: an answer or dismiss to another runner's ask is a submission, which its runner applies
  when it reads the log. Until then line 3 reads "Sent to desktop: approved" in `--action-text`, and
  the detail says under the question "Sent to desktop: approved. It applies when desktop reads the
  log." A refused submission reads "desktop refused approved: why" in `--warn-text` instead. An
  applied one shows as the ask's own answer.
- **Could not send**: a write the log did not take shows its error in place of the Sent line:
  nothing was sent, and the draft stays.
- **Command**: the armed option's command block and the Command section say "Runs on laptop".
- **Ask detail**: the context line carries the runner tag; under the actions a note says "This ask
  belongs to laptop. Your answer goes there through the log, and laptop applies it when it reads
  it. Snooze stays on laptop." Snooze is hidden, and the context does not link to the task, which
  lives on that runner.

It adds no token: it reuses `.tag`, `.facet`, `--action-text` and `--warn-text`.

### Ask detail

An ask opens into its detail: a full screen on a phone, the drawer on wide screens. It is ordered
for a decision. Every ask has its own URL, such as `/asks/42`.

**Decide** (the top, inside the thumb's reach on a phone):

1. **Header**: a back chevron with the list's name, the position "1/3" in `--type-small` and
   `--text-2` with tabular numbers, and previous and next buttons.
2. **Context line**: the ask tile, task key, step, lane, a "Command" tag for a command ask, age
   and revision. It wraps. Tapping it opens the task.
   An ask whose answer lets an irreversible step run, and that has a `risk_estimate`, adds a
   **History line** under it: the `lock` icon and the server's `line` in `--type-caption` and
   `--text-2`. It takes no color, tile or level, so it never reads as a verdict on this change.
3. **Heading** in `--type-question`, or `--type-question-wide` in the drawer: the ask's title.
   An untitled ask uses its question when that is one line of at most 80 characters. Otherwise
   the heading is "ENG-212 asks at approve-pr", or the question's first sentence for an ask with
   no task, cut at a word to about 80 characters with an ellipsis. A long agent question never
   fills the heading, so the options stay in view. Rows, the Desk's next thing, a task's open
   asks and the Herdr panels use the same heading.
4. **Recommendation**, with no box: "Recommended: Merge." in `--text` at 600, the agent's reason
   in `--text-2`, then a `corner-down-right` line with what the agent does next after that
   answer.
5. **Options**: one full-width button per option, in the agent's order. Never reorder them. Each
   is at least `--option-min` (58 px) tall with `--radius-card` and a left-aligned label in
   `--type-option`. Ordinary options sit on `--surface` with a 1 px `--control-line` edge. The
   recommended option keeps the `--surface` fill with a 1.5 px `--recommended` edge, and shows
   "Recommended" under its label in `--text-3`, plus "Runs a command" when its answer runs one. In the drawer the options sit in one row, each with
   its key `1` to `9`.
6. **Write an answer**: a ghost button that expands into a 16 px text field with a "Send"
   button, on every open ask. A step's ask and a hold's ask need an answer, so they show no
   Dismiss (`dismissable` is false).
7. **Dismiss** and the [snooze](#snooze) buttons: ghost buttons on the same line as "Write an
   answer". Dismiss closes the ask with no answer and sends at once, with no undo bar, as
   `priory dismiss` does. Snooze shows only on an ask that belongs to a task.

**Read** (below, in the order a doubt follows): a row of section chips (Why, Brief, Plan, PR,
Trace, History, and Command in the drawer) jumps to each part and marks the one in view. The
chips read as tabs: words in `--text-2`, and the one in view in `--text` over a 2 px `--text`
underline, with no fill.

1. **Why**: the full question in `--type-read`, with its line breaks and links, when the heading
   does not show all of it; then what the agent says, as Markdown, with the run that asked; then "Trade-off." and
   "What waits." as two facts with the `scale` and `hourglass` icons.
2. **Brief**: the task's brief file as Markdown, with its file name and an open link.
3. **Plan**: the task's plan as Markdown; a checklist shows done items with `check` in
   `--ok-text`. Each file of it has its own `.sh` line with its path.
4. **PR**: the pull request summary (below).
5. **Trace**: the runs that led to the ask, as a waterfall (below).
6. **History**: the answer history (below).
7. **Command**: for a command ask, the command and its target (below).

A section the task lacks, such as a plan, is left out with its chip.

An open ask that **approves a plan** puts Plan first, before Why, as the section the detail opens on.
Its options are disabled until the plan of its revision on screen is read, under a `layers` note,
"Reading the plan of revision 2 before you answer.", or "Could not read the plan: ..." when the read
fails. The answer bar shows the same note. When a file is cut, a `.note.warn` under the options, in the
answer bar and in the answer sheet says "The plan is cut at 64 KB. Read the rest at ... before you
answer.".

A **go ask**, the human step of a batch's planning task, reads the batch first. Its chips start with
Plan, Closes and Splits, then Why and the task's other sections. Plan is the [lane graph](#batch-page-and-lane-graph) with
numbered circles and each entry's files, under "Plan, 8 tasks in start order" and the plan revision.
Closes lists each issue the go closes, with why and its evidence; Splits each split issue, its parts
and why. The task's own plan result is left out, since the batch plan says the same. While the batch
is `asked`, "Edit the plan", a ghost button with `pencil` before the snooze buttons, jumps to the
Plan, and each entry row ends in a `more` icon button that opens its menu under the row: Move up,
Move down and Drop, `.btn secondary` at `--control` height. A move with no neighbour that way is
disabled. Drop on an entry that others start after arms first, as Cancel does on a task: the button
turns `--emph` with an `--action` ring and reads "Tap again to drop it and ENG-41.2, ENG-42". Each
edit sends the plan revision the page drew; the plan's `.note` then says the daemon's sentence, such
as "batch 4 is at plan revision 3", or why it was refused. Edit the plan never answers. The plan
shows only once it is read for the ask's revision on screen, so a Go never answers a plan the page
did not show. Until then the options are disabled under a `layers` note, "Reading the plan of revision
2 before you answer.", or "Could not read the plan: ..." when the read fails.

**Answer while reading.** On a phone, once the options scroll out of view:

- The header shrinks to one line: the heading, clamped, over the task key, step and revision,
  such as "ENG-212, approve-pr, revision 1", once the options scroll above the view. Options still
  below the fold leave the header whole. The back link keeps its chevron and drops its word, at
  `--control` width, and the position leaves. Scrolling the options back restores the header.
- The section chips stick under the header.
- A **sticky answer bar** on `--bar`, `--answer-bar` tall, holds two buttons with
  `--radius-card`: a secondary "3 options" with `chevron-up`, and the recommended option as a
  primary button with "Recommended" under its label. The recommended button arms on the first
  tap and answers on the second, as every option does. "3 options" opens the answer sheet.
- The **answer sheet** slides up over `--scrim`: "Answer ask #42" and the revision, every
  option, the command block for a command ask, and a close button. Tapping the scrim closes it.

In the drawer, the decide part stays pinned at the top and the reading part scrolls under it.
The pinned part takes at most 50 % of the drawer's height and scrolls on its own past that. In
it, the question clamps at 3 lines and the recommendation at 4, each with a "More" ghost button
that expands it. Options sit in one row while each label fits on two lines, and wrap to a column
otherwise.

- After an answer, the next ask slides in and the position updates.
- Previous, next and a horizontal swipe move without answering. Nothing answers except a second
  tap on an armed option or "Send".
- **Every option arms first.** The first tap arms an option: it turns solid `--emph` with a 3 px
  `--action` ring, reads "Tap again to send" over "Confirm: Merge" and "Cancels in 4 s", and a
  ring counts the 4 s down. The other options dim. A second tap on it within the 4 s sends at
  once, with no undo bar. A second tap within 250 ms of the first does nothing, so an accidental
  double tap cannot send. A held key does not repeat into a send. A tap on another option moves the arm there. After the 4 s the option
  disarms, and the next tap arms it again. A new revision of the ask disarms it, so a second tap
  never answers an older question. In the drawer, the option's key pressed twice does the same.
- "Send" under the text field sends at once: pressing it is already a second, deliberate step.
- Leaving the detail returns to the list at the same scroll position.

Ask states:

| State     | What the person sees                                                                      |
| --------- | ----------------------------------------------------------------------------------------- |
| open      | As above.                                                                                 |
| sending   | The chosen option shows a check; the others dim. A bar at the bottom says `Sending "Merge"` while the request is in flight. It has no Undo. |
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
- It arms on the first tap as every option does, but reads "Tap again to run". The second tap
  sends the answer and runs the command.
- While armed, the command block sits right under the armed option, in the options and in the
  sheet, or right above the answer bar when armed there: the command line, its target and
  "Bound to revision 2". The person sees exactly what the second tap runs without scrolling.
- After sending it shows the outcome: `running`, `done`, `failed` with the error, or `uncertain`
  in `--warn-text`: "Priory stopped while this ran. Check the target before you act on it."

### Markdown

Ask text, briefs and plans render as sanitized Markdown, never as HTML (principle 9).

- The web view parses a subset itself: paragraphs, headings, bullet, numbered and check lists,
  bold, inline code, fenced code blocks, links and images. Anything else, raw HTML included, shows
  as escaped text. A fence line's other words show as a `.info` line in mono over the block, and only
  a bare fence at least as long as the opening one closes it. A link or image whose target is not `https:` shows as its source text.
- A check list's open item shows an empty box in `--text-3`.

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
dot with its name and duration, and the review state. A check that passed takes the `check`; one
with no conclusion yet takes the live dot and "running"; a `skipped` or `neutral` one takes the
muted `minus`; every other conclusion takes the `x`. Check and review states read in plain words:
"passed", "changes requested", "timed out".

A task with a stack of two or more pull requests adds "Stack, bottom first: 3 PRs" in
`--text-2`, then one `.hist` row per pull request, bottom first: a small tile, the number as a link
in mono, "branch into base", the checks when known ("1 of 2 checks failing", for the latest one
only), and the state at the trailing edge. Tiles: merged `done`, closed `dropped`, the next to merge
`ready`, other open ones `queued`. The lowest open one reads "open, next to merge" in
`--action-text`. One pull request keeps the summary above with no stack rows. A stack whose latest
pull request has no events yet shows the stack rows alone, under the title "3 PRs".

### Trace waterfall and run blocks

The trace is the task's runs and waits on a time axis, ending at now.

- One row per run or wait, 28 px tall: a label (state icon in the role's text color, step and
  run id) and a bar on the axis. Bars: `--tile-muted` for moved and asleep runs, `--tile-warn`
  for lost ones, `--tile-ok` for done, a dashed outline for a wait, and stripes of `--wait` for
  the open ask. The duration sits after the bar, or before it near the right edge.
- The ask's row is selected: `--action-soft`, with no leading bar.
- On a phone the label column is 138 px and the axis shows two ticks; in the drawer, 190 px and
  every half hour.
- **Run blocks** are the same bars without labels, 10 px tall in one strip: a task's runs at a
  glance in a row or a card.

### Answer history

Every revision and answer of the ask, newest first: a small tile, a sentence and a time in mono.
The answer of the current revision says who gave it: "Answered Merge by agent:lead", or "by a
person" for an answer from the CLI or the web view. An answer from before Priory recorded who
answered says nothing. An answered row in the lists and the answered card say it too: "Answered
Merge by agent:lead", and "By agent:lead." after the age.
A refused answer shows on a warn tile with "Refused:" and the answer in `--warn-text`, and why.
Another runner's ask also lists each answer and dismiss sent to it, at the time it was sent: "Sent
approve to desktop for revision 2. Not applied yet" on a `queued` tile, then "desktop applied it" on an
`answered` tile, or on a `withdrawn` tile for a dismiss. A refused one reads "desktop refused it: why"
on a warn tile in `--warn-text`.

### Appearance screen

Settings, then Appearance. It holds:

1. A screen title, "Appearance", in `--type-screen`.
2. **Mode**: a segmented control, System, Light and Dark, `--control` (44 px) tall on
   `--raised`. The selected segment sits on `--surface`.
3. **Theme**: one card per theme, in the order Graphite, Indigo, Ink, Parchment, Abbey. Each card
   shows the desk in that theme, light and dark side by side at two fifths scale, the theme's
   name in the display role, one sentence about it, and a radio. The selected card has a 2 px
   `--action` edge and a filled radio.
4. A note in `--text-3`: "A theme changes color and type only. Every screen keeps the same
   layout."

- Choosing applies at once, with no save button, and is stored on this device.
- The cards are a `radiogroup`; each card is a `radio` named by its theme.
- Each preview renders the real desk component with the theme's attributes, not a picture, so it
  can never drift from the theme.

### Notifications screen

Settings, then Notifications, reached from More on a phone and from the Appearance screen on wide
screens. It turns Web Push on or off for this device. It holds:

1. A screen title, "Notifications", in `--type-screen`.
2. One sentence in `--text-2`: "Get a notification on this device when something waits on you. It
   shows a title and opens the item. A snoozed item sends nothing."
3. A status note, a `role="status"` line, when there is something to say:

   | Status      | Note |
   | ----------- | ---- |
   | on          | `check`: "On for this device." |
   | unconfirmed | "On in this browser, but the daemon did not confirm it. Check that the daemon runs, then reload the page." |
   | blocked     | "Notifications are blocked for this site. Allow them in the browser's site settings, then come back." |
   | install     | iOS Safari outside the Home Screen: "On iPhone and iPad, only Priory on the Home Screen can get notifications. Tap Share, then Add to Home Screen, and open Priory from there." |
   | insecure    | "Notifications need HTTPS. Open Priory at its https address, such as the one tailscale serve gives, or at localhost on this machine." |
   | unsupported | "This browser cannot get notifications from a web page." |

4. While they are on, a `fieldset` named "Notify this device about", in `--type-read-h2`, with one
   native checkbox per kind. A row is at least 60 px tall and wholly tappable, with a hairline above
   it: a 20 px box in `--action`, the kind's name at 600, and a sentence in `--type-caption` and
   `--text-2`. The kinds are Asks, Stopped tasks, Intake failures, Uncertain work, Spend caps, Landed
   wrong and Queue stalls. A change applies at once. While it saves, the boxes are `aria-disabled`.
   A refusal puts the box back and shows a `--warn-text` alert: "Could not save what this device
   gets: why. Check that the daemon runs, then try again." A lost reply proves nothing, so the boxes
   hide and the status note reads as unconfirmed. With no box checked, a note says "Nothing is
   chosen, so this device gets no notification." Under it: "Each device keeps its own choice. A
   change applies at once."
5. One button: "Turn on notifications", primary, while they are off; "Turn off notifications",
   secondary, while they are on or unconfirmed. The permission prompt opens only from this tap,
   as iOS requires. A failure shows a `--warn-text` alert: "Could not turn on notifications: why.
   Check that the daemon runs, then try again."
6. "Install Priory", secondary, only when the browser offers to install the view, with the note
   "Installing opens Priory in its own window, so notifications reach it with no tab open."

The view never asks for the permission on its own: no banner and no prompt at load.

A notification's title is "Priory". Its text is the ask's title, `task <key> asks at <step>` for an
ask without one, or `task <key> <state>` for a task that turned blocked or failed, and nothing else. Tapping it opens the ask or the task, in an
open Priory window when there is one.

### Task row and run row

- **Phone**: rows as on the desk. Line 1 is the key, workflow and step, state and age; line 2 the
  title. Line 3 is lane, band and the batch tag; a `blocked`, `held`, `failed` or `dropped` row
  shows its reason there instead of lane and band, in the role's text color, and keeps the batch tag. A run row's line 2 is its agent and
  model, line 3 its run id, outcome, duration and tokens.
- **The step never truncates.** Line 1 names the step once when it equals the workflow's name,
  "implement" rather than "implement implement", and "implement, approve-pr" otherwise. A long step
  wraps its own words. A row with tags on line 1, such as a runner or Command, wraps the tags and
  the age to a second line before it cuts the step.
- **Wide**: a table, `--row-wide` (38 px) at least, with columns for state, key, title, workflow
  and step, lane, band and age (tasks), or state, run id, key, step, agent and model, outcome,
  duration, tokens and started (runs). A task's reason sits under its title. Numbers are right
  aligned in `tabular-nums`.
- A task that is a batch's entry or planning task carries a `.tag` with the `layers` icon, "batch
  4", that opens the batch's page, `/batches/4`. The row's key is a link too, so a keyboard reaches
  the task; a tap anywhere else on the row opens it.
- A task with a stack of two or more pull requests shows `3 PRs` as a `.tag` after its state, in
  task, asleep and stopped rows. The wide desk table puts it above the row's reason. A task with one
  pull request shows none.
- States: default, pressed (`--raised`), selected (`--action-soft`, with no leading bar), new (fading `--action-soft`), focused (focus ring).
- A live run shows its duration counting up once per minute, not per second. Tapping a run opens
  its task's page, which holds the runs table and the terminal.
- Tokens add up input, output and cache tokens: "640", "1.5k", "312k", "3.3M". The task page's
  trace and the Costs panel use the same format.
- Another runner's task row shows its hold's tile and word in place of the state, and its workflow
  without a step. Its line 3, or the line under its title on wide screens, reads "On runner a, last
  seen 12 min ago" in `--text-3`, or "On runner a, never seen alive". Its key is plain text: its
  page stays on its runner. See [Another runner's task](#another-runners-task) for its actions.

### Another runner's task

A remote task row has no page here, so a tap anywhere on it opens it in place, under its lines, and
a second tap closes it. A `chevron-down` button, `chevron-up` while open, ends the row for the
keyboard, with `aria-expanded`. On wide screens the open part is a row of its own under the task's,
in the title column.

- **Actions** follow the hold, since the state stays on its runner: Resume, Cancel and Release for a
  `kept` task, Cancel only for a `waiting` one, which has no run to resume, release or say to. They
  reuse the task page's `.btn secondary` buttons, and Cancel arms on the first tap. A `kept` task
  adds the Say box. There is no Revive, since an ended task leaves the list, and no Hold.
- **Note**, under them, with the `corner-down-right` icon: "This task runs on laptop. An action goes
  there through the log, and laptop applies it when it reads it."
- **Sent**: the newest submission takes a line of its own above "On runner", never sharing it, so
  "last seen" is never cut. "Sent to laptop: cancel" in `--action-text` while pending; "laptop
  refused resume: task 2 is running, not blocked" in `--warn-text`; "laptop applied release" in
  `--text-3`. A say reads `say “text”`.
- **Not sent**: a 409 says "Not sent: task laptop:ENG-262 is at generation 3, not 2. The list reads
  it again: check the task, then send again."; a 503 says "Could not send: why. Nothing was sent: try again.";
  both under the actions, as the task page's notes. The Say draft stays. A sent action leaves no note:
  it shows as the Sent line once the list reads it again. A Cancel armed at one generation disarms
  when the list shows another.

It adds no token: it reuses `.task-acts`, `.acts-note`, `.writeform`, `--action-text` and
`--warn-text`. The board is in `~/shared/priory-remote-task-actions/board/` on the author's machine.

### Tasks panel

Every task. Saved views: Active (not `done`, `failed` or `dropped`, in start order), Blocked, and
Ended (newest first), each with a count. Facet chips: State, Lane and Workflow, each offering its
values A to Z with counts; a task with no lane shows under `none`. Ended lists the last 7 days first
and loads the older tasks once the person scrolls to its end, with or without a facet set; a set
facet counts the loaded tasks only. The rows take the whole width, with no
drawer: each opens the task's page. In a factory of several runners, Active also lists each task
another runner holds, after this runner's, and the State facet counts them under `elsewhere`. A
Runner facet then offers each runner with its count, as the Asks list's does: this runner reads
"workstation (here)".

### Runs panel

The runs of the last 7 days, newest first. Saved views: Recent and Live. Facet chips: Outcome, Agent and
Step. A run's outcome is both its state from the [State tile](#state-tile) table (`running` while
live) and its result's outcome, so `moved` and `passed` both find it. Recent
loads the older runs once the person scrolls to its end. As on Tasks, the rows take the whole width
and each opens its task's page.
- Actions live on the task page, not in the row: Resume, Cancel, Revive, Hold, Release, Move. Two
  exceptions, since acting on them is why they are there: the desk's stopped rows, and the Queue
  panel's waiting rows, which carry a Move menu.

### Queue panel

What starts next and why. Saved views: Waiting, Held and Last day, each with a count.

- **Waiting** opens with the Up next strip, as on the desk, then one section per band in the
  factory's band order, leaving out empty bands. Each band lists its queued and ready tasks in
  admission order. A row shows its place in the band (`#2`), key, state and age, its title, and why
  it waits in `--text-3`, as a row's third line: the daemon's reason when it has one ("waits: priory is paused"), else
  "Ready: starts when a runner has room.", "Waits behind ENG-240 in band later.", "Waits behind
  band today." or "Waits for its turn: first in band today.".
- **Lane caps**, a strip under Up next, has one line per lane with a `max_running`: the lane in
  mono, then its live runs against the cap in force ("2 of 2 running", in `--warn-text` at the cap,
  or "3 running, no cap now"). Under them, in `--text-3`, the cap and its window: "max_running 2
  mon-fri 08:00-18:00, no cap outside". With no capped lane, the strip is left out.
- Each waiting row ends with a **Move** menu, a `.btn secondary` at `--control-fine` height
  (`--control` on a coarse pointer). It opens a picker like the facet chips': To the top and Up one
  when a task is ahead in the band, Down one when one is behind, then To band X for each of the
  factory's other bands, by name. A tap sends `POST /api/tasks/{id}/move` with the revision the
  page drew, as `priory move --expected` does. A refused move keeps the row and shows why under
  it, with the `circle-x` icon in `--warn-text`; a stale revision says to read again.
- **Held** lists each held task with its hold's reason. **Last day** lists the tasks that ended in
  the last day. Both use task rows that open the task page.
- On wide screens Waiting is a table: place, state, key, title with why it waits, and the Move
  menu, grouped by band.

### Task detail and workflow grid

- The back link names the list the person came from and returns to it, view included: Desk,
  Queue, Tasks or Inbox. A task page opened first goes back to Tasks. The batch page does the same.
- Context line: task key in mono, the task's state tile and word, workflow, lane, band, and
  "batch 4" linking to the batch page when the task is a batch's entry or planning task.
- Title in `--type-task`, then the task's reason in its role's text color.
- **Open asks**, before the actions: one bordered row per open ask of the task, snoozed ones too,
  oldest first. Each shows the ask tile, its title, its age and a primary "Answer…" link to
  `/asks/<id>`, so a person who follows the Desk's "Open task" can still answer.
- The context line ends with "Answers asks itself at **high**" when the task has an `auto_answer`
  level.
- **Answered asks**, after Say: a `--type-body` heading with the count, "3, newest first", then
  one bordered card of rows split by hairlines, newest answer first. Each row has a 16 px icon for
  who answered (`cog` for auto, autopilot or a lead agent, `terminal` for a session, `user` for a person),
  the ask's title, "Answered **40** by a person. Recommended 50." with the recommendation only when
  another option won, and the age at the right. A session reads "by session `7f3c2a`" in mono.
  When the agent stated a confidence, a `--text-3` caption shows it as three bars, filled in
  `--text` up to its level (high 3, medium 2, low 1), then "medium: " and the rationale. An auto
  answer's row opens wider: its title in 600, "**high** confidence, the agent's own. The task
  answers at high and above.", then a `--raised` box of Why, Trade-off and The ask, then a caption
  "The first answer stands. To change course, tell the agent." with underlined links to Say and to
  the ask. There is no overturn control: the daemon refuses a second answer. An autopilot answer's
  row opens the same way, without the task's level, and its box adds Autopilot: why it could answer.
  Its caption reads "Until the agent gets it, priory answer replaces it. After that, tell the
  agent.", or for a plan "To change course, cancel the task, then revive it at the plan's step."
- **Actions**, in `.btn secondary`: Resume and Cancel for a blocked task, Revive for a failed or
  dropped one, Cancel for any other active task. Cancel and Revive arm on the first tap, as a
  command option does: the button turns `--emph`, reads "Tap again to cancel" over its verb, and a
  ring counts 4 s down. The second tap sends. After it, a `--text-2` note says the verb and the
  daemon's sentence: "Cancelled: task 3 dropped."
- **Say**: for an active task, a text box "Say to the agent" and a primary Say button under the
  actions. The draft stays on this device until the daemon takes it.
- **Progress**, when the task's agent or a person published one: the goal, the acceptance
  criteria as rows of id in mono, description and a Required or Optional tag, then the latest
  change and the next action. Each part hides when unset. A **Plan** section follows when the
  task has a `plan` result.
- **Workflow grid**: one cell per step in a row of equal columns with `--radius-md` and hairlines
  between cells. Each cell holds the step name and its decider with an icon: `cog` for model and
  code steps, `user` for human steps. Done steps show a check in `--ok-text`. The current step is
  solid `--emph` with a live dot. Future steps are in `--text-3`. An irreversible step has a
  1.5 px `--danger-text` edge and the `lock` icon.
- **Feedback**, in the pull request section: each review feedback item the agent sorted, as a
  history row with its link and its class in plain words: "trivial", "nothing to do" or "your call".
  On wide screens each check's result and each item's class sit beside its name, in one column,
  not at the row's right edge.
- **Inbox**, after the pull request: the events routed to the task, newest first, at most 20, as
  history rows. Each shows a tile for its prompt (`done` delivered, `queued` still queued, `running`
  being typed, `prompt` unconfirmed, `dropped` withdrawn, `asleep` kept for the next run, `ready`
  otherwise), the type
  in plain words in bold, as in the Inbox panel, and the subject, then what it did under them in `--text-2`, worded as `priory task ID`
  prints it: "kept, still queued" or "woke, delivered as prompt, fired wait 3". Past 20, a note
  says "Latest 20 of 57. `priory task 7` lists every event."
- Then the runs table and the terminal of the selected run. Under each run, one row per scorer
  with the run's latest score: its verdict tile, the scorer in mono, the verdict word in its role's
  text color, and the reason under them in `--text-2`. A run with no score shows no row.
- The full workflow diagram (steps as nodes, outcome edges) keeps the same encodings, top to
  bottom on compact and left to right on wide, with pinch, drag and a "Fit" button.

### Batch page and lane graph

The batch page, `/batches/<id>`, opens from the desk's batch row and from the task page of each of
its tasks. On a phone it sits under the Tasks tab with a back link to the list the person came
from, as the task page's; on wide
screens it fills the list, as the task page does.

- Context line: the batch tile, the planning task's key in mono, which opens the task, the batch's
  state, lane, "went 5 h ago by max" and "plan rev 2".
- Title in `--type-question`, then the **stage bar**: one 8 px bar with `--radius-sm`, a part
  per stage with entries, as wide as its count: merged `--tile-ok`, started `--tile-live`, blocked
  `--tile-warn`, failed `--tile-danger`, waiting, queued and dropped `--tile-muted`, and planned a
  1.5 px `--tile-hold` outline. Under it, the words: each stage's count in bold and its word, in the
  order merged, started, waiting, queued, blocked, planned, failed, dropped, then "2 closed at the
  go". The bar is hidden from screen readers; the words say it.
- Then "8 entries in start order", the key line, the lane graph, then Closes and Splits as in the go
  ask.

The **lane graph** draws a plan, in the batch page and in a go ask.

- One row per entry in start order, with a hairline between rows. Line 1: the issue in mono, a
  `split` tag for a part, then the task as a link and its state word in its role's text color, or,
  before the go, the entry's files in `--text-3`. Line 2: the title. Line 3, in `--text-2`: when it
  starts in words ("starts at the go", "stacks on #1 ENG-40", "starts once #4 ENG-41.1 merges"),
  then the task's reason, or its step and age.
- The node sits in the row's lane: the state tile at 22 px after the go, a numbered 22 px circle on
  `--surface` with a `--control-line` ring before it. Lanes are 18 px apart.
- After the start words, line 3 names the task's latest pull request, such as "#231 merged", when it has one.
- An edge runs down its predecessor's lane and turns into the entry's lane just above the entry. A
  single 2 px `--text-3` line is `merged`; a double line, two `--text-2` rails 2 px apart, is
  `waiting`. Each line has a halo in the panel's background. A key line above the graph names both.
- An entry takes the lane of the first predecessor it is the last to wait on, else the lowest lane
  whose entry has no successor left below. A phone shows at most three lanes; entries past them share
  the third, and their words still say when each starts.

| State | What shows |
| ----- | ---------- |
| loading | "Loading batch 4…" |
| error | "Could not load batch 4: ..." and `priory batch show 4` |

### Panel chrome

- Header, 44 px, sticky: title in the display role, count in `--text-3`, then up to two icon
  buttons (filter, menu). Any further action goes in the menu.
- Body scrolls on its own with `overscroll-behavior: contain`.
- When the event stream drops, a `--warn-text` strip under the header says "Reconnecting. Last
  update 2 min ago." The panel keeps its rows.

### Terminal pane

- Always the `--term-*` tokens, dark in every theme and mode, with `--radius-md`. Mono at 12 / 16
  on compact, 12.5 / 19 on wide.
- tmux sets the width, so lines never wrap. The pane scrolls sideways inside its own box, and its
  scrollbar takes the `--term-*` colours in every theme. A "Fit" toggle scales the text down to fit
  the width, never below 9 px.
- A new screen rewrites only the rows whose text or style changed, so a selection in the other rows
  stays while the agent writes.
- The cursor fills its cell with `--term-fg`, its character in `--term-bg`, without blinking. It
  shows only when the screen carries one, never while earlier rows sit above the screen and never
  on an ended run. Under forced colours it takes a `CanvasText` outline.
- Read-only by default. "Type" opens a one-line input at the bottom, with a "Say" button, that
  sends the line on Enter through the task's say verb: it reaches the agent once its run is idle,
  never as a slash command. The header's tag reads `read-only`, or `say` while the input is open.
  A said line clears and a `--term-green` line under the input says so; a refused line stays in
  the input and a `--term-yellow` line says why. Escape or "Type" again closes it. An ended run
  has no "Type".
- Header: the `terminal` icon, session name in mono, run id, "Type" with the `message-text` icon,
  "Copy screen", "Fit", and a menu (`more`). The menu holds "Copy attach command", which copies
  `priory attach N --read-only`, since a page cannot open a tmux client, and the keyboard rows
  below. Under 480 px of pane width the buttons show their icon only, with their name as a label,
  and under 400 px the run id hides.
- Keys, only when the screen says `keyboard: true` (`[serve] terminal = "type"`). The menu's "Take
  the keyboard" opens a confirm sheet, a modal `dialog` over `--scrim` like Add's: "Take the
  keyboard of run N?", then the line "Your keys reach the agent and its shell directly. Priory's
  guard does not check them." in `--warn-text` with `shield`, then three facts with `clock`,
  `pause` and `pencil`: 15 minutes, the agent's prompts wait, the trace keeps each key and each
  text's length, never the text. "Take the keyboard" is the danger button; "Keep watching" closes.
- While the page holds the keyboard, the pane takes a 2 px `--term-yellow` ring and the tag reads
  `keys` in `--term-yellow`. A bar at the bottom says "Your keys until 14:32" with "Give back". Its
  input pastes text without Enter: Return in it sends the text only, and a note says "Sent 15
  characters. Press ⏎ to submit them." Under it a key row sends one key a tap: Esc and ⌃C first,
  edged in `--term-red`, then ⏎ Enter, Tab, ⇧Tab, the arrows, PgUp, PgDn, ⌃D, y, n and 1 to 9,
  scrolling sideways, each `--control` on a coarse pointer. Each send reads the screen at once.
- "Give back" opens a second sheet that shows the screen's last two rows and says that text typed
  and not submitted stays in the agent's input box: clear it with ⌃C or submit it with ⏎ first.
  "Give back" there is primary. When a grant ends on its own (its 15 minutes, another device took
  it, the run ended), the bar and ring go and a `.note` above the pane says why, with "Take it
  again". The grant's secret lives in the page's memory only; leaving the page or switching runs gives the keyboard back.
- SGR colours draw in the eight `--term-*` colours. Bright colours, the 256 palette and RGB take the
  nearest of the eight. A background is its colour at 30 % over `--term-bg`. Reverse video fills the
  cell with the text's colour, solid, in `--term-bg` text. Bold, faint, italic and underline show;
  any other escape shows as text. An ended run's screen drops its
  colours with the rest of its text to `--term-dim`.
- A `[redacted]` in the screen takes `--term-dim` with a dotted underline, whatever its colour.
- "Load earlier" sits above the screen, a quiet button with the `chevron-up` icon, while tmux holds
  rows above those shown. Each tap reads 500 more, up to 2,000. The earlier rows sit above a dashed
  rule where the screen starts. At 2,000 a `--term-dim` caption replaces the button: "The last 2,000
  lines are as far back as this pane reads."
- Notes about the screen sit above the pane in page tokens, as `.note` lines: `--warn-text` with
  the `shield` icon for "2 secrets hidden on this screen", `--warn-text` with `unplug` for
  "Reconnecting. Screen from 14:02, 40 s ago." when a poll fails, and `--text-2` with the run's
  ended state icon for "Run 1 ended as done. This is the last screen this page saw."
- When the run ends, the pane keeps its last screen, dims its text to `--term-dim` and shows the
  run's state tile.
- The page keeps no screen in local storage, and drops it on sign-out, on a `401` and when the pane
  switches runs.

### Reports and tables

- Numbers right-aligned in `tabular-nums`. The first column stays sticky when the table scrolls
  sideways on a phone.
- Headers in `--type-caption`, `--text-3`, sentence case.

### Reports panel

The web view's `/reports`: the report written last, as the Herdr [Bonjour](#bonjour) panel shows
it. Both use one view model, so sections, picks and threads read the same in both. On a phone it
sits under More.

- **Toolbar**: above the scroll region, older and newer arrows around the report's date, then Next
  pick, which scrolls to the next pick to agree and focuses its first option, and a `search` button.
  An arrow with no report beyond it is disabled. Search opens a field under the toolbar; its text
  keeps the rows whose key, text, meta, owner or comments hold it, with "3 found" beside it, and
  "Nothing in this report holds “x”." when none does.
- **Header**: the greeting in the display role, then one `--text-2` line with the date, a link to
  the task that wrote it and "Since 18:00. 2 picks to agree.", then a 6 px `--action` progress bar
  on `--raised` with "1 of 4 agreed". A report with no pick shows no bar.
- **Sections** in the report's own order, each a `.sect` with its title and its count. Section
  chips sit under the header and stick to the top. From 1024 px they become a 200 px list on the
  left, sticky, with each section's count. The lit chip is the last section whose top has passed
  under the chip bar, or the last section once the list is scrolled to its end. On a phone the
  chip strip scrolls sideways to keep the lit chip in view.
- **Row**: its tile, its key in mono, its text, then a caption line with its meta, owner and time.
  The text draws as [Markdown](#markdown), so a link shows its label with its host after it, and
  the key takes the factory's autolinks. The meta stops at 2 lines within `--measure`, with its
  full text in its `title`. With 720 px of panel width, a pick's control sits right of the text
  and meta, its Picking line under it; narrower, it follows the meta.
- **Pick**: a segmented control on `--raised`. A `check` means chosen, so no option shows one
  before a choice. The recommended option has a dotted underline, and its label adds
  "recommended" for a screen reader; an open pick has no note. A tap turns the option solid `--emph`
  with a `check`, and the caption under the control says `Picking "Take"` with a ghost Undo for
  4 s. Nothing is sent before the 4 s end. Each row's pick has its own caption and Undo, so the
  other rows stay open to pick meanwhile. An agreed pick folds to one line:
  a `check` in `--ok-text`, "Picked Take." and what the workflow did with it. Until the workflow
  applies it, a ghost Change button ends the line: it reopens the control with the choice solid,
  "Pick another option to change it." and Keep Take, and a new option goes through the same Undo.
- **Section chips**: a section with a pick has a row of chips under its title, styled as saved
  views: All, Open and Picked with their counts, then "Recommends" in `--text-3` and one chip per
  recommended option, None last. A pressed chip is solid `--emph`. The row ends with a primary
  "Pick all recommended" button and its count, disabled at 0 or while the section's batch waits.
  While it waits, a `--raised` strip under the chips says `Picking the recommended option on 9 rows
  in Sentry` with a secondary "Undo all 9"; each row in it says "With the batch" under its control.
- **Threads** follow the Bonjour rules, with Reply and Resolve as ghost buttons in the card. A
  row's threads fold behind a disclosure, `message-text` and "2 comments", which opens them with
  the row's Comment button after them; an open comment field keeps it open. A row with an id and
  no thread ends with a Comment ghost button, on the newest report only: an older one takes replies. It opens the comment field under the row, with
  Cancel and Comment. Reply opens the same field, with "Replying in thread 3" above it.

| State | What shows |
| ----- | ---------- |
| empty | "No report yet. A workflow writes one with the `report` tool or `priory report put`." |
| loading | The frame's: the report loads with the desk's listings, so it shows once they do |
| offline | The stale strip. Picks and comment buttons are disabled, and each open pick says "Reconnect to pick." |
| picking | The chosen option solid, `Picking "Take"` with Undo under it for 4 s, then `Sending "Take"…` while it sends. Other rows stay pickable |
| refused pick | The report redraws, and the row says why in `--warn-text`, naming the option: "Take: Already picked Later." or "Take: The report changed to revision 5. Read it and pick again." |
| refused comment | The row says why in `--warn-text`, and the field keeps its text |

### Flows panel

The web view's `/workflow`, named Flows: each workflow of the factory drawn as a timeline, from
`GET /api/flows`. It reads the factory once, so it needs no refresh; the history is as of that read. On a phone it sits under More.

- **Key**: a `.sect` under the header with the six node shapes, the four line styles and the
  highlight, in `--type-caption` and `--text-2`, so no reader has to guess a mark.
- **One `.sect` per workflow**: its name, then "7 steps" in `--text-3`, then an inline SVG that
  scales to the panel's width, at most 560 px wide. The step the workflow starts at is the top row.
  Each step sits one row below the latest step that leads to it, and `done` and `stop` close the
  last row. A row holds at most two steps, so a wide branch wraps to the next row and the labels stay
  readable on a phone. No chart or graph library draws it.
- **Node shape is the step's kind, and never color alone**: a pill for `read`, a rectangle for a
  `reversible` write, an octagon for an `irreversible` write, a hexagon for an `ask` (a human step),
  a parallelogram for an `answer` (a human step that approves a gate) and a diamond for a `verdict`
  (a step with effect `none` that only decides). Terminals are `--raised` pills. Step id in mono at
  12 px, its kind in a `--text-3` line at 10 px. `ask` and `answer` fill `--action-soft`, `verdict`
  `--raised`, the rest `--surface`, all with a 1.5 px `--control-line` edge.
- **An irreversible step** has a 2.5 px `--danger-text` edge and a `lock`. Its second line is the
  gate in `--danger-text`: "approved by approve-pr", or "autonomous:" and its reason, clipped to 24
  characters. The approvers are the human steps that approve it and lead to it, as `priory check
  --explain` lists them. A step with no gate reads "no approval".
- **Edge line style is who decides**, in dash and weight as well as color: `code` thin and dashed
  `6 2` in `--text-3`, `model` dashed `9 5` in `--action-text`, `human` solid and heavy (3.5 px) in
  `--emph`, `asleep` dotted in `--text-3`. Each ends in an arrowhead, and carries its outcome in 11
  px `--text-2` over a `--bg` halo. An edge that skips a row, or loops back, runs down a lane right
  of the nodes with its label turned sideways, so it never crosses a step. So does an outcome that
  shares both ends with another, so each keeps its own line and label. Lanes sit 20 px apart, and
  a turned label sits right of its own line, clear of the next lane, at the middle of the longest
  stretch that no outer lane's bend crosses. A slanted edge's label sits on the side the edge
  heads to, where its own curve is not.
- **Ungated paths**: each edge on a path from the start to an irreversible step that follows no
  `human` edge, the paths `priory check --explain` lists, has a 6 px `--danger-text` halo at 22 %.
  Under the diagram, "Reaches an irreversible step with no person's decision:" lists each path in mono
  in `--danger-text`.
- **History**: when `GET /api/flows` carries a `risk_estimate`, a `.ctbl` table follows the
  diagram, under "Tasks through each irreversible step in the last 180 days, judged within 30 days
  of it." in `--type-caption` and `--text-2`. One row per irreversible step: Step in mono, Caught,
  Judged wrong and Landed wrong. Each outcome cell holds its count, such as "17 of 53", over "95 %
  bound 43.4 %" or "needs 20" in caption `--text-3`. Under 600 px of panel width the table stacks:
  the step and "Caught 4" on one line, then "Judged wrong 17 of 53, 95 % bound 43.4 %" and "Landed
  wrong 3 of 53, 95 % bound 13.7 %" as two caption lines, so Landed wrong never starts off screen.
  With no state, the table is left out.
- **Text alternative**: the SVG has `role="img"` and an `aria-label` that reads each step, its kind
  and where each outcome leads, decided by whom. Each node and edge has a `title`. A "Steps as
  text" disclosure, 44 px tall, lists the same facts as an ordered list.

| State | What shows |
| ----- | ---------- |
| loading | "Loading the flows…" |
| error | "The flows did not load: ..." with `priory check --explain` as the fallback, in `role="alert"`, and a secondary Retry button |
| empty | "This factory has no workflows. Add one under `workflows/`." |

### Costs panel

The web view's `/costs`: what the factory spent this week against last week (`/api/costs`), and
what it cost the person (`/api/attention`). On a phone it sits under More, with two saved views,
Costs and Attention (`/costs?view=attention`). From 1024 px both show side by side, Costs on the
left, with a hairline between them.

- **Stat tiles**: a row of equal cells in one `--radius-md` box with hairlines between them, each a
  `--text-3` caption label, a value in the display role at `--type-option`, and a caption line.
  A comparison's line is a `trending-up` or `trending-down` icon and "-31% vs last week", in
  `--text-2`: more cost is not good or bad by itself, so no state color. With no previous value it
  reads "No change to show".
- **This week**: tiles for cost, merges and cost per merge. A cost with no priced token reads `-`,
  never `$0`. Unpriced tokens and records without tokens follow in a note: "Not in the cost." with
  each model and its tokens, and where to price them.
- **Each day**: an inline SVG bar chart, one bar per day of both weeks, in `--text-2` this week
  and `--tile-muted` last week. A dotted `--text-3` line labelled "this week" separates the weeks,
  so the split never rests on color. The busiest day's cost labels the top. Under each bar sit its
  date, with the weekday from 520 px, and its merges. Each bar has a `title` with its sentence, and
  the chart's accessible name lists every sentence.
- **Split**: "This week by workflow" with a Split by `select` (workflow, agent, model, machine,
  repo, lane), then a table of the value in mono, cost, tokens and notes. The first column sticks when
  the table scrolls sideways.
- **Scores**: under the split, when the window has scores, "Scores by workflow" with the caption
  "The pass rate leaves out errors and not applicable." Then a table of the value and the scorer in
  mono, the pass rate (`pass` over `pass` and `fail`, `-` with neither), and the count of each
  verdict. Below 600 px each row stacks: the value and scorer on line 1, then a `--text-2` caption,
  "pass rate 79%, 11 pass, 3 fail, 1 error", which names only the verdicts it counted.
- **Attention headline**: the person's minutes per merged pull request in `--type-count` and
  `--text`, never `--count`, which belongs to asks; beside it "minutes of your attention per
  merge" in the display role and a caption with the minutes, merges and acts.
- **Ask waits**: tiles for the median and p90 wait, each with its time "while you were present",
  and the asks acted on with those still open.
- **Where each task's time went**: one row per task, its key in mono linking to the task, its
  workflow and its wall time, then a 12 px SVG bar split by a 2 px `--bg` line into working
  (`--tile-live`), waiting on you (stripes of `--wait`), waiting on the world (a dashed `--text-3`
  outline) and queued (`--raised`), the trace waterfall's encodings. A legend names the four
  parts, each part has a `title`, and the bar's accessible name is the whole split as a sentence.

| State | What shows |
| ----- | ---------- |
| empty | Costs: "No cost in the last two weeks. Costs appear once a model step records its tokens. Price each model in `prices.toml` in the factory folder." Attention: "No attention counted this week. Your answers, dismissals and snoozes on asks count here, and each task's time once it runs." |
| loading | Nothing for 300 ms, then three skeleton blocks on `--raised` |
| offline | The stale strip over the last costs read, which the device keeps, under "Costs as read Nov 21, 09:14." The Split by `select` is disabled |
| error | A `--warn-text` line above the last costs read: "Could not load costs: why. Check that the daemon runs on its runner, then retry." with Retry, then "Costs as read Nov 21, 09:14." |

Opening the panel reads both reports again when the last read is over a minute old. While a
read over a minute old shows, "Costs as read" says when it was read. A comparison
with no change shows its line without an icon.

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
- **Loading**: until the first listing arrives, the frame says "Loading the desk…", or shows the
  listing this device kept. A page then shows one
  sentence in the empty state's place, with the one verb "Loading" and what loads: "Loading task
  7…", "Loading the flows…". A list that loads its older rows ends with "Loading older runs…". The
  terminal says "Loading the screen…", and Herdr's panels say the same as the web. The ask's plan
  says "Reading the plan of revision 2 before you answer.", since it gates the answer. The Costs
  panel alone shows nothing for 300 ms, then three skeleton blocks on `--raised`. Never a
  full-page spinner. A list that has data keeps it while it refreshes.
- **Error**: where it failed, why, and how to fix it, with a "Retry" button. "Could not load runs:
  the daemon refused the connection. Check that the daemon runs on its runner, then retry." Keep
  any stale data visible under the error. A task that cannot be read again keeps its last read under
  a `.note`: "Could not read task 7 again: why. This is its last read." A listing that no answer depends
  on says the same in the `.stale` strip, with Retry, and the connection stays "live". A page that fails to draw
  keeps the frame, says "Could not draw this page" and why, and has "Retry" and "Open the Desk".
- **Not found**: the title "Page not found", the sentence "No page lives at this address." and an
  "Open the Desk" button in `.btn secondary`.

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
- A link drawn smaller than that, such as an ask's or batch's context line, a task key on the batch
  page or another runner's chevron, keeps its look and takes a transparent hit area of `--control`
  height on coarse pointers, where no other link sits under it. A row's task key needs none: the
  whole row opens the same task. A chevron button is 24 by 24 px at least everywhere.
- Put the primary action of a screen in its bottom third on compact screens.
- Every gesture has a visible button that does the same thing.
- Allowed gestures: horizontal swipe between asks in an ask's detail and between tabs; pinch and
  drag on diagrams; the browser's own back swipe. Nothing else.
- No pull to refresh: panels update live. Set `overscroll-behavior: contain` on scrolling panels.
- No long press, no hover-only controls, no double tap except the second tap of an ask's option, a
  Cancel or a Revive.
- Respect the safe-area insets on all four edges.

## Accessibility

- Meet WCAG 2.2 AA in every theme and mode. The tokens already meet the contrast part; keep it so.
- Focus ring: 2 px solid `--action-text` with a 2 px offset, on every focusable element, using
  `:focus-visible`.
- Every control works by keyboard. Desktop shortcuts: `j` and `k` move, `Enter` opens, `1` to
  `9` pick an option of the open ask (twice for a command), `n` adds a task, `Esc` closes. `s`
  snoozes the open ask for 1 h when it is open, not snoozed, belongs to a task on this runner, and
  the page is online with no answer sending. Single-key shortcuts never fire inside an input or an
  open sheet, and a setting turns them off. That setting is not built yet (see Open questions).
- The sheet over `--scrim` is a modal `dialog`: focus moves into it, stays there, and returns to
  the answer bar when it closes. The section chips are a `nav` of in-page links.
- Each panel is a `region` labelled by its title. The tab bar is a `nav`.
- Each page has one `h1`: the panel's title, or a detail page's question or title. The phone
  Desk's `h1` is visually hidden. In a drawer, the ask's question is an `h2` under the list's `h1`.
- The document title names the page, then Priory: "ENG-224 · Priory", "Ask #3 · Priory",
  "Queue · Priory".
- A new ask is announced once through a polite live region: "New ask from task 61." Never
  assertive.
- A state tile has the state word as its accessible name when the word is not shown beside it.
  Decorative icons are `aria-hidden`.
- The terminal is a `pre` with an accessible name ("Terminal of run 18") and a "Copy screen"
  button, so its text can be read elsewhere.
- Text reflows at 320 px wide and at 200 % zoom. Only the terminal, diagrams and wide tables
  scroll in two directions.
- Under `prefers-contrast: more`, `--line` takes the value of `--control-line`.
- Under `forced-colors: active`, cards and option buttons keep a 1 px `CanvasText` edge.

## Copy

- Use the terms of the PRD's Concepts table: task, run, ask, desk, inbox, queue, wait, result,
  lane, band, runner. Never "job", "ticket", "worker" or "prompt" for these.
- Show state names exactly as the daemon writes them, in lowercase. A pull request's review and
  check states are GitHub's, not the daemon's: they read in plain words ("changes requested",
  "passed").
- Name a task by its key without the source prefix: "ENG-245", not "linear:ENG-245". The numeric
  id never stands alone; it goes in the `title`.
- Name actions with the CLI verb: Answer, Resume, Cancel, Revive, Hold, Release, Move, Attach.
  Confirmations reuse the verb: "Answered", "Resumed".
- Errors say what failed, why, and what to do. No apologies, no "Oops", no exclamation marks.
- Times: "just now" under a minute, relative under 24 h ("12 min", "3 h"), then a date ("Nov
  18, 14:02"). Since you looked, a task's inbox and its trace follow it too. The full
  timestamp is in the element's `title` and in `<time datetime>`.
- Sentence case for everything. No uppercase labels.
- Plain English, short sentences, active voice, no em dashes.
- Agent text appears as the agent wrote it, as text. Links in it show their host.
- A notification carries one line a stranger could read on the lock screen: the ask's title, or a
  task's key and state. Never a brief, plan, option, command or answer.

## Herdr adapter

Priory's panels also ship as Herdr mods (R8.6). A mod renders natively with gpui-kit, in a pane
beside other panes, with a fine pointer. It keeps this file's structure, tokens, state tiles, copy
and ask rules. This section lists only what differs, and the panels the adapter ships: Desk, Asks,
Queue, Bonjour, Costs, Flows, Task, Inbox, Tasks and Runs, and the usage line. Bonjour is also the web view's Reports panel. [`docs/adapters/herdr.md`](docs/adapters/herdr.md) explains where
the adapter lives and how it reads and acts, and [`docs/reference/herdr-panels.md`](docs/reference/herdr-panels.md) lists the data each panel needs. The approved boards are in
`~/shared/priory-round-6/` on the author's machine.

### What differs from the web view

- **Density.** A pane takes the `--pane-*` tokens from
  [`design/tokens/structure.css`](design/tokens/structure.css): rows of at least `--pane-row`
  (52 px) with a `--pane-tile` (24 px) tile, views and chips `--pane-chip` (28 px) tall, options
  `--pane-option` (40 px) tall, the question in `--type-pane-question` (18 / 24), the desk count in
  `--type-pane-count` (40 / 1), and buttons at `--control-fine`.
- **Width.** An aside opens at 360 px and narrows to 280 px at least. Every panel should work at
  `--pane-narrow` (420 px), which is wider than both: see the open question on Herdr asides. Below
  `--pane-split` (720 px) a selected row opens its detail in place, under the row. From
  `--pane-split`, the list and the detail sit side by side.
- **Selection.** A selected row takes the `--action-soft` fill on its own rounded surface. It has
  no 3 px leading bar: gpui-kit's design guide forbids leading-edge selection bars.
- **Placement.** Each view gets its own layout, and the default layout follows the `default`
  view. Narrow panels open as asides and wide ones as tabs in the center, and the Priory bar shows
  one button per aside and per center tab. When the view files change, the bar posts one notice,
  "The view files changed", whose click regenerates the layouts. The Task panel and an Asks panel the view
  lacks open as tabs in the center.
- **Navigation.** Herdr owns tabs and panes, so a panel has no tab bar, rail or Answer capsule.
  `Enter` opens the selected row.
- **Add.** The Desk and Queue panels open [Add](#add) as a card under their header, not as a
  sheet: the field with Add and close buttons, then a dropdown each for workflow, band
  and lane, with a line that says what happened. Each dropdown is the facet chips' popover, at most
  320 px tall and wide, and its first row clears the pick. The focus stays on the list and the arrow
  keys move its highlight on `--raised`. Names truncate, at 160 px in the closed dropdown. `N` opens it.
- **Overlays.** The answer sheet becomes a popover from the answer bar's "3 options". Secondary
  commands sit in a menu, opened by a `more-horizontal` button, and in a context menu. No command
  appears only on hover.
- **Stacks.** A Desk or Queue row shows `3 PRs` in `--text-2` after its state, and the wide Desk
  table under the row's text. The Asks PR section draws each stack row's state in words, with no
  tile.
- **The armed option** counts its 4 s down with a bar that drains under its label, instead of a
  ring.
- **Pointer.** Buttons keep the arrow cursor. Only links that leave Priory take the pointing hand.
- **Mode.** A mod draws with Priory's tokens, not Herdr's theme. The adapter resolves each
  `light-dark()` pair from Herdr's appearance.
- **Theme.** A panel draws the theme that `priory.theme` in Herdr's `settings.json` names, and
  Graphite when it names none. Its state tiles are stroke icons with no fill, as in the web view,
  drawn by `stateTile` in `adapters/herdr/lib/ui.js`.
- **Fonts.** gpui-kit loads no font file: a mod names a family, and the machine must have it
  installed. Without it, Herdr falls back to its own UI font. Install Instrument Sans on the
  machine that runs Herdr to see Graphite's type.
- **Markdown.** Herdr's `TextView` drops `<br>`, so `markdownHtml` in `lib/priory.js` draws each
  line of a paragraph as its own paragraph, a check list's boxes as characters, and a link
  without its host. The Bonjour panel draws a row's text, meta and comments with it.

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

The calm count, then the one next thing, then the explorer list.

1. **Count** in `--type-pane-count`, as the web desk's: "2 asks need you. Oldest waits 42 min. 1
   stopped. 2 snoozed. 3 events reached no task in 24 h. 2 events no agent read. 1 request a script
   dropped in 24 h." Each of the last three sentences shows only when its count is above zero. The Unrouted view is the [Inbox](#inbox-1) panel.
2. **The next thing**: a card on `--surface` with the desk's `next`: an alarm or the ask that ranks
   first, why it ranks first, its recommendation, and the buttons Answer… (primary, for an ask),
   Open task, Attach and Snooze 1 h. Answer… opens the
   ask in the Asks panel, so a person answers in one place. Open task opens the [Task](#task)
   panel; Attach opens the live run's terminal.
3. **Saved views**: Needs me, Running, Asleep, Snoozed and All.
4. The pause, room, intake and spend cap banners, then Asks, Stopped, To-dos and Later, as on the
   web desk. The memories row's detail
   lists each candidate as on the web's [memory candidates](#memory-candidates) page, with a Current
   text button instead of the disclosure, and an armed Accept that reads "Press again to accept".
   A routed ask shows the
   [routed mark](#routed-ask) under its question, and in the next thing's card. A selected
   to-do's detail offers Done.
5. **Since you looked**, from 1024 px: the events since the person's last visit.

From 1024 px the count, next thing, banner and Since you looked form a 440 px column; the rest is
the wide table with Recommends and the key hint bar (`J` `K`, `Enter`, `S`, `R`, `?`).

### Asks

Every ask, with the web's ask detail fitted to the pane.

- Saved views: Open, Snoozed, Answered and All. Facet chips: Command, Lane, Band, Risk and Task,
  and Runner when the asks name runners.
- [Another runner's ask](#another-runners-ask) shows its runner tag, "last seen", Sent or refused
  lines and the note as on the web. It offers no snooze, Open task or Attach to task. The Desk
  shows the same tag and lines, and offers no snooze on it.
- A routed ask shows the [routed mark](#routed-ask) as a tag in its row and as a line under the
  question in its detail. An answered row and the history say who answered.
- The detail keeps the web's order: context line, History line, heading, recommendation with what happens next,
  options with keys `1` to `9`, write an answer, then the section chips Why, Brief, Plan, PR,
  Trace and History. An ask that approves a plan leads with Plan, and waits for it as on the web.
- Once the options scroll out of view, the sticky answer bar shows "3 options", the recommended
  option and the overflow menu.
- On an ask that belongs to a task, the overflow menu offers Open task, which opens the
  [Task](#task) panel, and Attach to task.

### Queue

What runs next and why.

- **Up next**: a strip with the next 3 runs, queued or scheduled. Each line shows when ("now, on
  room", "about 11:50", "08:30 Fri"), the task or schedule, and why. It opens the task.
- Then Running, the bands in order (Blocker, Now, Today, Later, or the factory's names), Held with
  each hold's reason, and Last day, folded.
- Each waiting row shows its position in its band and why it has not started, in its role's text
  color. A task with its own wait reason, such as "Waits for its claim to reach the log.", says
  that reason, in its row and in Up next, where "on room" is left out. A grip in a fixed trailing lane, and `Shift J` and `Shift K`, reorder a band.
- An opened row offers Move…, Hold… and Cancel…, and Open task.

### Bonjour

The morning dossier.

- A toolbar with the date and older and newer arrows, "n of m agreed" with a progress bar, Next
  pick and search.
- The greeting in the display role, then the report's sections **in the order the factory gives
  them**. The panel knows no section by name. Each section has a title, an optional count and
  rows; a row may carry a pick.
- In Herdr a section draws its first 15 rows, then a secondary "Show 140 more rows" button that
  draws the rest. Past the 15, the selected row, a row being picked, a row with a notice and a row
  with threads still show. A search with text and the Picks view draw every row.
- A pick row is one block on the panel's background, `--action-soft` when selected: its tile, key
  and text on one line, its meta on one truncated line, full when selected, and its control on the
  right, wrapping under the text in a narrow pane. A `check` marks the chosen option only; the
  recommended option is underlined, as GPUI draws no dotted line. An agreed pick folds to one line
  with Change. A pick row's threads show only while it is selected; until then its Comment button
  reads "2 comments" and selects it.
- A section with a pick draws the web's section chips and Pick all recommended as `--type-caption`
  chips, and its batch strip on `--raised` with Undo all.
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

### Costs

The web's [Costs panel](#costs-panel).

- From 1024 px Costs and Attention sit side by side. Below, Costs and Attention chips in the
  header pick one, as saved views do.
- The Split by `select` becomes a row of chips, `workflow` to `lane`, beside the split's title.
- The day chart and each task's bar are drawn with elements, not SVG. The week line is a dashed
  `--text-3` edge before this week's first bar. Waiting on you keeps its `--wait` stripes.
- Loading reads "Loading costs…", with no skeleton.
- Scores follow the split as the web's do. From 600 px they are rows of columns, the scorer in
  mono and the pass rate in weight 600. Below, each row stacks, as on the web. With no cost, the
  split chips sit beside the scores' title.

### Usage line

One 30 px line docked at the bottom of every layout, right of Herdr's status line, on `--bar` with a
`--line` top edge, in UI text at 12 px, as Herdr's own status line. The approved board is
`~/shared/priory-usage/board.png` on the author's machine.

- **Line.** "Priory" in weight 600, then "$12.40 today of $40", "$171.20 this week of $200",
  "2 held at a cap" and "Claude 7d 43%", split by `--text-3` dots. Without a factory cap, a spend
  stands alone; with no held task or no Over9k file, its part is left out. A figure takes
  `--warn-text` at 80% of its cap or limit and `--danger-text` once reached, else `--text-2`.
- **Narrow.** Below 520 px the line drops cents and Claude's figure: "$12 of $40 today",
  "$171 of $200 wk", "2 held". It never cuts a number.
- **Detail.** A click opens a 380 px popup on `--surface`: Spend caps, each with its label, its
  spend against the cap and a 6 px bar on `--raised`, filled `--text-2`, `--tile-warn` from 80%
  and `--tile-danger` once reached, with "Reached: new runs wait for the next day" in
  `--danger-text`; then "This week by workflow" with each workflow in mono and its cost; then
  "Held at a cap", each task's key in mono `--action-text` and the cap it reached, which opens the
  task; then Claude's 7-day limit as a bar with when it resets. Hairlines separate the sections.

### Flows

The web's [Flows panel](#flows-panel).

- Nodes and edges paint with gpui-kit paths in the web's shapes, dashes, weights and colors, scaled
  to the pane, at most 560 px wide.
- gpui-kit turns no text, so a lane's outcome sits upright beside its lane, clipped to 10
  characters, and the drawing keeps 76 px on its right for it.
- "Steps as text" is a button that shows or hides the list under the drawing. "Read again" in the
  header reads the flows again.
- The history always takes the web's stacked layout, since gpui-kit draws no table: the step and
  "Caught 4", then the Judged wrong and Landed wrong lines in caption `--text-2`, each step under a
  hairline.

### Task

The web's [task detail](#task-detail-and-workflow-grid), cut to what a pane reads, as a tab in the
center stack. Open task on the Desk, in Queue and in the Asks overflow menu, and an Up next line,
open it.

- A 44 px header with the task's tile at 18 px, its key in mono, its state word in its role's text
  color, and Attach as a secondary button at the right.
- The title in `--type-pane-question`, then the workflow, step, lane and band in `--text-2`, then
  the reason in its role's text color.
- Asks lists open asks first, then answered ones newest first. An answered ask's 18 px tile is
  its answerer's icon in `--text-2`, as on the web's Answered asks, with the age at the right and
  "Answered after by session 7f3c2a, medium" as its caption. An auto answer adds its three
  confidence bars and the word, its rationale in a bordered `--surface` box, and "The first answer
  stands. Say to the agent to change course." An autopilot answer does too, with "Until the agent
  gets it, priory answer replaces it. After that, say to the agent.", or for a plan "To change
  course, cancel the task, then revive it at the plan's step."
- Asks, Pull request, Inbox and History, each a caption heading with its count, then rows with an
  18 px tile and a `--type-small` line over a `--text-3` caption, split by hairlines. Inbox lists
  the latest 20 events routed to the task, newest first, with the web's tiles and wording and the
  time in the caption: "kept, still queued · 11:50". History lists the last 10 runs, newest first.
- With no task opened: "Open a task from the Desk to see it here." While it reads: "Loading task
  7…"
- Under each History run, one row per scorer with the run's latest score: its 18 px verdict tile,
  the scorer in mono, the verdict word in its role's text color, and the reason in a `--text-2`
  caption, under a hairline.
- The actions, Say, Progress and the workflow grid stay on the web page for now.

### Inbox

The web's [Inbox](#inbox) panel, as a tab in the center stack.

- A 44 px header with "Inbox" and the Unrouted chip with its count, selected, since it is the one
  saved view.
- Each group is a `--surface` card: the resource in mono and its count, then a button with its
  task's 24 px tile, "task 12", its state and key, which opens the [Task](#task) panel. Then its
  5 newest events, each its type in weight 600, subject and time on one line, and its first line in
  `--text-2` under them.
- The route button sits at the end of the card. Revive and Request a task act on the first press,
  as the Desk's buttons do: Request a task adds the task with the group's text and its task's
  workflow, with no Add card, since that text names the resource the new task must own.
- The verb's sentence shows under the button in `--text-2`; a refusal says why. Subscribe carries
  the revision the panel read, as the web's does.
- Unheard cards come first under an "Unheard" heading, then "Unrouted" over the owned cards, as on
  the web. An asleep task's card shows its `follow_up` command in mono `--text-2` instead of Say,
  which stays on the web. A blocked task's card has Resume task 15 and "It reads them when it
  resumes."
- Dropped cards come after the unheard ones under a "Dropped" heading, as on the web, with no
  button.
- While it reads: "Loading the unrouted events…"

### Tasks

The web's [Tasks panel](#tasks-panel), as a tab in the center stack.

- Under the 44 px header, the saved views Active, Blocked and Ended as chips with their counts
  after the facets, then a State, Lane and Workflow chip each, whose menu lists every task's
  values A to Z with counts, as on the web.
- Every row is the desk's task row at every width: tile, key in mono, workflow, step, the state
  word in its role's text color and the age in `--text-3`; the title in `--type-small`; then lane
  and band in `--text-3`, or the reason in its role's text color, then `batch N`. A row opens the
  [Task](#task) panel.
- The batch tag is text: Herdr has no batch panel yet.
- Another runner's task row shows its hold's tile, the `server` icon on `--tile-muted`, and word in
  place of the state, and its workflow without a step. Its last line reads "On runner a, last seen
  12 min ago" in `--text-3`. Its page stays on its runner, so pressing it opens it in place, under
  the row, at every width: Resume, Cancel and Release (Cancel only for `waiting`) as `.btn
  secondary` buttons at `--control-fine`, Cancel armed by the first press with the draining bar,
  then the web's note. Its newest submission takes a caption line above "On runner", in the web's
  colors, and a failed send shows under the buttons in `--warn-text`. Say stays on the web, as for
  the Task panel.
- In a factory of several runners a Runner chip follows Workflow, and this runner reads
  "workstation (here)", as on the web.
- While it reads: "Loading the tasks…"

### Runs

The web's [Runs panel](#runs-panel), as a tab in the center stack.

- Under the 44 px header, the saved views Recent and Live as chips with their counts after the
  facets, then an Outcome, Agent and Step chip each, as in Tasks.
- Every row is a run row at every width: the run state's tile, the task key in mono, step, the
  state word in its role's text color and how long ago it started in `--text-3`; the agent and
  model in `--type-small`; then "run 92 · passed · 42 min · 312k tokens" in `--text-3`. A row
  opens its task in the [Task](#task) panel.
- Recent ends with a "Show 3 older runs" button in `--action-text` while runs older than 7 days
  match: the pane already holds them, so it shows them on a press instead of on scroll.
- While it reads: "Loading the runs…"

### Open questions

- **The same ask three times.** In the full layout the oldest ask shows as the Desk's next thing,
  as the Desk's first row and in the Asks panel. Max will tweak this next week.
- **Not built in the web view.** These were drawn and are not on `main`: the wide top bar's
  command bar (`Ctrl K`) and runner name, the phone top bar's filter and search buttons, the query
  bar, the shortcuts sheet (`?`), and the `w` and `[` `]` keys. Build
  each with its rule here, or leave it out. The setting that turns single-key shortcuts off is
  missing too; WCAG 2.2 (2.1.4, Character Key Shortcuts) needs it, so build it.
- **Herdr asides.** An aside is a bare column, while the frame's main pane and the `cockpit-fleet`
  panels take a card: ground, 1 px border, 10 px radius and a small shadow. Decide whether asides
  take the same card. Asides open at 360 px with a 280 px floor, under `--pane-narrow` (420 px),
  and `cockpit-fleet` holds three of them at 320 px: decide `--pane-narrow` at 320 or 420 px.

## Docs site

The documentation website, built by mdBook from `site/`, uses the Graphite theme, light and dark,
and this file's type, spacing, radii and copy rules. Its board is the docs home and how-to page on
the [Graphite board](https://claude.ai/artifact/PJWYYqCiEBRFnpVBE3HYRY). It reads on its own, so
it differs from the view:

- **Tokens.** `site/build.py` stages `design/tokens/` beside the pages, and the theme links both
  files. `site/theme/site.css` adds only the docs' own sizes and copies no color.
- **Type.** Body text is 16 / 26 within `--measure`, except beside a wide table. On wide screens,
  the title is 36 / 42 and h2 22 / 28, at `--display-weight` and normal width; h3 is 17 / 24 at
  600. The paragraph under the title is the lede, 17 / 27 in `--text-2`.
- **Layout.** A 52 px top bar with the lockup, the section tabs, search and a light or dark
  toggle. From 1200 px, a 264 px navigation on the left, the page at most `--measure` wide, and a
  216 px "On this page" list on the right. From 900 px, the list folds under the title. Below
  900 px, the navigation and the search open over the page, and the tabs hide: the navigation's
  groups carry the same sections.
- **Section tabs.** Tutorial, How-to guides, Reference and Explanation, one per Diátaxis type, in
  the order `GROUPS` in `site/build.py` lists the docs. Each links to its first page; Explanation
  holds every group after Reference. The current tab is in `--text` with a 2 px `--emph` rule
  under it.
- **Copy as Markdown.** A 28 px `--line` button right of "Edit on GitHub" copies the page's source
  Markdown. `site/build.py` writes each page's source beside it as `<page>.md.txt`, so the site
  needs no server. It shows only once the script can reach the clipboard, and it is 44 px tall
  below 900 px.
- **Navigation.** One group per docs folder, its head a caption in `--text`. The current page sits
  on `--raised`. In "On this page", a hairline runs beside the list and the current heading takes a
  2 px `--wait` rule.
- **Reference tables.** They scroll in their own box with the key column sticky. Below 900 px, each
  row stacks into one block: key, type and "yes" when required, then meaning, default and example.
- **Code** follows the mode: `--raised` with a `--line` edge and `--radius-card`, and a copy button.
  Callouts are a `--line` box on `--surface`, with no tinted fill.
- **Diagrams** are images, so each SVG keeps a copy of the Graphite colors.
  `python3 site/build.py diagrams` rewrites them, and a test fails when they drift. A person's
  step is outlined in `--wait`.

## When to change this file

Change `DESIGN.md` and the token files in the same pull request as the code that needs the
change, and update the artifact to match. A new component, state, token or theme starts here, not
in the code. A new theme defines every token in `themes.css`, passes the contrast checks above in
both modes, and adds a card to the Appearance screen.

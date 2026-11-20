# DESIGN.md

The design system for Priory's web view (`docs/prd.md`, R8). Follow it for every panel, view and
component. The live reference is the
[Priory design system artifact](https://claude.ai/artifact/3YYj2s5hmKksarCCC3VgHb): tokens, the
main components and their states in both themes. When the artifact and this file disagree, this
file wins; fix the artifact.

The M2 test is "one working day of asks answered from a phone only". Every decision below serves
that first: a person on a phone, often one-handed, reads an ask and answers it correctly.

## Principles

1. **The desk comes first.** What waits on a person (asks, blocked tasks, uncertain prompts, room
   banners) is the default view and the first tab. Everything else is context for an answer.
2. **Calm by default, loud only when a person must act.** Neutral surfaces, one accent, and color
   spent on state. A screen with nothing to do looks quiet.
3. **Dense, not cramped.** Rows show state, key, step and age in one glance. Density comes from
   tight type and hairlines, never from small touch targets.
4. **Phone first, desktop extends.** Design each panel at 360 px wide first. Wider screens show
   more panels side by side; they do not get different components.
5. **State is shape and word, then color.** Every state has an icon and its daemon name. Color
   only reinforces it.
6. **Never answer by accident.** No gesture answers an ask. Answers to command asks need a second,
   deliberate tap.
7. **Agent text is untrusted.** Render questions, options, summaries and terminal output as text
   or sanitized Markdown. Never as HTML.
8. **Offline is normal.** Show the last known state with its age when the connection drops.
   Never blank a panel.

## Tokens

Tokens are CSS custom properties. The block below is the source of truth: copy it into the view's
stylesheet as it is, and style components only through these names. Never write a literal color
in a component rule.

The theme follows the system setting. A person can pin light or dark per device; the view stores
that choice locally and sets `data-theme` on `<html>`.

### Color

Each text role meets WCAG AA (4.5:1) on `--bg`, `--surface` and `--raised`, and on its own
`-soft` fill, in both themes. `--accent` and `--line-control` meet 3:1 on every surface, so focus
rings and control borders stay visible. `--line` is decorative: never use it as the only edge of
a control.

`--ok` and `--danger` differ mostly in hue. That is acceptable only because every state also
carries an icon and a word (principle 5).

```css
:root {
  color-scheme: light;
  /* Grounds */
  --bg: #F6F7F9;            /* page ground behind panels */
  --surface: #FFFFFF;       /* panels, cards, sheets */
  --raised: #F0F2F5;        /* hover, pressed, inset fields, skeletons */
  --line: #E3E6EB;          /* hairlines between rows and around panels */
  --line-control: #7E8595;  /* borders of inputs and secondary buttons, 3:1 */
  /* Text */
  --text: #15171C;          /* primary text */
  --text-2: #4B5161;        /* secondary text, labels */
  --text-3: #636A7A;        /* metadata, timestamps, placeholders */
  /* Accent: lapis, the blue of illuminated manuscripts */
  --accent: #2B4FC4;        /* links, focus ring, selection marker, accent icons */
  --accent-solid: #2B4FC4;  /* primary button fill */
  --on-accent: #FFFFFF;     /* text on --accent-solid */
  --accent-soft: #E9EEFC;   /* selected row, recommended option */
  /* State roles: text and icon on the matching -soft fill or any ground */
  --ok: #12704F;      --ok-soft: #E3F2EC;      /* done */
  --warn: #8A5200;    --warn-soft: #FCF0DA;    /* blocked, lost, needs you */
  --danger: #B3212B;  --danger-soft: #FCE9E9;  /* failed, exited */
  --hold: #6A3EC2;    --hold-soft: #F0EAFB;    /* held */
  --muted: #4B5161;   --muted-soft: #EDEFF3;   /* queued, asleep, dropped, ended runs */
  --danger-solid: #B3212B;  /* destructive button fill */
  --on-danger: #FFFFFF;
  /* Terminal: one dark palette in both themes, because agents print for a dark terminal */
  --term-bg: #0B0D12;
  --term-fg: #D5D9E2;
  --term-dim: #8C93A3;
  --scrim: rgba(15, 17, 23, 0.48);  /* behind sheets and dialogs */
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    color-scheme: dark;
    --bg: #0F1117;  --surface: #161920;  --raised: #1D2029;
    --line: #272B35;  --line-control: #6E7586;
    --text: #E7E9EF;  --text-2: #A8AEBC;  --text-3: #8C93A3;
    --accent: #8AA4FF;  --accent-solid: #3D5FD6;  --on-accent: #FFFFFF;  --accent-soft: #1C2442;
    --ok: #4FC99A;      --ok-soft: #112821;
    --warn: #E5AE4A;    --warn-soft: #2B2212;
    --danger: #FF8086;  --danger-soft: #311A1D;
    --hold: #B59CFF;    --hold-soft: #231C38;
    --muted: #A8AEBC;   --muted-soft: #232733;
    --danger-solid: #C93A44;  --on-danger: #FFFFFF;
    --scrim: rgba(0, 0, 0, 0.6);
    --shadow-overlay: 0 1px 2px rgba(0, 0, 0, 0.4), 0 8px 24px rgba(0, 0, 0, 0.5);
    --shadow-sheet: 0 -8px 32px rgba(0, 0, 0, 0.6);
  }
}
:root[data-theme="dark"] {
  /* The same values again, so a pinned theme wins both ways. */
  color-scheme: dark;
  --bg: #0F1117;  --surface: #161920;  --raised: #1D2029;
  --line: #272B35;  --line-control: #6E7586;
  --text: #E7E9EF;  --text-2: #A8AEBC;  --text-3: #8C93A3;
  --accent: #8AA4FF;  --accent-solid: #3D5FD6;  --on-accent: #FFFFFF;  --accent-soft: #1C2442;
  --ok: #4FC99A;    --ok-soft: #112821;
  --warn: #E5AE4A;  --warn-soft: #2B2212;
  --danger: #FF8086;  --danger-soft: #311A1D;
  --hold: #B59CFF;  --hold-soft: #231C38;
  --muted: #A8AEBC;   --muted-soft: #232733;
  --danger-solid: #C93A44;  --on-danger: #FFFFFF;
  --scrim: rgba(0, 0, 0, 0.6);
  --shadow-overlay: 0 1px 2px rgba(0, 0, 0, 0.4), 0 8px 24px rgba(0, 0, 0, 0.5);
  --shadow-sheet: 0 -8px 32px rgba(0, 0, 0, 0.6);
}

body { background: var(--bg); color: var(--text); }
```

Measured contrast, light and dark, on `--surface`: `--text` 17.9 and 14.5, `--text-2` 7.9 and
7.9, `--text-3` 5.4 and 5.7, `--accent` 7.0 and 7.4, white on `--accent-solid` 7.0 and 5.5,
`--line-control` 3.3 and 3.8. Each state role on its `-soft` fill is at least 4.7 in both themes.
Re-check with a contrast tool after any change to these values.

### Typography

One family for the interface, one for machine text.

- **Geist** (SIL OFL 1.1) for all interface text. It is narrow and even at 13 and 14 px, which
  keeps rows dense and legible.
- **Geist Mono** (SIL OFL 1.1) only for machine text: ids, task keys, branches, commit shas,
  commands and the terminal. Never for labels or headings.
- Ship the `woff2` files with the daemon and serve them from the view. Never load fonts from a
  CDN: the view must work on a factory with no internet.

```css
:root {
  --font-sans: "Geist", system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --font-mono: "Geist Mono", ui-monospace, "SF Mono", Menlo, Consolas, monospace;
}
```

| Style     | Size / line height | Weight | Use                                                         |
| --------- | ------------------ | ------ | ----------------------------------------------------------- |
| `caption` | 12 / 16            | 500    | badges, counts, timestamps in rows                          |
| `small`   | 13 / 18            | 400    | second line of a row, metadata, help text                   |
| `body`    | 14 / 20            | 400    | default text, row titles (500)                              |
| `reading` | 16 / 24            | 400    | ask questions, results, report prose, every input field     |
| `title`   | 16 / 24            | 600    | panel titles, sheet titles                                  |
| `heading` | 20 / 28            | 600    | the title of a task, run or ask page                        |
| `display` | 24 / 32            | 600    | the desk's count of open asks, nothing else                 |
| `mono`    | 13 / 20            | 400    | ids, keys, shas, commands; 12 / 16 inside rows              |

- Use three weights only: 400, 500, 600.
- Set inputs at 16 px at every width. Smaller inputs make iOS Safari zoom the page.
- Use `font-variant-numeric: tabular-nums` for every column of numbers, durations and times.
- Keep reading text under 72 characters per line.
- Use sentence case everywhere. Never uppercase labels.

### Spacing

A 4 px grid. Lay out siblings with `gap`, not margins.

```css
:root {
  --space-1: 4px;   /* icon to label */
  --space-2: 8px;   /* between items in a row, between touch targets */
  --space-3: 12px;  /* row vertical padding, card inner gap */
  --space-4: 16px;  /* side gutter at every width, card padding */
  --space-5: 24px;  /* between groups in a panel */
  --space-6: 32px;  /* between sections of a page */
  --space-8: 48px;  /* empty state padding */
}
```

### Radii

Radius marks what an element is. It does not decorate.

```css
:root {
  --radius-sm: 4px;      /* badges, kbd hints, chips */
  --radius-md: 6px;      /* buttons, inputs, menu items, option buttons */
  --radius-lg: 10px;     /* cards, panels on wide screens, sheets (top corners) */
  --radius-full: 999px;  /* status dots, toggles, count pills */
}
```

Rows inside a panel have no radius. Panels on compact screens are full bleed with no radius.

### Elevation

Hairlines separate; shadows mean "this floats above the page". Panels, rows and cards use
`--line`, never a shadow. The dark values sit in the theme blocks above; in dark, a floating
element also takes `--raised` as its fill, since shadows barely show on a dark ground.

```css
:root {
  --shadow-overlay: 0 1px 2px rgba(16, 18, 24, 0.06), 0 8px 24px rgba(16, 18, 24, 0.12); /* menus, popovers */
  --shadow-sheet: 0 -8px 32px rgba(16, 18, 24, 0.16);  /* bottom sheets, dialogs */
}
```

### Motion

Motion shows what changed after a person acts. Nothing moves on its own except the dot of a live
run.

```css
:root {
  --dur-1: 100ms;  /* press, hover, toggle */
  --dur-2: 160ms;  /* expand, collapse, fade */
  --dur-3: 240ms;  /* sheet in, page push */
  --ease-out: cubic-bezier(0.2, 0, 0, 1);  /* entering */
  --ease-in: cubic-bezier(0.4, 0, 1, 1);   /* leaving */
}
@media (prefers-reduced-motion: reduce) {
  :root { --dur-1: 0ms; --dur-2: 0ms; --dur-3: 0ms; }
}
```

- A new row gets an `--accent-soft` background that fades out over 1.2 s. With reduced motion it
  keeps a small accent dot until the person scrolls past it.
- The live-run dot pulses once per 2 s. With reduced motion it stays solid.
- Sheets slide up with `--dur-3`. With reduced motion they appear in place.
- Never animate layout height of a list that updates live. Insert rows without moving the row
  under the person's thumb: if they scrolled, keep their scroll position and show "3 new" at the
  top instead.

## Layout

### Breakpoints

| Name    | Width         | Layout                                                                 |
| ------- | ------------- | ---------------------------------------------------------------------- |
| compact | below 600 px  | One panel at a time. Bottom tab bar. Detail pages push over the list.  |
| medium  | 600 to 1023   | List and detail side by side, list 320 to 400 px wide.                 |
| wide    | 1024 and up   | The view file's tree of rows, columns and tabs, as written.            |

- A view file (R8.2) describes the wide layout. On compact screens, each leaf panel becomes a tab
  in the tree's order. The bottom bar shows the first four and a "More" tab for the rest.
- The default view puts the desk first, then queue, tasks and runs. Inbox, workflows, reports and
  terminals sit under "More".
- Panels lay out by their own width with container queries, not by the viewport. A panel can be
  320 px wide on a 1440 px screen.
- A panel narrower than 320 px in a wide layout folds into its sibling's tabs.
- Every task, run and ask has its own URL, such as `/asks/42`. A link from a notification opens
  that ask directly, on any screen size.

### Frame

- Top bar, 48 px: factory name, connection state, search, theme menu. It stays sticky below the
  safe-area inset.
- Bottom tab bar on compact, 56 px plus `env(safe-area-inset-bottom)`. The desk tab shows the
  count of open asks.
- Side gutter: `max(var(--space-4), env(safe-area-inset-left))` and the same on the right.
- The page never scrolls sideways. Only tables, diagrams and the terminal scroll inside their own
  box.
- Use `height: 100dvh` for one-screen layouts so mobile browser bars do not hide the tab bar.

## Components

Every component below lists its states. Build no state that is not listed without adding it here
first.

### Status badge

A badge names a task or run state with an icon and the daemon's exact word, in lowercase, as
`priory tasks` and `priory runs` print it.

Anatomy: 20 px tall, `--radius-sm`, padding `0 6px`, `caption` style, 12 px icon, role text on
the role's `-soft` fill. Icons come from [Lucide](https://lucide.dev) (ISC license), inlined as
SVG.

Task states (`crates/priory-core/src/state.rs`, `tasks.state`):

| State     | Role     | Icon            | Means for the person                          |
| --------- | -------- | --------------- | --------------------------------------------- |
| `queued`  | muted    | `circle-dashed` | Waits in the queue for its turn.              |
| `held`    | hold     | `pause`         | Held by a person or a condition; shows why.   |
| `ready`   | accent   | `circle-dot`    | Next to start when a runner has room.         |
| `running` | accent   | live dot        | An agent is working on it now.                |
| `asleep`  | muted    | `moon`          | Waits on an event; holds no process.          |
| `blocked` | warn     | `hand`          | Needs a person. Always on the desk.           |
| `done`    | ok       | `check`         | Finished.                                     |
| `failed`  | danger   | `circle-x`      | Ended in failure. Can be revived.             |
| `dropped` | muted    | `minus`         | Stopped on purpose. Can be revived.           |

An asleep task whose `retired` column is set shows `asleep` plus a second muted chip `retired`
with the `archive` icon: it has no worktree and resurrects on its event.

Run states (`runs.ended_as`; a run with no `ended` is live):

| State       | Role   | Icon          | Means                                                 |
| ----------- | ------ | ------------- | ----------------------------------------------------- |
| `running`   | accent | live dot      | Live. Shows `idle` in `--text-3` when the agent waits for input. |
| `done`      | ok     | `check`       | The task reached done in this run.                    |
| `moved`     | muted  | `arrow-right` | The step ended; the task went to its next step.       |
| `asleep`    | muted  | `moon`        | Ended by `wait` or `ask`.                             |
| `blocked`   | warn   | `hand`        | Ended by a verdict that needs a person.               |
| `lost`      | warn   | `unplug`      | Lost with its tmux server; resumes once.              |
| `restarted` | muted  | `rotate-ccw`  | Restarted by a handler.                               |
| `cancelled` | muted  | `x`           | Cancelled by a person.                                |
| `dropped`   | muted  | `minus`       | The task was stopped.                                 |
| `exited`    | danger | `log-out`     | The agent process exited unexpectedly.                |
| `failed`    | danger | `circle-x`    | The run failed.                                       |

If the daemon reports a state not in these tables, show it as a muted badge with its raw word and
the `circle-help` icon. Add it here in the same change that adds it to the daemon.

### Ask card

The most important component. It shows one ask (`asks` table, R6.3) and takes its answer.

Anatomy, top to bottom:

1. **Context line** (`small`, `--text-3`): task key in `mono`, the step, the lane, and the age on
   the right ("12 min"). Tapping it opens the task.
2. **Question** (`reading`, `--text`): the full question. Clamp at 8 lines with "Show all".
3. **Options**: one full-width button per option, in the agent's order. Never reorder them.
   Each is at least 48 px tall, `--radius-md`, left-aligned label, `--line-control` border.
   The recommended option has an `--accent-soft` fill, an `--accent` border and a "Recommended"
   caption under its label.
4. **Trade-off** (`small`, `--text-2`), under the options, headed "Trade-off".
5. **Other answer**: a "Write an answer" button that expands into a 16 px text field with a
   "Send" button. Free text is always possible.
6. **Footer** (`caption`, `--text-3`): the revision ("revision 2") and links to the task and the
   run's terminal.

States:

| State        | What the person sees                                                                      |
| ------------ | ----------------------------------------------------------------------------------------- |
| open         | As above.                                                                                 |
| sending      | The chosen option shows a check; the others dim. A bar at the bottom says `Sending "Merge"` with an "Undo" button for 4 s. The view sends after the 4 s. |
| answered     | The card collapses to the question, the answer, who answered and when. No buttons.        |
| refused      | `--warn-soft` banner: "The question changed. Read revision 3 and answer again." The card reloads the new revision; the old answer stays in the text field. |
| taken        | "Already answered 2 min ago: Merge." Shown when another answer came first.         |
| withdrawn    | Muted: "Withdrawn: the task was revived." No buttons.                                     |
| offline      | Options stay visible but disabled, with "Reconnect to answer".                            |

A **command ask** (R6.8) also shows the command and its target in a `mono` block on `--raised`
above the options. Its answer runs the command, so:

- It has no undo bar. The first tap turns the option into "Tap again to run" in `--accent-solid`
  for 4 s; the second tap sends.
- After sending it shows the outcome: `running`, `done`, `failed` with the error, or `uncertain`
  in warn: "Priory stopped while this ran. Check the target before you act on it."

### Desk and answer mode

- The desk lists, in this order: room banners (R4.3), open asks oldest first, blocked tasks,
  uncertain prompts (R5.4), then reports waiting for review.
- On compact screens, "Answer" opens **answer mode**: one ask fills the screen. The question
  scrolls at the top; the options sit in the bottom half, inside the thumb's reach.
- After an answer, the next ask slides in. "3 of 7" sits in the top bar.
- "Skip" (bottom left) and "Next" move without answering. A horizontal swipe does the same.
  Nothing answers except a tap on an option or "Send".
- Leaving answer mode returns to the desk at the same scroll position.

### Task row

- Compact, two lines, at least 56 px tall: line 1 is the status badge (icon only, with the state
  as its accessible name) and the task key in `mono`, truncated in the middle so the number
  stays visible. Line 2 (`small`, `--text-3`) is workflow and step, lane, band and age.
- Medium and wide, one line, 36 px tall: badge with word, key, workflow and step, lane, band,
  age, each in its own column.
- A `blocked`, `held` or `failed` row shows its reason on line 2 in the role's color.
- States: default, hover (`--raised`), pressed (`--raised`), selected (`--accent-soft` and a
  2 px `--accent` bar on the leading edge), new (fading `--accent-soft`), focused (focus ring).
- Actions live on the task page, not in the row: Resume, Cancel, Revive, Hold, Release, Move.

### Run row

- Run id in `mono`, task key, step, agent, started, duration in `tabular-nums`, and the run
  badge. A live run shows its duration counting up once per minute, not per second.
- Tapping a run opens its terminal.
- Same states as the task row.

### Panel chrome

- Header, 44 px, sticky: title (`title` style), count in `--text-3`, then up to two icon buttons
  (filter, menu). Any further action goes in the menu.
- Body scrolls on its own with `overscroll-behavior: contain`.
- Wide screens: `--surface`, 1 px `--line` border, `--radius-lg`. Compact: full bleed, no border.
- When the event stream drops, a `--warn-soft` strip under the header says "Reconnecting. Last
  update 2 min ago." The panel keeps its rows.

### Terminal pane

- Always the `--term-*` palette, in both themes. `mono` at 12 / 16 on compact, 13 / 18 on wide.
- tmux sets the width, so lines never wrap. The pane scrolls sideways inside its own box. A "Fit"
  toggle scales the text down to fit the width, never below 9 px.
- Read-only by default. "Type" opens a one-line input at the bottom that sends a line on Enter.
  The header shows `read-only` or `typing` as a badge.
- Header: session name in `mono`, run id, the run badge, "Copy screen", "Fit".
- When the run ends, the pane keeps its last screen and shows the run badge.

### Workflow diagram

- Steps are nodes; edges carry the outcome that takes the task from one step to the next.
- Node shape says who decides: model steps have `--radius-lg` corners, human steps have square
  corners and the `user` icon, code steps have `--radius-sm` corners and the `cog` icon.
- An irreversible step has a `--danger` border and the `lock` icon.
- The task's current step has an `--accent-soft` fill and `--accent` border. Visited edges are
  solid `--text-2`; others are dashed `--line-control`.
- Top to bottom on compact, left to right on wide. Pinch and drag to move; a "Fit" button resets.

### Reports and tables

- Numbers right-aligned in `tabular-nums`. The first column stays sticky when the table scrolls
  sideways on a phone.
- Headers in `caption` style, `--text-2`, sentence case.

### Buttons and inputs

| Kind      | Fill             | Text            | Border           | Use                                  |
| --------- | ---------------- | --------------- | ---------------- | ------------------------------------ |
| primary   | `--accent-solid` | `--on-accent`   | none             | One per region: the expected action. |
| secondary | `--surface`      | `--text`        | `--line-control` | Every other action.                  |
| ghost     | none             | `--accent`      | none             | Links and low-weight actions.        |
| danger    | `--danger-solid` | `--on-danger`   | none             | Cancel task, drop. Always confirms.  |

- Height 32 px on a fine pointer, 44 px on a coarse one (`@media (pointer: coarse)`).
- States: hover (8 % darker fill or `--raised`), pressed, focused, disabled (`--text-3` text,
  no fill change, `aria-disabled`), busy (an inline 12 px spinner replaces the icon; the label
  stays).
- Inputs: 16 px text, `--surface` fill, `--line-control` border, `--radius-md`, 44 px tall on
  coarse pointers. Errors show under the field in `--danger` with the `circle-x` icon.

### Empty, loading and error states

- **Empty**: one sentence that says what appears here and what makes it appear, plus the action
  or CLI command. "No open asks. Agents ask here when they need you."
- **Loading**: show nothing for the first 300 ms, then skeleton rows on `--raised` at the real
  row height. Never a full-page spinner. A list that has data keeps it while it refreshes.
- **Error**: where it failed, why, and how to fix it, with a "Retry" button.
  "Could not load runs: the daemon refused the connection. Check that the daemon runs on
  its runner, then retry." Keep any stale data visible under the error.
- **Banner** (desk top, R4.3): `--warn-soft`, the `hard-drive` or `memory-stick` icon, and the
  sentence from the daemon: what is short, by how much, since when, and the waiting task.

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

- Meet WCAG 2.2 AA. The tokens above already meet the contrast part; keep it that way.
- Focus ring: 2 px solid `--accent` with a 2 px offset, on every focusable element, using
  `:focus-visible`.
- Every control works by keyboard. Desktop shortcuts: `j` and `k` move, `Enter` opens, `1` to
  `9` pick an option, `w` writes an answer, `Esc` closes, `?` lists shortcuts. Single-key
  shortcuts never fire inside an input, and a setting turns them off.
- Each panel is a `region` labelled by its title. The tab bar is a `nav`.
- A new ask is announced once through a polite live region: "New ask from task 61." Never
  assertive.
- An icon-only badge has the state word as its accessible name. Decorative icons are
  `aria-hidden`.
- The terminal is a `pre` with an accessible name ("Terminal of run 18") and a "Copy screen"
  button, so its text can be read elsewhere.
- Text reflows at 320 px wide and at 200 % zoom. Only the terminal, diagrams and wide tables
  scroll in two directions.
- Under `prefers-contrast: more`, `--line` takes the value of `--line-control`.
- Under `forced-colors: active`, badges and option buttons keep a 1 px `CanvasText` border.

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

Change `DESIGN.md` in the same pull request as the code that needs the change, and update the
artifact to match. A new component, state or token starts here, not in the code.

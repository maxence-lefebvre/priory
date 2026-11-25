# The web view in React

This page explains how the web view moves from hand-built HTML strings to a React app, slice by
slice, without a visible change. It holds the plan the lead approved on 2026-11-23, with the person's "go
SPA" decisions applied. [ADR 0004](../adr/0004-the-web-view-is-a-react-app-whose-built-bundle-is-committed.md)
records the build decision.

## Why

`render()` in `app.js` rebuilds `#app` with `innerHTML` on every change. It then restores focus,
caret, scroll, open menus and countdowns by hand. Each missed case is a bug a person sees: the ask
drawer loses Brief, Inbox and Trace on a refresh, and the task page's Say textarea loses focus. A
component tree keeps its elements across renders, and a query cache keeps the last data on screen
while it re-reads. Both remove that restore code, page by page.

## Measured before slice 1

| What | Number | How |
|---|---|---|
| Web view source | 7,601 lines of JavaScript in 36 files. `app.js` is 3,389 lines. | `wc -l` on `main` at ecb1035 |
| JavaScript the page loads | 326 KB raw, 96 KB gzip, in 17 modules | the files `app.js` imports |
| Everything a cold load transfers | 485 KB in 24 requests, fonts included. The server sends no `Content-Encoding`. | Chromium resource timing, `encodedBodySize` |
| Page load on the demo server | DOMContentLoaded 56 to 67 ms. Desk drawn (first `.row` or `tr`) 208 to 235 ms. | 5 cold loads, 27 s apart, headless Chromium, debug build |
| The `node --test` line | 491 tests in 11.7 s locally. CI took 10 to 14 s over 8 runs on `main`. | |

The first plan measured a median of 1.34 s to the drawn Desk on another machine. Most of a load
waits on the seven `/api` reads of `refresh()`, not on JavaScript. Loads run back to back are slower:
a closed page's `/api/events` long poll holds one of the 8 workers for up to 25 s. Hence the 27 s gap.

## 1. Workspace

```
package.json               # private root: "packageManager": "pnpm@12.21.0", scripts web:build, web:check
pnpm-workspace.yaml        # packages: crates/priory-core/web
pnpm-lock.yaml
.gitattributes             # crates/priory-core/web/dist/** linguist-generated -diff
crates/priory-core/web/
  package.json             # @priory/web
  build.mjs                # esbuild: main.jsx -> dist/app.js; --check compares instead of writing
  main.jsx                 # the React shell (slice 2a): root, route, data, events, LegacyPage
  router.js, queries.js, events.js, store.js   # the shell's plain modules, with node:test files
  dist/app.js              # the built bundle, committed
  *.js, *.test.js          # view models and their tests: unchanged, same place
  app.js                   # the legacy drawing: drawn inside LegacyPage, shrinking page by page
  app.css, sw.js, boot.js  # served as files, as before
```

- The web app stays in `crates/priory-core/web/`. Moving it would change `serve.rs`, the Verify glob
  and every file's history, for no gain.
- The view models (`asks.js`, `desk.js`, `tasks.js` and the others) stay plain ES modules with their
  `*.test.js` beside them. The Verify `node --test` line does not change. The Herdr adapter's tests
  still compare its copies of `tasks.js` and the others with these files.
- The test runner stays Node's built-in `node:test`.
- The Herdr and Claude plugin adapters stay out of the workspace: they run unbundled inside their
  hosts. The GitHub extension joins later, see section 8.

## 2. Build and embedding

The built bundle `dist/app.js` is committed. `serve.rs` embeds it with `include_bytes!` and serves it
at `/app.js`. CI rebuilds it and fails on any difference.

| Path | Needs Node? |
|---|---|
| `cargo build`, `test`, `clippy`, `cargo install --path` or `--git` | No: `include_bytes!` reads the committed file |
| Nix flake (`buildRustPackage` on the git tree) | No |
| `install.sh` (downloads a release binary) | No |
| Release workflow (`cargo build --release` on a checkout) | No |
| CI `check` job | Yes, one step: `corepack pnpm install --frozen-lockfile && corepack pnpm web:check` |
| Changing web code | Yes: `corepack pnpm web:build`, then `cargo build` |

- esbuild writes one minified file with no code splitting, so no new served path appears.
- `build.mjs --check` builds in memory, compares with `dist/app.js`, and fails with "run corepack
  pnpm web:build" on a stale bundle. It also fails past a 500 KB budget: the JavaScript
  plus React, with room for the page ports.
- pnpm comes through corepack, which ships with Node 22 and 24. The exact `packageManager` pin keeps
  every machine on 12.21.0. No nixpkgs package and no new Action.
- `pnpm-workspace.yaml` refuses esbuild's install script. It only checks the binary that esbuild's
  platform package already holds.

Rejected:
- A `build.rs` that runs pnpm: every `cargo build`, clippy run and `cargo install` would need Node,
  and the Nix build would need a pnpm dependency hash kept in step with the lockfile.
- A release-time build: a `cargo build` from a checkout would embed no app, and the flake would need
  a second derivation.

Cost: two open web pull requests both change `dist/app.js` and conflict on it. The fix is
mechanical: merge `main`, run `corepack pnpm web:build`, commit. CI catches a bundle merged by hand.

## 3. Dependencies

Approval of this plan approves exactly this list. Versions and dates come from the npm registry on
2026-11-23.

| Package | Licence | Pin | Released | Kind | From | Why |
|---|---|---|---|---|---|---|
| esbuild | MIT | 0.28.2 | 2026-08-08 | dev | slice 1 | Bundles, minifies and compiles JSX. No plugins. |
| pnpm | MIT | 12.21.0 | 2026-11-13 | tool, via corepack | slice 1 | The workspace and lockfile the person asked for |
| react | MIT | 19.3.1 | 2026-10-14 | runtime | slice 2a | Components |
| react-dom | MIT | 19.3.1 | 2026-10-14 | runtime | slice 2a | `createRoot`; `react-dom/server` renders component tests to markup. Brings `scheduler` (MIT). |
| @tanstack/react-query | MIT | 5.107.1 | 2026-11-06 | runtime | slice 2a | The server data cache, approved by the person. Brings `@tanstack/query-core` (MIT). |

Slice 2a pins `@tanstack/react-query` at 5.107.1, the newest patch at least a week old on
2026-11-23. The newest was then 5.108.4, released that day.

Rejected:
- Vite and `@vitejs/plugin-react` (MIT, 8.3.8 and 6.3.0, November 2026): `cargo` embeds the assets and
  the server proxies no dev server. It pulls rolldown, lightningcss and postcss.
- TypeScript and `@types/react`: it would widen the port. It is a separate decision; esbuild strips
  types if it comes.
- Vitest, jsdom and Testing Library: `node:test` covers the view models and the query layer,
  `renderToStaticMarkup` the markup, and browser checks against the demo the interactions.
- TanStack Router and React Router: `pages.js` and the History API already route. A router comes
  only with a concrete need, such as a loader a page cannot do without.
- Preact: smaller, but the person asked for React.
- Tailwind or CSS-in-JS: `DESIGN.md` tokens and the CSP (`style-src 'self'`) keep plain CSS files.

## 4. Migration order

1. **Slice 1, the bundle.** The pnpm workspace, esbuild, `dist/app.js` built from today's `app.js`,
   the CI step, `serve.rs` and `sw.js`, the serve test, the docs and this note. No change in drawing
   or behaviour.
2. **Slice 2a, the shell and the query layer.** Built. `main.jsx` is the bundle's entry. React owns
   the root, the route, the server data and the events long poll:
   - `router.js` holds the route. `go()` pushes it with the History API and draws the new page
     before it returns, as `app.js` expects. Back moves it too. Each visit draws, also one to the
     address already shown.
   - `queries.js` holds one `QueryClient`, with one query per listing. `refresh()` reads every
     listing from the server, drops the read when a later one supersedes it, and then writes all of
     them in one step. So a read draws once, and a failed read changes nothing. No component observes
     the queries yet: the first page port does.
   - `events.js` is the long poll, which `main.jsx` runs. `/api/events` answers only a revision, not
     what changed, so a moved revision reads every listing again. Structural sharing keeps each
     unchanged listing the same object, so a read with nothing new draws nothing.
   - `LegacyPage` renders `#app` once and lets `app.js` draw inside it, frame included. A draw that
     throws is reported and leaves the page as it was, since React would unmount the root.
   The person approved that the frame (navigation, the top bar, the page title) moves to React with
   slice 2b, since the compact and wide layouts compose it around each page differently.
3. **Slice 2b, the task page**, stacked on 2a. The Say textarea bug lives there. The frame moves
   with it.
4. Then one pull request per page: Asks with the answer sheet, Desk, Tasks, Runs, Queue, Inbox,
   Costs, Reports, Flows, Batch, Terminal, Settings and sign-in, the Add sheet. Each port deletes
   that page's legacy drawing.
5. **Last:** delete `LegacyPage`, `html`, `raw`, `render()` and its restore code, and the document
   event delegation.

There are no islands and no `[data-island]` event contract. Inside `LegacyPage` the legacy handlers
keep working as they do today, on `document`. A port removes its page's cases from them. Controls React draws use React handlers.
The shell's links are plain `<a href>` that one click handler routes, as `app.js` does today, so
modified clicks and hashes keep their behaviour.

Rules for every port:
- Move only the drawing. View models keep their signatures and tests.
- Ported markup keeps the same elements, classes and ARIA, so `app.css` and `DESIGN.md` hold.
- Agent text stays escaped: JSX escapes by default. `markdown.js` output, already escaped, renders
  through one `Markdown` component, the only `dangerouslySetInnerHTML`.
- Each port shows before and after screenshots at 1440 and 390 px, light and dark, and runs a
  browser check against the demo for its interactions: press twice, the arm timeout, focus after a
  refresh, scroll kept.

## 5. What stays the same

- URLs, including `/asks/42`, `?view=` queries, modified clicks and in-page hashes. The server still
  serves the index for every page path.
- The API, the events long poll, the token and the snapshot in `localStorage`.
- The service worker: network first, the cached copy offline, Web Push and its `priory-push` cache.
  Slice 1 renames the shell cache to `priory-shell-2`, and `activate` deletes the older
  `priory-shell`, so an offline start never mixes the old `app.js`, which imported modules, with the
  bundle. `install` caches the whole list with `addAll`, which fails whole, so a broken install
  keeps the old worker.
- The CSP. The bundle is a same-origin script.
- `DESIGN.md` rules and tokens, `app.css`, the 390 px phone layout, the themes, `boot.js`'s first paint.
- The four Verify commands. A fifth joins them, and runs before the Node tests, which import
  TanStack Query: `corepack pnpm install --frozen-lockfile && corepack pnpm web:check`.

## 6. In-flight web work

Four briefs change today's web code: `priory-asks-decide-first-web` (Asks order, press twice),
`priory-desk-links-web` (Desk links), `priory-web-select-dark` (`app.css` only) and
`priory-web-ask-flicker` (the drawer and the Say textarea on refresh).

- Slice 1 does not wait for them. It changes no drawing code. Whichever merges second merges `main`
  and runs `corepack pnpm web:build`. A brief that adds a module adds no `serve.rs` or `sw.js` row:
  the bundle holds it. `app.css` is served as a file, so the select fix needs no rebuild.
- Slice 2b waits for `priory-web-ask-flicker`. The Asks and Desk ports wait for their briefs. Each
  port keeps those briefs' view-model tests unchanged and passing.
- The Asks port's browser check replays decide-first's rules: the first press arms, a second within
  `ARM_MS` answers, a late press re-arms, a press on another option moves the arm. Its markup test
  checks the order: title, recommendation, options, the action row with Snooze, chips, sections.
- The Desk port's markup test checks the link spans.

## 7. Refresh behaviour

Rules, each with a test:

1. **Keep the last data while re-reading.** A refresh never swaps a drawn page or section for a
   loading state. A ported page's query uses `placeholderData: keepPreviousData`, and structural sharing keeps
   the objects that did not change, so their components do not redraw. Only a first visit to an id
   draws "Loading". A failed re-read keeps the last data and shows the error beside it.
2. **Never unmount a focused input or move its caret.** Keys come from ids, never from a revision or
   an index. No component returns a loading state once it has data.
3. **Keep scroll and open menus** across a refresh, for the same reason.

Tests:
- `queries.test.js`, with a fake server: the cache shows the last listings until a re-read lands; a
  failed re-read keeps them; a read lands in one notification; a read with nothing new keeps every listing the same object, and one
  with a change replaces only that listing. `events.test.js` covers the long poll.
- `scripts/demo/refresh.mjs`, a browser check. It changes the demo's data while a page is open and
  samples the DOM every animation frame. It fails on the ask drawer losing a section it drew
  (`/asks/1`: Brief, Inbox, Trace), and on a task page drawing "Loading task" or its Say field
  losing focus, text or caret. It drives Chromium over the DevTools protocol with Node's own
  WebSocket, so it needs no package, only the `sqlite3` command. It also checks that a visit to the
  same address draws, a superseded refresh does not, and a draw that throws keeps the page. CI has no browser, so each port's pull request runs it against
  the demo and pastes its output. Run against a build from before #418, it fails on both pages.

## 8. The GitHub extension

`adapters/github/` is a Manifest V3 extension of about 1,500 lines with a hand-written
`manifest.json`, a content script and a side panel. It joins the workspace as
`@priory/github-extension` after slice 2a, so it can share React components, such as the ask card,
with the web view.

| Option | Licence | Latest | Weighed |
|---|---|---|---|
| WXT | MIT | 0.21.5, 2026-10-26 | Generates the manifest and reloads in development. Brings Vite and about 25 more packages. Its 0.x API still changes. |
| Vite with `@vitejs/plugin-react` | MIT | 8.3.8, 2026-11-11 and 6.3.0, 2026-11-09 | A dev server the extension cannot use without a plugin for the manifest. |
| esbuild, already in the workspace | MIT | 0.28.2, 2026-08-08 | One entry per script, the manifest kept by hand. No new dependency. |

Recommendation: esbuild. The manifest is small, and the extension's CSP already keeps scripts
same-origin. WXT is worth it only if the extension targets Firefox or Safari too.

The new pull request page fix (`priory-github-new-pr-page`) lands before the move or carries its
tests into the package unchanged.

## Design artifact

The port changes no pixel, so the artifact records the screens each slice must reproduce, not a new
look: the shell and `LegacyPage` diagram, the build path, and today's screens at 1440 and 390 px.
The person approved it on 2026-11-24: https://claude.ai/artifact/DJzXheB2nzY27bSuxTgerC.

## Risks

1. Legacy and React handlers both acting on one click: legacy delegation listens on `document`,
   so each port removes its page's cases, with a test per page.
2. Offline start after the upgrade: the cache rename and cleanup, tested in `sw.test.js`.
3. Bundle size over plain HTTP: minifying the legacy code offsets part of React. The budget fails
   the build past 500 KB.
4. A stale or hand-merged bundle: `web:check` in CI.

## Later slices

1. Slice 2b: the task page, and the frame.
2. The page ports of section 4, one pull request each.
3. The GitHub extension in the workspace.
4. Deleting `LegacyPage` and the legacy drawing.

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
  test.mjs                 # runs the tests of components/, hooks/ and pages/, which hold JSX
  .oxlintrc.json           # the lint rules, each one turned off with its reason
  main.jsx                 # the entry: root, router, cache, events, the hidden tab
  routes.jsx               # the route tree, in code
  router.js, frame.js      # the history and links; where an address leads, and what it reads
  queries.js, data.js, events.js, session.js, shown.js, acts.js, store.js   # the plain modules
  pages/<page>.jsx         # one file per ported page, its test beside it
  components/, hooks/      # what several pages draw, and the hooks that read
  dist/app.js              # the built bundle, committed
  *.js, *.test.js          # view models and their tests: unchanged, same place
  registry.js, entries/<page>.js   # each page's entry: what fills its slots, and its events
  reads/<family>.js        # the keys and reads beside the listings, each with its hook in hooks/
  app.js, legacy/<page>.js # the legacy drawing, one file per page, inside a legacy slot
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
  pnpm web:build" on a stale bundle. It also fails past a 720 KB budget, which holds the legacy and
  the React pages while both exist. The last port lowers it.
- `web:check` then runs oxlint, where a warning fails too, and `test.mjs`. CI's `node --test` line
  names only the tests beside `app.js`, so the tests in folders run here and no workflow changes.
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

| @tanstack/react-router | MIT | 1.170.46 | 2026-10-22 | runtime | foundation | Owns every address, as the person decided on 2026-11-25. Brings `@tanstack/history`, `router-core`, `react-store` and `store`, `cookie-es`, `seroval`, `seroval-plugins` and `use-sync-external-store` (all MIT), and `isbot` (Unlicense). |
| oxlint | MIT | 1.94.0 | 2026-11-16 | dev | foundation | Lints the web view in `web:check`. Named by the person on 2026-11-25. |

The router uses no plugin and no code splitting. Its routes are code.

Slice 2a pins `@tanstack/react-query` at 5.107.1, the newest patch at least a week old on
2026-11-23. The newest was then 5.108.4, released that day.

Rejected:
- Vite and `@vitejs/plugin-react` (MIT, 8.3.8 and 6.3.0, November 2026): `cargo` embeds the assets and
  the server proxies no dev server. It pulls rolldown, lightningcss and postcss.
- TypeScript and `@types/react`: it would widen the port. It is a separate decision; esbuild strips
  types if it comes.
- Vitest, jsdom and Testing Library: `node:test` covers the view models and the query layer,
  `renderToStaticMarkup` the markup, and browser checks against the demo the interactions.
- React Router: the person chose TanStack Router. The first plan rejected both, since `pages.js` and
  the History API already routed.
- Preact: smaller, but the person asked for React.
- Tailwind or CSS-in-JS: `DESIGN.md` tokens and the CSP (`style-src 'self'`) keep plain CSS files.

## 4. Migration order

1. **Slice 1, the bundle.** The pnpm workspace, esbuild, `dist/app.js` built from today's `app.js`,
   the CI step, `serve.rs` and `sw.js`, the serve test, the docs and this note. No change in drawing
   or behaviour.
2. **Slice 2a, the shell and the query layer.** Built, then replaced by the foundation.
3. **The foundation.** Built. It holds what each port needs:
   - `routes.jsx` is the route tree. Every page is a child of one route that renders `PageGrid`, so a
     slot stays mounted from one page to the next. `router.js` holds the history, and
     the one click handler for plain `<a href>` links: modified clicks and other sites are the
     browser's, and a hash of this page scrolls without changing the address. A search string keeps
     its own text, since the router's default codec would rewrite `?view=` links as JSON.
   - `frame.js` says where an address leads, which listings it shows, and the list a detail was
     opened from. A history entry keeps that list, so Back restores it.
   - `queries.js` holds the cache rules of section 7, and `hooks/useData.js` one hook per listing and
     per detail. The frame reads `/api/views`, `/api/links` and `/api/desk`. A page reads its own.
   - `PageGrid` draws one slot per panel and one for the drawer. A slot holds a React part or a legacy
     one, as the page's entry says. A legacy slot draws with `innerHTML` in an element with `display: contents`, inside the React
     frame. Its listings land together, once none of them is being read, so a legacy draw never mixes
     two reads. A draw that throws is reported and leaves the page as it was.
   - The frame, the sign-in and the task page are React.
4. Then one pull request per page: Asks with the answer sheet, Desk, Tasks, Runs, Queue, Inbox,
   Costs, Reports, Flows, Batch, Terminal, Settings, the Add sheet. Each port deletes that page's
   file under `legacy/`.
5. **Last:** delete the legacy slot, `html`, `raw`, `render()` and its restore code, and the document
   event delegation. Lower the bundle budget.

There are no islands and no `[data-island]` event contract. Inside a legacy slot the legacy handlers
keep working as they do today, on `document`. Controls React draws use React handlers.

### How to port one page

Two ports touch no common line but the bundle's: a port edits `entries/<page>.js`,
`scripts/demo/pages/<page>.mjs` and files of its own. Every other file names no page or stays as it
is. `registry.js` says what an entry holds.

1. Write `pages/<Page>.jsx` and `pages/<Page>.test.jsx`. Read with a hook of `hooks/`: the listings' in
   `useData.js`, then `useBatch`, `useCosts`, `useFlows`, `useInbox` and `useRequest`. A new read is
   a file in `reads/`.
2. Draw the same elements, classes, ids and ARIA as the legacy file: `List` returns its `.list` beside
   other panels, `Page` what stands alone, `Detail` one task or batch, `Drawer` the ask's `.drawer`.
   `usePlace()` gives the size.
3. In the page's entry, put the components in place of the legacy `list`, `page`, `detail` or `drawer`.
   Delete the `needs`, `fields`, `kept`, `older` and events those served.
4. Delete `legacy/<page>.js`, unless the list below keeps it. Its part of `state` goes with it.
5. Add the page's reads to `scripts/demo/pages/<page>.mjs`, and a `check` there for what it keeps
   through a refresh.
6. Run `corepack pnpm web:build`, `web:check` and `scripts/demo/refresh.mjs`. With
   `WEB=crates/priory-core/web` the last one needs no Rust build.

No port deletes these files:

| File under `legacy/` | Until |
|---|---|
| `state.js`, `shared.js`, `data.js`, `frame.js` | the last port: every legacy part draws with them |
| `terminal.js` | the Terminal and the task page's pane are both in React |
| `sections.js` | Asks is ported: its drawer draws a task's sections |
| `taskacts.js` | Desk, Tasks and Inbox are ported |
| `desk.js` | Settings is ported: its theme previews draw the Desk. Asks and Queue use its model |
| `asks.js` | Desk is ported: it draws ask rows |
| `batch.js` | Asks is ported: a go ask draws the plan's lanes |
| `tasks.js` | Queue is ported: its Held and Last day views draw task rows |
| `add.js` | Desk and Inbox are ported, and the Add sheet is in React |
| `settings.js` | More is ported: the file draws both pages |

Such a port deletes only what no other file imports.

When `dist/app.js` conflicts, merge `main`, run `corepack pnpm web:build` and commit.

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
- The API, the events long poll, the token and the snapshot in `localStorage`. The snapshot keeps each
  listing this device has read, with its own age. Since a page reads only its own listings, one never
  read shows as loading offline.
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

0. **Read only what is on screen.** A moved revision reads each mounted query again, at most one read
   of each in flight. A person's action or Retry starts a new read, which supersedes one in flight. A
   hidden tab reads nothing, and reads once when it is shown. `/api/events` answers only a revision,
   not what changed. Once a minute a shown page reads its queries again, since a snooze ends and an age
   moves with no new revision.
1. **Keep the last data while re-reading.** A refresh never swaps a drawn page or section for a
   loading state. A ported page's query uses `placeholderData: keepPreviousData`, and structural sharing keeps
   the objects that did not change, so their components do not redraw. Only a first visit to an id
   draws "Loading". A failed re-read keeps the last data and shows the error beside it.
2. **Never unmount a focused input or move its caret.** Keys come from ids, never from a revision or
   an index. No component returns a loading state once it has data.
3. **Keep scroll and open menus** across a refresh, for the same reason.

Tests:
- `queries.test.js`, with a fake server: a page reads its own listings and no other; a move joins a
  read in flight, and a person's refresh supersedes it; a hidden tab reads nothing; the cache shows the
  last listings until a re-read lands; a failed re-read keeps them; a read with nothing new keeps every
  listing the same object, and one with a change replaces only that listing; a kept snapshot restores,
  with each listing's own age. `events.test.js` covers the long poll, `frame.test.js` each page's reads.
- `scripts/demo/refresh.mjs`, a browser check. It changes the demo's data while a page is open and
  samples the DOM every animation frame. It fails on the ask drawer losing a section it drew
  (`/asks/1`: Brief, Inbox, Trace), and on a task page drawing "Loading task" or its Say field
  losing its element, focus, text or selection. It counts what each page reads once the revision
  moves, and what a hidden tab reads. It also checks Back and forward, that a superseded refresh never
  draws, and that a legacy draw that throws keeps the page. It drives Chromium over the DevTools
  protocol with Node's own WebSocket, so it needs no package, only the `sqlite3` command. CI has no
  browser, so each port's pull request runs it against the demo and pastes its output.

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
   the build past 720 KB.
4. A stale or hand-merged bundle: `web:check` in CI.

## Later slices

1. The page ports of section 4, one pull request each.
2. The GitHub extension in the workspace.
3. Deleting the legacy slot and the legacy drawing.

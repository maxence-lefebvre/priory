# The web view is a React app whose built bundle is committed

Date: 2026-11-23. Status: accepted, 2026-11-24.

## Context

The web view was plain JavaScript modules in `crates/priory-core/web/`, served one file each and
embedded in the binary with `include_bytes!`. `render()` rebuilds the page from HTML strings and
restores focus, caret and scroll by hand, and each missed case is a bug a person sees. The person asked for
React, a pnpm workspace in the repository, and TanStack Query as the data layer.

Today nothing in a Rust build needs Node: `cargo build`, `cargo install`, the Nix flake, the release
workflow and `install.sh` all work without it. A React app needs a bundler, so the bundle must come
from somewhere.

## Decision

- A pnpm workspace at the repository root, with one package, `@priory/web`, in
  `crates/priory-core/web/`. pnpm comes through corepack, pinned by `packageManager`.
- esbuild bundles `main.jsx`, the React shell, with `app.js` and the view models into one minified file,
  `crates/priory-core/web/dist/app.js`. The bundle is committed. `serve.rs` embeds it and serves it
  at `/app.js`.
- CI runs `corepack pnpm install --frozen-lockfile && corepack pnpm web:check`, which rebuilds the
  bundle in memory and fails when the committed file differs or passes its budget (500 KB at first,
  see the amendment).
- The service worker's shell cache gets a new name, `priory-shell-2`, and drops the older one, so an
  offline start never mixes the module files with the bundle.
- The migration goes page by page behind a React shell: each unported page renders in a
  legacy slot until its port. [web-react.md](../design/web-react.md) holds the order.

## Consequences

- A Rust build still needs no Node, nor do the flake, the release and `install.sh`.
- A change to web code needs Node and `corepack pnpm web:build` before `cargo build`. A contributor
  who forgets it fails CI, not the build.
- Two open web pull requests conflict on `dist/app.js`. Merging `main` and rebuilding resolves it.
- The view models stop being served one file each. A browser loads one script instead of 17.

## Amendment, 2026-11-25

The person decided to make the web view a full React single-page app, with TanStack Router and oxlint
beside React Query.

- `@tanstack/react-router` owns every address, with its routes in code, no plugin and no code
  splitting, so the server still serves one bundle. This reverses the note's first rejection of a router.
- oxlint runs inside `web:check`, where a warning fails. No workflow file changes.
- The budget is 720 KB while legacy and React pages coexist. The last port lowers it.
- A page reads only the listings it shows. A refresh used to read all of them on every page.

## Amendment, 2026-11-26

The port ended. No legacy slot remains, and every page, sheet and pane is a React component.

- The budget is 635 KiB: the bundle's 590,978 bytes when the port ended, plus 10%.
- The shell cache is `priory-shell-3`, and an offline start reads only that shell's copy.
- The GitHub extension follows the same rule since its React port: `@priory/github`, in
  `adapters/github/`, is the workspace's second package, and its committed `dist/frame.js` keeps the
  folder loadable unpacked with no build. `corepack pnpm github:check` fails on a stale bundle.

## Rejected

We rejected a `build.rs` that runs pnpm, since every `cargo build`, clippy run and `cargo install`
would then need Node, and the flake would need a pnpm dependency hash. We rejected a release-time
build, since a `cargo build` from a checkout would then embed no app.

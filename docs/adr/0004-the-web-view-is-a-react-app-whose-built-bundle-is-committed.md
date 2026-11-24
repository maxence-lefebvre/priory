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
  bundle in memory and fails when the committed file differs or passes 500 KB.
- The service worker's shell cache gets a new name, `priory-shell-2`, and drops the older one, so an
  offline start never mixes the module files with the bundle.
- The migration goes page by page behind a React shell: each unported page renders in a
  `LegacyPage` component until its port. [web-react.md](../design/web-react.md) holds the order.

## Consequences

- A Rust build still needs no Node, nor do the flake, the release and `install.sh`.
- A change to web code needs Node and `corepack pnpm web:build` before `cargo build`. A contributor
  who forgets it fails CI, not the build.
- Two open web pull requests conflict on `dist/app.js`. Merging `main` and rebuilding resolves it.
- The view models stop being served one file each. A browser loads one script instead of 17.

We rejected a `build.rs` that runs pnpm, since every `cargo build`, clippy run and `cargo install`
would then need Node, and the flake would need a pnpm dependency hash. We rejected a release-time
build, since a `cargo build` from a checkout would then embed no app.

# Priory on GitHub: developer notes

This folder holds the Chromium extension that adds Priory to GitHub's pull request page.
[docs/adapters/github.md](../../docs/adapters/github.md) says how to install and use it, what it
shows, and its token and `Origin` rules.

## Build

The folder is the unpacked extension, and a person who installs it runs no build. The frames are
React, bundled with esbuild into `dist/frame.js`, which is committed. After you change
`frame.jsx`, `lib/` or a web view component the frame imports, rebuild and commit the bundle:

```sh
corepack pnpm install
corepack pnpm github:build
```

`corepack pnpm github:check` fails when the committed bundle is stale, and runs oxlint. Load the
folder with Load unpacked on `chrome://extensions`, and choose the reload button on its card after
a change. The release workflow zips the folder as `priory-github-extension.zip`, without `test/`,
`node_modules/`, the `.jsx` sources and the build files.

## Test

```sh
node --test adapters/github/test/*.test.js
```

The tests cover the request builder, the pull request and guide parsers, the ask state machine,
the say routing, the store and the vendored files. They also run `content.js` on fixture pages and
the side panel's sign-in, the strip's and panel's reads, an answer sending and Generate guide in
the committed `dist/frame.js`, on the small DOM in `test/dom.js`. They need no browser.

## Files

- `manifest.json`: the extension's manifest. Its `key` fixes the extension's id.
- `content.js`: the content script on github.com. It finds the pull request, places the strip and
  the panel as iframes, and reads the guide block.
- `frame.html`, `frame.jsx`, `frame.css`: the strip and the panel, as extension pages. Every daemon
  call and all agent text live here. The frame draws the ask options with the web view's
  `AskOption`, and its icons and state tiles with the web view's `Icon` and `Tile`.
- `command.html`: the browser side panel that runs a command ask, with the same bundle.
- `dist/frame.js`: the committed bundle of `frame.jsx`, which both pages load.
- `build.mjs`: writes `dist/frame.js`, or with `--check` fails when it is stale.
- `lib/`: the modules the frame and `content.js` use. `content.js` imports `pr.js` and `page.js`
  at runtime, so they stay plain modules. `page.js` finds the tab row and the description on the
  classic and the new pull request page. `connect.jsx` is the connect form both panels show.
- `test/dom.js`: a small DOM for the tests, so `content.js` and the React frame run in Node. It
  covers only what they use.
- `test/fixtures/`: pull request pages, scrubbed to placeholders (`octo/demo`, `#1`, `alice`).
- `vendor/`: copies of the fonts and style sheets in `design/`, since an unpacked extension cannot
  reach outside its folder. A test checks that they match.
- `test/`: the Node.js tests.

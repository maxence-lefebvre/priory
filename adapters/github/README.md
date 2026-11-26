# Priory on GitHub: developer notes

This folder holds the Chromium extension that adds Priory to GitHub's pull request page.
[docs/adapters/github.md](../../docs/adapters/github.md) says how to install and use it, what it
shows, and its token and `Origin` rules.

## Build

There is no build step. The folder is the unpacked extension: load it with Load unpacked on
`chrome://extensions`, and choose the reload button on its card after a change. The release
workflow zips the folder, without `test/` and `package.json`, as `priory-github-extension.zip`.

## Test

```sh
node --test adapters/github/test/*.test.js
```

The tests cover the request builder, the pull request and guide parsers, the ask state machine,
the say routing, the store and the vendored files. They also run `content.js` on fixture pages and
the side panel's sign-in, the strip's and panel's reads and Generate guide in `frame.js`, on the small DOM in `test/dom.js`. They need no browser.

## Files

- `manifest.json`: the extension's manifest. Its `key` fixes the extension's id.
- `content.js`: the content script on github.com. It finds the pull request, places the strip and
  the panel as iframes, and reads the guide block.
- `frame.html`, `frame.js`, `frame.css`: the strip and the panel, as extension pages. Every daemon
  call and all agent text live here.
- `command.html`: the browser side panel that runs a command ask, with the same `frame.js`.
- `lib/`: the modules `frame.js` and `content.js` use. `page.js` finds the tab row and the
  description on the classic and the new pull request page. `connect.js` is the connect form both
  panels show. All but `live.js` have tests of their own.
- `test/dom.js`: a small DOM for the tests, so `content.js` and `frame.js` run in Node. It covers
  only what they use.
- `test/fixtures/`: pull request pages, scrubbed to placeholders (`octo/demo`, `#1`, `alice`).
- `vendor/`: copies of the fonts and style sheets in `design/`, since an unpacked extension cannot
  reach outside its folder. A test checks that they match.
- `test/`: the Node.js tests.

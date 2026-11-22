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

The tests cover the pure parts: the request builder, the pull request and guide parsers, the ask
state machine, the say routing, the store and the vendored files. They need no browser.

## Files

- `manifest.json`: the extension's manifest. Its `key` fixes the extension's id.
- `content.js`: the content script on github.com. It finds the pull request, places the strip and
  the panel as iframes, and reads the guide block.
- `frame.html`, `frame.js`, `frame.css`: the strip and the panel, as extension pages. Every daemon
  call and all agent text live here.
- `command.html`: the browser side panel that runs a command ask, with the same `frame.js`.
- `lib/`: the modules `frame.js` uses. All but `live.js` have tests.
- `vendor/`: copies of the fonts and style sheets in `design/`, since an unpacked extension cannot
  reach outside its folder. A test checks that they match.
- `test/`: the Node.js tests.

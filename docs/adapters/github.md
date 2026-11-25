# The GitHub extension

Priory on GitHub is a Chromium extension, for Chrome and Edge, that adds Priory to GitHub's pull
request page. On a pull request a task holds, a strip under the title shows the task, its state and
step, and the asks that wait on you. A panel docked at the right answers those asks, says to the
task's agent, and asks it to update its branch or resolve conflicts. A Guide tab walks you through
the agent's review guide and asks its quiz. It is a second view host beside the
[Herdr adapter](herdr.md).

The extension reads and writes through `priory serve`, with the same token as the web view. The
Guide needs neither: it reads the pull request's description from GitHub. The extension's code
lives in `adapters/github/`, and its [README](../../adapters/github/README.md) says how to test it.

## Install it

1. Add the extension's origin to the factory's `factory.toml` and restart the daemon or `priory serve`:

   ```toml
   [serve]
   extension_origins = ["chrome-extension://ekngedcpijdeijlmffkeehilmafoikkm"]
   ```

   The manifest's `key` fixes this id, so it stays the same on every machine and reinstall.
2. Get the extension's folder: unzip `priory-github-extension.zip` from a release (see
   [Installing a release](../installing-a-release.md#what-a-release-holds)), or use
   `adapters/github` in a clone.
3. Open `chrome://extensions` (or `edge://extensions`), turn on Developer mode, choose Load
   unpacked and pick that folder.
4. Open a pull request on github.com. The Priory strip appears above its tabs, on the classic
   page and on GitHub's new pull request page.

After a `git pull`, or after you unzip a new release into the same folder, choose the reload
button on the extension's card.

## Sign in

1. Choose Sign in on the strip. The panel opens on its connect form.
2. Enter the address `priory serve` listens on: `127.0.0.1:7340` on the same machine, or an
   `https://` address beyond it, such as the one `tailscale serve` gives (see
   [How to use the web view](../using-the-web-view.md)). Plain `http://` is refused
   beyond loopback, so the token never travels in clear.
3. Paste the token: the value of `PRIORY_TOKEN`, or of the variable `[serve] token_env` names.
4. Choose Connect, and allow the browser to let the extension reach that address.

The panel then reads the task. You can also sign in from the browser's side panel: open it, pick
Priory on GitHub, and fill in the same form. A sign-in in either place signs in every Priory frame
in the browser.

Sign out is at the bottom of the Task tab. It forgets the token and
everything the extension kept: each pull request's last read, an unsent answer and your quiz
answers. A `401` from the daemon signs you out too.

## What it shows

- **The strip**: the owning task, its state and step, and a pill when asks wait on you. On a pull
  request no active task holds, it says so, with Add a task…. When the daemon is out of reach, it
  says how old the last read is.
- **Ask**: the open asks of the task, oldest first, with the web view's order: context, question,
  recommendation, options, Write an answer, Dismiss and Snooze. Every open ask shows
  Write an answer; a step's ask and a hold's ask need an answer, so they show no Dismiss. Which asks
  accept a written answer is in [waits.md](../daemon/waits.md#asks-and-answers). An answer waits 4 s with Undo, then
  goes with the revision you saw. Leave the page or close the panel before then, and the panel
  shows the answer unsent the next time it opens on that pull request, with Send and Discard,
  until the question is asked again at a new revision. A changed question keeps your draft and
  says so. Keys 1 to 9 pick an option while the panel has focus.
- **A command ask**: a tap on an option that runs a command opens the browser's side panel with the
  command, its folder and its revision. Run it there, within 2 min of the tap. No page can cover
  the side panel. Run reads the ask again first, and runs only when the daemon, the revision and
  the command are still the ones shown.
- **Task**: the workflow grid, Update branch and Resolve conflicts, Say, the runs, and the other
  tasks on the pull request. The buttons send fixed text with `say` when the task's agent can hear
  it, and say why not otherwise. Update branch and Resolve conflicts stay off unless the task owns
  the pull request's branch, by its key or its stack. Update branch merges the base, so nothing
  needs a force push.
- **Guide**: the review guide from the description, and its quiz.

When a write loses its connection, the daemon may have applied it. The panel keeps the draft, says
so, and clears the warning once a read shows the answer, or when you send again.

The Ask tab shows no plan: read it in the web view or Herdr before you answer an ask that approves
one. See [The plan an ask approves](../daemon/asks.md#the-plan-an-ask-approves).

## If the strip is not above the tabs

When the extension finds no tab row on a pull request page, it draws the strip at the top of the
page instead, without the Guide tab. The panel still opens from the strip. After 3 s it also logs
one warning in the page's DevTools console: `Priory: no tab row found on <the page's URL>`. GitHub
changed its page: report the URL, the warning and which tab you were on.

## The review guide block

An agent writes the guide into the pull request's description, inside a collapsed block:

````markdown
<!-- priory-guide -->
<details><summary>Review guide</summary>

```json
{"version": 1, "head": "9f3c2e1", "map": ["One line on what the change does."],
 "steps": [{"title": "Back edges run down a lane", "file": "adapters/herdr/flows.js", "lines": "40-56"}],
 "quiz": {"pass": 1, "questions": [{"id": "1", "question": "...", "options": ["...", "..."],
   "answer": "sha256:3f1c9a0e5b7d2c44", "cite": "crates/priory-core/src/flows.rs:88"}]}}
```

</details>
<!-- /priory-guide -->
````

GitHub drops the HTML comments from the page, so the extension finds the block by its summary,
"Review guide". Agents find it in the raw body by the markers.

| Field | Rule |
| ----- | ---- |
| `version` | `1` |
| `head` | The commit the guide describes, 7 to 40 hex digits |
| `map` | Up to 4 lines on what the change does |
| `steps` | Up to 8, each with `title`, `file` and optional `lines`, `N` or `N-M` |
| `quiz` | Optional. `pass`, then 1 to 4 `questions`, each with `id`, `question`, 2 to 6 `options`, `answer` and optional `cite` |
| `answer` | `sha256:` and the first 16 hex digits of SHA-256 of `<owner>/<repo>#<n>:<id>:<option>` |

The block stays under 8 KiB, and every string under 500 characters. A score belongs to the pull
request's head commit and the guide's text: a push or a new guide resets it. The quiz changes
nothing on GitHub, and anyone can hash the options: it keeps the reviewer honest, nothing more.

## Security model

The extension holds the daemon's token, which can answer, say, cancel and revive. These rules keep
it out of GitHub's page:

- **Two origins.** The content script runs in GitHub's page. It finds the pull request, places two
  iframes and reads the guide block. Every daemon call and all agent text live in the iframes,
  which are extension pages in `chrome-extension://<id>`. A script on github.com cannot read them.
- **The token** lives in the extension origin's IndexedDB, which content scripts and GitHub's pages
  cannot open. It is sent only in the `Authorization` header, only to the address you gave, and only
  on a path under `/api`. It is stored in plain text in the browser profile, as the web view's is.
- **Origin.** `serve` refuses a `POST` whose `Origin` differs from its `Host`, which an extension
  page's always does. [`[serve] extension_origins`](../reference/factory.md#serve) lets this one
  extension write, as [serve.md](../daemon/serve.md) explains. A web page cannot
  send another origin's `Origin`, and the token still guards every call.
- **CORS.** The extension asks for a host permission for the one address you entered, which lets its
  pages skip the browser's CORS checks. `serve` needs no CORS headers.
- **Messages.** The frames talk to the content script with `chrome.tabs.sendMessage`, never
  `postMessage`, and ignore every message from the page.

What a script on github.com can and cannot do:

| It can | It cannot |
| ------ | --------- |
| See that the extension is installed: the iframes are in its DOM | Read the token, the task or the asks |
| Remove, move or cover the iframes | Call the daemon with the token |
| Fake a guide block: it is page content anyway, shown as text | Send a write without your taps in the panel |

A hostile page could lay out or cover the iframes so your taps land on an option you did not mean.
On github.com that needs GitHub itself compromised, and the 4 s Undo shows what was sent. A command
runs only from the browser's side panel, which no page can cover.

## Limits

- It runs on github.com only, not on GitHub Enterprise Server, and in Chromium browsers from
  version 116: Chrome and Edge. Firefox and Safari are not supported.
- It is not in the Chrome Web Store. You load it unpacked, with Developer mode on.
- It signs in to one `priory serve` address at a time, and shows the tasks that daemon reads.
- Beyond loopback, the address must be `https://`.
- A command ask runs only from the browser's side panel, within 2 min of the tap.
- The quiz is a reading aid, not a gate: it changes nothing on GitHub.

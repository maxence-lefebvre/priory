# Bug investigation

When someone adds the label `bug` to an issue of `owner/repo`, an agent reproduces the bug, finds
its root cause, fixes it with a test, and asks you before it opens the fix pull request.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB`
set for the intake and the daemon. Replace `owner/repo` in `triggers/bug-label.toml`, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- `approve-pr` asks you before `open-pr` pushes the branch and opens the pull request.
- A bug the agent cannot reproduce ends the task at `investigate`, with nothing pushed.

## Run it

```sh
cp -r examples/bug-investigation my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

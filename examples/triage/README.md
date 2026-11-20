# Triage

When someone opens an issue in `owner/repo`, an agent labels it. If the issue lacks the steps,
the expected result, what happened or the version, the agent drafts a comment that asks for it, and
asks you before it posts it.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB`
set for the intake and the daemon. Replace `owner/repo` in `triggers/new-issues.toml`, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- `triage` adds labels, which can be taken off, so it is `reversible`. It never creates a label.
- `approve-comment` asks you before `post-comment` posts the drafted comment. `skip` ends the
  task with the labels only.

## Run it

```sh
cp -r examples/triage my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

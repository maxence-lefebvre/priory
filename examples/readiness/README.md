# Readiness

`priory add` starts a check of `owner/repo`. An agent runs `priory check --repo . --json` in the
repository and drafts one issue for each gap a pull request can close, such as a missing
`AGENTS.md` or a CI workflow. Once you approve, it files them with the label `readiness`. Each
labelled issue starts a fix task that ends on a pull request for you to review.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB` set for
the intake and the daemon. The agent runs `priory`, so put Priory's binary on the daemon's `PATH`.
Create the label `readiness` in the repository. Replace `owner/repo` in both files of `triggers/`,
and rename `runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- `check` only reads: `priory check --repo` changes nothing, and the agent's permissions deny file
  edits.
- `approve-issues` asks you before `file-issues` files the drafts. `skip` ends the task with the
  drafts in its result.
- Gaps that only you can close, such as required checks in the repository's settings or the
  factory's `gate`, get no issue. The result lists them with their fix.
- `open-pr` is `autonomous`: it opens the pull request without an ask. Nothing here merges, so a
  change lands only when a person merges it.

## Run it

```sh
priory check --repo ../my-project   # the report alone, before any task runs
cp -r examples/readiness my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
priory add my-factory "Check the repository's readiness"
```

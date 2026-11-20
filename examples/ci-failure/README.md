# CI failure

Every hour on workdays, from 09:00 to 18:00, an agent looks for failed checks on the default branch
and on open pull requests of `owner/repo`. It finds the cause of one, fixes it, and asks you before
it opens the fix pull request.

No trigger fires on a failed check yet, so a schedule starts each look. A look that finds nothing to
fix ends as done.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB`
set for the intake and the daemon. Replace `owner/repo` in `triggers/hourly-checks.toml`, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- `approve-pr` asks you before `open-pr` pushes the branch and opens the pull request.
- The agent's permissions deny `git push`, `gh pr create` and `gh run rerun`.

## Run it

```sh
cp -r examples/ci-failure my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

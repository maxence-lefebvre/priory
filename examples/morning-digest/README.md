# Morning digest

At 08:30 on workdays, an agent lists the factory's open asks, its blocked, failed and held tasks,
and the pull requests of `owner/repo` merged yesterday. It posts each line to your desk with
`priory post digest`, and changes nothing else. It keeps what it listed on its trigger's journal,
so the next morning marks what is new on any runner. See [journals.md](../../docs/journals.md).

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB`
set for the intake and the daemon. The agent runs `priory`, so put Priory's binary on the daemon's `PATH`. Replace
`owner/repo` in `triggers/weekday-morning.toml`, and rename `runners/my-machine.toml` after
your machine's Tailscale hostname.

## Gates

- No step is irreversible, so it asks you nothing. Its only writes are report lines on your own
  desk, which `priory inbox read SEQ` marks read.
- The agent's permissions deny file edits, `git push` and `gh api`.

## Run it

```sh
cp -r examples/morning-digest my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

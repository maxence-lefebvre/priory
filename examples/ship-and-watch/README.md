# Ship and watch

When someone adds the label `ship` to an issue of `owner/repo`, an agent implements it and Priory
opens the pull request. The task does not end there. Once a person merges the pull request, the
agent watches the release for an hour, then drafts an announcement for the team channel, which
posts only after you approve it.

| Step | What happens |
| ---- | ------------ |
| `implement` | The agent makes the change, tests it and commits. |
| `open-pr` | Priory opens the pull request. Its checks and reviews wake `implement`, as they do before `done`. |
| `watch` | `after = "merged"`: the task enters it only once the pull request merged. The agent looks every 10 minutes for an hour, sleeping on a `time` wait between looks. |
| `look` | Only a problem the watch found reaches your desk. You pick `announce` or `drop`. |
| `draft` | The agent writes the announcement in its result, or ends with `none` when the change does not alter how people work. |
| `approve-announcement` | You read the draft and pick `post` or `skip`. |
| `announce` | The agent posts the approved message to Slack. |

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, a Slack token or
webhook the agent can call, and `PRIORY_DB` set for the intake and the daemon. Replace
`owner/repo` in `triggers/ship-label.toml`, and rename `runners/my-machine.toml` after your
machine's Tailscale hostname.

## Gates

- Nothing here merges. The watch starts only after a person merged the pull request.
- A quiet watch puts nothing on your desk. Its result's summary records what it checked.
- `announce` is `irreversible` and `approve-announcement` approves it. Its
  `guard = { allow = ["slack-write"] }` opens Slack writes in that step only, so a post from `draft`
  or `watch` is denied.
- The watch's cadence and its hour come from its prompt. The agent counts the window, so a person
  can still `priory cancel` a watch that runs long.

## Run it

```sh
cp -r examples/ship-and-watch my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

`e2e/merged.toml` plays the whole path with fake agents: see
[testing-a-factory.md](../../docs/testing-a-factory.md).

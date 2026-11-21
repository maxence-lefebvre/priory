# Groom

When someone adds the label `groom` to an issue of `owner/repo`, an agent reads it, grades it red,
yellow or green with a one-line reason, and writes the brief. `priory tasks` shows the grade.

| Grade | Then |
| ----- | ---- |
| green | It implements the brief, grades the merge risk, describes the change and opens the pull request. |
| yellow | It writes its plan into the task, implements, gets an independent review pass, grades the merge risk, describes the change, then opens the pull request. |
| red | It writes its plan and waits for you to approve it, then goes on as yellow. |

The groom step also sends a `goal`, a `testing` method, a `brief` (what to change and why) and a
`verification` (the commands or checks that prove it). The approval ask shows all four,
`priory task <id> --json` stores them as `plan_goal`, `plan_testing`, `plan_brief` and
`verification`, and the implement steps' prompts name the verification. The step's `plan_requires`
makes `priory check` refuse the workflow if its prompt and agent stop asking for them.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` and `codex` on `PATH`, and
`PRIORY_DB` set for the intake and the daemon. Replace `owner/repo` in `triggers/groom-label.toml`, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- Every grade ends on a pull request for you to review. Nothing here merges, so a change lands only
  when a person merges it.
- `open-pr` is `autonomous`: it opens the pull request without an ask. The task ends as done once
  the pull request's checks pass, before anyone reviews it.
- The review pass runs on another agent, `reviewer`, on Codex. It reads only the brief in
  `.priory/brief.md` and the diff, and writes its findings to `.priory/review.md` for the
  implementer. Neither file is committed. Any second model or harness works as the reviewer.
- Before the pull request opens, a third agent, `risk`, grades the finished change's merge risk
  low, medium or high on five dimensions, one line each, and writes them to `.priory/risk.md`.
  `priory task <id> --json` shows the verdict as `merge_risk`, and the pull request body ends with
  it, so you see the risk before the diff.
- The implementer's `describe` step then writes the pull request's title and body.
- `approve-plan` holds a red task until you answer `approved` or `rejected`.

## Run it

```sh
cp -r examples/groom my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

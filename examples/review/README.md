# Review

When someone asks you to review a pull request, in any repository `gh` can read, an agent grades its
merge risk. Then another agent reads it and the ticket it links to, runs the checks if they are
cheap, and writes a pending GitHub review.
Only you see a pending review. You edit it and submit it on GitHub.
The reviewer writes its inline comments with the skill in `skills/review-comments/`, which Priory
gives every run as `priory:review-comments`.

## What it needs

`gh` logged in to GitHub as the reviewer, so the pending review is yours, `claude` on `PATH`, and
`PRIORY_DB` set for the intake and the daemon. Replace `your-login` in
`triggers/review-requests.toml` with that GitHub login, and rename `runners/my-machine.toml` after
your machine's Tailscale hostname.

## Gates

- The agent never submits, approves or requests changes. Its prompt forbids it, and its permissions
  deny `gh pr review` and `gh pr merge`.
- The `review` step is `reversible`: a pending review stays invisible and can be deleted.
- The workflow allows the guard `gh-api-write`, so the agent can create the pending review with
  `gh api`. A merge or an approval through `gh api` is still denied: it needs an approved step.
- First, a second agent, `risk`, grades the pull request's merge risk low, medium or high on five
  dimensions, one line each. It reads only: it never comments on or reviews the pull request.
- `submit` asks you to submit the review on GitHub, then to answer `submitted` or `discarded`.
  `priory asks` shows the merge risk with it, and `priory asks --risk high` lists only the risky
  ones.

## Run it

```sh
cp -r examples/review my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

# Which threads and issues the intake watches

The intake reads open waits on `linear_comment`, `linear_state`, `slack_reply`, `pull_request`,
`checks` and `review` from the state file `PRIORY_DB` every poll, and the open `--merge` holds
(see Holds in `docs/daemon/runs.md`). Without `PRIORY_DB`, it watches none. It skips the waits and
holds of a task that is `done`, `failed` or `dropped`. Each wait gives a resource and its agent's
last prompt time. A hold gives its pull request and the time it was set.

- For a pull request, waited on or held for, the intake polls its repository as it polls a trigger's (see
  `github-polling.md`). A repository that no trigger names starts with a baseline, whose reviews and
  checks still reach the wait. A restart resumes it from where it stopped, as long as a wait still
  names it.
- For each Slack thread, the intake reads `conversations.replies` from that time on, every poll,
  until the wait fires or is withdrawn. herdr-bus drops replies it already holds.
- For a Linear issue, the intake starts the issues and comments streams when no Linear trigger
  has. The streams read, in two queries, every issue updated and comment created in the workspace
  since their position, so it never polls one issue at a time. Their first poll is a baseline, but
  its events still reach the wait: the daemon judges events by time, not by `baseline`.

A missing key fails only that source's poll, so a wait cannot stop GitHub polling.

# Which threads and issues the intake watches

The intake reads the open waits on `linear_comment`, `linear_state`, `slack_reply`, `pull_request`,
`checks` and `review` from the state file `PRIORY_DB` on every poll; without `PRIORY_DB` it watches
none. It skips the waits of a task that is `done`, `failed` or `dropped`. Each wait gives a resource
and the time of its agent's last prompt.

- For a pull request, the intake polls its repository as it polls a trigger's (see
  `github-polling.md`). A repository that no trigger names starts with a baseline, whose reviews and
  checks still reach the wait.

- For each Slack thread, the intake reads `conversations.replies` from that time on, every poll,
  until the wait fires or is withdrawn. herdr-bus drops the replies it already holds.
- For a Linear issue, the intake starts the issues and comments streams when no Linear trigger has.
  Those streams read every issue updated and every comment created in the workspace since their
  position, in two queries, so the intake never polls one issue at a time. Their first poll is a
  baseline, whose events still reach the wait: the daemon judges an event by its time, not by
  `baseline`.

A missing key fails only the poll of that source, so a wait cannot stop the GitHub polling.

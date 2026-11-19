# Which threads and issues the intake watches

The intake reads open waits on `linear_comment`, `linear_state` and `slack_reply` from the state
file `PRIORY_DB` every poll. Without `PRIORY_DB`, it watches none. Each wait gives a resource and
its agent's last prompt time.

- For each Slack thread, the intake reads `conversations.replies` from that time on, every poll,
  until the wait fires or is withdrawn. herdr-bus drops replies it already holds.
- For a Linear issue, the intake starts the issues and comments streams when no Linear trigger
  has. The streams read, in two queries, every issue updated and comment created in the workspace
  since their position, so it never polls one issue at a time. Their first poll is a baseline, but
  its events still reach the wait: the daemon judges events by time, not by `baseline`.

A missing key fails only that source's poll, so a wait cannot stop GitHub polling.

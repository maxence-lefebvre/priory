+++
description = "Labels a new issue and drafts a comment that asks for what is missing."
harness = "claude-code"
model = "haiku"
memory = "4GB"
permissions = { defaultMode = "auto", deny = ["Edit", "Write", "Bash(git push:*)", "Bash(gh issue close:*)"] }
+++
# Triage

You triage one new GitHub issue.

- Read the issue and the repository's existing labels with `gh label list`.
- Add the labels that fit with `gh issue edit --add-label`. Never create a label.
- Check that the issue has what a person needs to act on it: the steps, what was expected, what
  happened, and the version.
- When something is missing, draft one short, polite comment that asks for it. Put the draft in
  your result. Post it only in the `post-comment` step, after a person approved it.

//! The messages the intake publishes on `factory.intake`, as the schemas in `docs/schemas/events/`
//! describe them. The publishers build the same fields with `json!`; the intake tests validate
//! what they publish against these schemas.

use priory_factory::factory::Hold;
use schemars::JsonSchema;

/// A message on `factory.intake`: a herdr-bus message shaped like a CloudEvent.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Envelope<Data> {
    /// A UUIDv5 of the message's name, so a retried publish keeps its id.
    #[schemars(extend("format" = "uuid"))]
    pub id: String,
    /// The message type, such as `github.issues.labeled`.
    #[serde(rename = "type")]
    pub kind: String,
    /// A short line for listings, such as `owner/name#42`.
    pub subject: String,
    /// When the event happened, RFC 3339.
    #[schemars(extend("format" = "date-time"))]
    pub time: String,
    pub data: Data,
    /// GitHub and Linear events carry one link to the event's page.
    pub links: Vec<Link>,
}

/// A link for people.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// `issue` or `pull_request`.
    pub label: String,
    pub url: String,
}

/// The fields every GitHub, Linear and Slack event ends with.
#[derive(JsonSchema)]
pub struct Source {
    /// When the event happened, RFC 3339.
    #[schemars(extend("format" = "date-time"))]
    pub time: String,
    /// True when the event came from the first poll of its stream.
    pub baseline: bool,
    /// The triggers the event matched, so the triggers it requests tasks for; may be empty.
    pub triggers: Vec<String>,
}

/// A GitHub issue labeled, from the repository's issue events.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueLabeled {
    /// `owner/name`.
    pub repo: String,
    /// The issue number.
    pub issue: u64,
    /// The issue title when the intake saw the event.
    pub title: String,
    /// The issue's page on GitHub.
    pub url: String,
    /// The label added.
    pub label: Option<String>,
    /// The GitHub login who added the label; null for a deleted user.
    pub actor: Option<String>,
    /// GitHub's issue event id; null from a webhook, whose payload lacks it.
    pub github_id: Option<u64>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A GitHub issue opened, from the repository's issues list.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueOpened {
    /// `owner/name`.
    pub repo: String,
    /// The issue number.
    pub issue: u64,
    /// The issue title when the intake polled, which may differ from the title it opened with.
    pub title: String,
    /// The issue's page on GitHub.
    pub url: String,
    /// The GitHub login who opened it; null for a deleted user.
    pub actor: Option<String>,
    /// GitHub's issue id.
    pub github_id: u64,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A GitHub issue closed, from the repository's issue events.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueClosed {
    /// `owner/name`.
    pub repo: String,
    /// The issue number.
    pub issue: u64,
    /// The issue title when the intake polled.
    pub title: String,
    /// The issue's page on GitHub.
    pub url: String,
    /// The GitHub login who closed it; null for a deleted user.
    pub actor: Option<String>,
    /// GitHub's issue event id.
    pub github_id: u64,
    #[serde(flatten)]
    pub source: Source,
}

/// A comment on a GitHub issue, from the repository's issue comments; bots' comments are skipped.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueComment {
    /// `owner/name`.
    pub repo: String,
    /// The issue number.
    pub issue: u64,
    /// The comment's page on GitHub.
    pub url: String,
    /// The GitHub login who commented; null for a deleted user.
    pub actor: Option<String>,
    /// The comment's Markdown, as written.
    pub body: String,
    /// GitHub's comment id.
    pub github_id: u64,
    /// GitHub's `author_association` of the author, such as `OWNER`, `COLLABORATOR` or `NONE`.
    /// Feedback from anyone but the owner, a member or a collaborator reaches no task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub association: Option<String>,
    /// Whether the login `gh api user` names wrote it, the login a worker posts as; absent while
    /// `gh` cannot say. A comment another login wrote wakes its task even when it was written
    /// during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    /// Whether an agent wrote it: `own`, and its body holds `<!-- agent-comment -->`; absent while
    /// `gh` cannot say. An agent's comment starts no task. The person's unmarked one wakes its task
    /// even when it was written during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    #[serde(flatten)]
    pub source: Source,
}

/// The pull request every pull request event names, read when the intake polled.
#[derive(JsonSchema)]
pub struct PullRequest {
    /// `owner/name`.
    pub repo: String,
    /// The pull request number.
    pub pull_request: u64,
    /// The pull request's page; the review's for a review, the check run's for a check.
    pub url: String,
    /// The head branch; null when GitHub no longer returns the pull request.
    pub head_branch: Option<String>,
    /// The head commit; for a check, the commit it ran on.
    pub head_sha: Option<String>,
    /// The branch the pull request merges into.
    pub base_branch: Option<String>,
    /// The head branch's repository, `owner/name`, which differs from `repo` for a fork; absent
    /// when the event does not say, as for a check run or a deleted fork.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_repo: Option<String>,
}

/// A pull request opened.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestOpened {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The pull request title when the intake polled.
    pub title: String,
    /// The GitHub login who opened it.
    pub actor: Option<String>,
    /// GitHub's issue id of the pull request; null from a webhook, whose payload lacks it.
    pub github_id: Option<u64>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A label added to a pull request.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestLabeled {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The pull request title when the intake polled.
    pub title: String,
    /// The label added.
    pub label: Option<String>,
    /// The GitHub login who added the label.
    pub actor: Option<String>,
    /// GitHub's issue event id; null from a webhook, whose payload lacks it.
    pub github_id: Option<u64>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A pull request closed, merged or not. No trigger starts a task from it.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestClosed {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The pull request title when the intake polled.
    pub title: String,
    /// Whether this close merged it: the pull request is merged and this is its last close.
    pub merged: Option<bool>,
    /// The commit the merge made on the base branch; null unless `merged`.
    pub merge_commit: Option<String>,
    /// The pull request description when the intake read it; empty when it has none.
    pub body: String,
    /// The GitHub login who closed it.
    pub actor: Option<String>,
    /// GitHub's issue event id; null from a webhook, whose payload lacks it.
    pub github_id: Option<u64>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A review submitted on a pull request; pending reviews are skipped.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewSubmitted {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// `approved`, `changes_requested`, `commented` or `dismissed`, at poll time.
    pub state: String,
    /// The review's summary Markdown, as written; empty when the reviewer wrote none.
    pub body: String,
    /// The commit the review was written on, which `head_sha` passes once a commit is pushed after it.
    pub commit: Option<String>,
    /// The GitHub login who reviewed.
    pub actor: Option<String>,
    /// True when a bot reviewed; absent otherwise. A trigger starts a task from a bot's review
    /// only when its `commenter` filter names the bot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot: Option<bool>,
    /// The GitHub login who opened the pull request; absent when GitHub does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Whether the pull request was open when the intake read it; a done task reopens only
    /// for feedback on an open one.
    pub open: bool,
    /// GitHub's review id.
    pub github_id: u64,
    /// GitHub's `author_association` of the author, such as `OWNER`, `COLLABORATOR` or `NONE`.
    /// Feedback from anyone but the owner, a member or a collaborator reaches no task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub association: Option<String>,
    /// Whether the login `gh api user` names wrote it, the login a worker posts as; absent while
    /// `gh` cannot say. A comment another login wrote wakes its task even when it was written
    /// during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    /// Whether an agent wrote it: `own`, and its body holds `<!-- agent-comment -->`; absent while
    /// `gh` cannot say. An agent's comment starts no task. The person's unmarked one wakes its task
    /// even when it was written during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A comment on a pull request's conversation; bots' comments are skipped.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestComment {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The GitHub login who commented; null for a deleted user.
    pub actor: Option<String>,
    /// The GitHub login who opened the pull request; absent when GitHub does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Whether the pull request was open when the intake read it; a done task reopens only
    /// for feedback on an open one.
    pub open: bool,
    /// The comment's Markdown, as written.
    pub body: String,
    /// GitHub's comment id.
    pub github_id: u64,
    /// GitHub's `author_association` of the author, such as `OWNER`, `COLLABORATOR` or `NONE`.
    /// Feedback from anyone but the owner, a member or a collaborator reaches no task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub association: Option<String>,
    /// Whether the login `gh api user` names wrote it, the login a worker posts as; absent while
    /// `gh` cannot say. A comment another login wrote wakes its task even when it was written
    /// during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    /// Whether an agent wrote it: `own`, and its body holds `<!-- agent-comment -->`; absent while
    /// `gh` cannot say. An agent's comment starts no task. The person's unmarked one wakes its task
    /// even when it was written during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A comment in a review thread on a pull request's diff; bots' comments are skipped.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewThreadComment {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The GitHub login who commented; null for a deleted user.
    pub actor: Option<String>,
    /// The GitHub login who opened the pull request; absent when GitHub does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Whether the pull request was open when the intake read it; a done task reopens only
    /// for feedback on an open one.
    pub open: bool,
    /// The comment's Markdown, as written.
    pub body: String,
    /// The file the thread is on.
    pub path: String,
    /// The line of the diff's new side the comment is on; null once the line is outdated.
    pub line: Option<u64>,
    /// The thread: the id of its first comment.
    pub thread: u64,
    /// GitHub's comment id.
    pub github_id: u64,
    /// GitHub's `author_association` of the author, such as `OWNER`, `COLLABORATOR` or `NONE`.
    /// Feedback from anyone but the owner, a member or a collaborator reaches no task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub association: Option<String>,
    /// Whether the login `gh api user` names wrote it, the login a worker posts as; absent while
    /// `gh` cannot say. A comment another login wrote wakes its task even when it was written
    /// during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    /// Whether an agent wrote it: `own`, and its body holds `<!-- agent-comment -->`; absent while
    /// `gh` cannot say. An agent's comment starts no task. The person's unmarked one wakes its task
    /// even when it was written during the agent's turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    /// The `X-GitHub-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// GitHub does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A check run completed on a pull request's head commit, once per pull request.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckRunCompleted {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// The check run's name, as branch protection names a required check.
    pub name: Option<String>,
    /// `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, `action_required`,
    /// `stale` or `startup_failure`.
    pub conclusion: Option<String>,
    /// The slug of the GitHub App that ran the check.
    pub app: Option<String>,
    /// Check runs on the same commit not completed at the poll, counting those past the first 100.
    pub pending: u64,
    /// Check runs on the same commit completed with a conclusion other than success, neutral or
    /// skipped, leaving out a cancelled run that `superseded_by` passed.
    pub failing: u64,
    /// GitHub's check run id.
    pub github_id: u64,
    /// For a cancelled run on a merge commit: the later commit of the base branch whose run of the
    /// same name succeeded, so the cancelled run counts as passing. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A snapshot of a pull request head's check runs: the first time a poll finds any, or none past
/// the 5-minute grace period.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckRuns {
    #[serde(flatten)]
    pub pull: PullRequest,
    /// How many check runs the head had at the poll.
    pub runs: u64,
    /// When GitHub last updated the pull request, before the poll.
    #[schemars(extend("format" = "date-time"))]
    pub updated_at: String,
    #[serde(flatten)]
    pub source: Source,
}

/// An open pull request that asks a person for a review, as GitHub's search lists it.
#[derive(JsonSchema)]
pub struct RequestedPull {
    /// `owner/name`.
    pub repo: String,
    /// The pull request number.
    pub pull_request: u64,
    /// The pull request title when the intake polled.
    pub title: String,
    /// The pull request's page on GitHub.
    pub url: String,
    /// The GitHub login who opened it; null for a deleted user.
    pub author: Option<String>,
    /// When the intake first found the request: the time of that poll.
    #[schemars(extend("format" = "date-time"))]
    pub since: String,
}

/// A pull request that asks a person for a review and did not at the previous poll.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequested {
    #[serde(flatten)]
    pub pull: RequestedPull,
    /// The GitHub login asked for the review, as the trigger names it.
    pub reviewer: String,
    #[serde(flatten)]
    pub source: Source,
}

/// A pull request that left a person's review requests since the previous poll: they reviewed it,
/// someone removed the request, or it closed.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequestRemoved {
    #[serde(flatten)]
    pub pull: RequestedPull,
    /// The GitHub login asked for the review, as the trigger names it.
    pub reviewer: String,
    /// Whether the reviewer submitted a review since `since`. False means the request ended some
    /// other way: someone removed it, or the pull request closed.
    pub reviewed: bool,
    #[serde(flatten)]
    pub source: Source,
}

/// The fields of a Linear issue every Linear event carries, as the intake saw it.
#[derive(JsonSchema)]
pub struct LinearIssue {
    /// The issue identifier, such as `ENG-7`.
    pub issue: String,
    /// Linear's id of what the event is about: the issue, history entry or comment. A webhook's
    /// state change has no history entry, so it holds the issue's id.
    pub linear_id: String,
    /// The issue title.
    pub title: String,
    /// The issue's page, or the comment's for a comment.
    pub url: String,
    /// The issue's team key.
    pub team: String,
    /// The issue's project name; null without one.
    pub project: Option<String>,
    /// Linear's id of the issue's project; null without one.
    pub project_id: Option<String>,
    /// The issue's state name.
    pub state: String,
    /// The issue's label names.
    pub labels: Vec<String>,
}

/// Where a Linear issue stands in its project, as the intake saw it.
#[derive(JsonSchema)]
pub struct IssueOrder {
    /// Linear's id of the project.
    pub project_id: String,
    /// The issue's milestone; null without one.
    pub milestone: Option<MilestoneOrder>,
    /// The issue's manual order in Linear. Lower comes first.
    pub sort_order: f64,
}

/// Where a milestone stands in its project.
#[derive(JsonSchema)]
pub struct MilestoneOrder {
    /// Linear's id of the milestone.
    pub id: String,
    /// The milestone's manual order in its project. Lower comes first.
    pub sort_order: f64,
    /// The milestone's target date, `YYYY-MM-DD`; null without one.
    pub target_date: Option<String>,
}

/// An issue that entered or left a saved Linear view, or a Linear project's issues that a trigger's
/// filter keeps, since the previous poll.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewChanged {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The view, as the trigger names it. View events only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// The stream, `linear.project:<project>`, then `:` and the trigger's filter as JSON when it has
    /// one. Project events only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// Where the issue stands in its project; null without a project, and absent from an issue the
    /// intake last read before it kept the order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<IssueOrder>,
    #[serde(flatten)]
    pub source: Source,
}

/// A saved Linear view whose filter changed since the previous poll. The issues the change brought
/// in or took out publish no `linear.issue.entered` or `linear.issue.left` and request no task.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewFilterChanged {
    /// The view, as the trigger names it.
    pub view: String,
    /// The identifiers of the issues now in the view that were not in it at the previous poll.
    pub entered: Vec<String>,
    /// The identifiers of the issues in the view at the previous poll that are not in it now.
    pub left: Vec<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A Linear issue created.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearIssueCreated {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The Linear display name of who created it.
    pub actor: Option<String>,
    /// The identifiers of the open issues that block it; null past 20 relations.
    pub blockers: Option<Vec<String>>,
    /// The `Linear-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// Linear does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// An entry of a Linear issue's history that moves it from one state to another.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearStateChanged {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The state it moved from; null from a webhook, whose payload names only its id.
    pub from: Option<String>,
    /// The state it moved to.
    pub to: String,
    /// The type of the state it moved to: `triage`, `backlog`, `unstarted`, `started`,
    /// `completed` or `canceled`. Linear's Duplicate state is of type `canceled`.
    pub to_type: Option<String>,
    /// The identifier of the issue it duplicates, as its `duplicate` relation names it; null
    /// without one and from a webhook.
    pub duplicate_of: Option<String>,
    /// The Linear display name of who moved it.
    pub actor: Option<String>,
    /// The identifiers of the open issues that block it; null past 20 relations.
    pub blockers: Option<Vec<String>>,
    /// The `Linear-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// Linear does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// An entry of a Linear issue's history that edits its title, description, labels, assignee or
/// priority, or moves it to another team, for an issue a task owns or subscribes to. It never
/// names a trigger.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearIssueUpdated {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The fields the entry changed, of `title`, `description`, `labels`, `assignee` and
    /// `priority`, or `identifier` alone when it moved the issue to another team.
    pub changed: Vec<String>,
    /// The identifier the issue had before it moved to another team, which a task watched it by,
    /// when `changed` is `identifier`; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_issue: Option<String>,
    /// The first 200 characters of the issue's description when the entry changed it, as the
    /// intake read it, not as the entry left it: Linear records no description text in history.
    /// Null otherwise.
    pub description: Option<String>,
    /// The Linear display name of who edited it.
    pub actor: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A Linear issue whose blocker changed, since Linear does not update an issue when its blocker
/// closes.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearBlockers {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The identifiers of the open issues that block it; null past 20 relations.
    pub blockers: Option<Vec<String>>,
    /// The identifier of the blocker whose change published the event.
    pub blocker: String,
    #[serde(flatten)]
    pub source: Source,
}

/// A comment on a Linear issue.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearCommentCreated {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The comment, in Markdown.
    pub body: String,
    /// The Linear display name of who commented.
    pub actor: Option<String>,
    /// The `Linear-Delivery` of the webhook delivery that reported it; absent when a poll did.
    /// Linear does not sign it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A Sentry issue created, from a signed webhook delivery.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SentryIssueCreated {
    /// Sentry's issue id, such as `1234567890`.
    pub issue: String,
    /// The issue's short id, such as `PYTHON-Y`.
    pub short_id: String,
    /// The issue title.
    pub title: String,
    /// Where the error happened, such as a function; null when Sentry names none.
    pub culprit: Option<String>,
    /// The issue's page on Sentry.
    pub url: String,
    /// The slug of the issue's project.
    pub project: String,
    /// The issue's level, such as `error` or `fatal`; null when Sentry names none.
    pub level: Option<String>,
    /// The issue's category, such as `error`, `outage` or `feedback`; null when Sentry names none.
    pub category: Option<String>,
    /// Who created it: a user's name, an integration's, or `Sentry`.
    pub actor: Option<String>,
    /// The `Request-ID` of the delivery. Sentry does not sign it.
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A comment on a Figma file, from a webhook delivery with the right passcode.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FigmaFileCommentCreated {
    /// The file key, such as `zH44k2FUM629Fa4EMShiHL`.
    pub file: String,
    /// The file name; null when Figma names none.
    pub file_name: Option<String>,
    /// Figma's comment id.
    pub comment: String,
    /// The comment's text, each mention written as `@<handle>`.
    pub text: String,
    /// The id of the comment this one replies to; null for a top-level comment.
    pub parent: Option<String>,
    /// The handle of the person who commented; null when Figma names none.
    pub author: Option<String>,
    /// The handles of the people the comment mentions.
    pub mentions: Vec<String>,
    /// The file's page on Figma.
    pub url: String,
    /// The id of the webhook that delivered it.
    pub webhook: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A CloudWatch alarm's state change, from an EventBridge API destination.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloudwatchAlarmStateChanged {
    /// The alarm's ARN, such as `arn:aws:cloudwatch:us-east-1:123456789012:alarm:ServerCpuTooHigh`.
    pub alarm: String,
    /// The alarm's name.
    pub name: String,
    /// The alarm's description; null when it has none.
    pub description: Option<String>,
    /// The new state: `ALARM`, `OK` or `INSUFFICIENT_DATA`.
    pub state: String,
    /// The state before; null when the event names none.
    pub previous_state: Option<String>,
    /// Why the state changed, in CloudWatch's words; null when the event gives none.
    pub reason: Option<String>,
    /// The AWS account id.
    pub account: Option<String>,
    /// The AWS region, such as `us-east-1`.
    pub region: Option<String>,
    /// The EventBridge event id, which stays the same across EventBridge's retries.
    pub delivery: String,
    #[serde(flatten)]
    pub source: Source,
}

/// A contact started an Intercom conversation, from a signed webhook delivery.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntercomConversationCreated {
    /// Intercom's conversation id, such as `1295`.
    pub conversation: String,
    /// The workspace id, the notification's `app_id`.
    pub workspace: String,
    /// The conversation's title; null when it has none.
    pub title: Option<String>,
    /// The first message's subject, as on an email; null when it has none.
    pub subject: Option<String>,
    /// The first message's text: plain text from API version 2.15, HTML before it.
    pub body: Option<String>,
    /// The name of whoever wrote the first message.
    pub author: Option<String>,
    /// The author's type, such as `user` or `lead`.
    pub author_type: Option<String>,
    /// The page where the contact started the conversation; null for email and bots.
    pub page: Option<String>,
    /// The notification's id, which starts with `notif_`. Intercom signs it with the body.
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A Honeycomb Trigger fired, from a webhook delivery that carried the Shared Secret.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoneycombTriggerFired {
    /// Honeycomb's Trigger id, such as `JwwZxfcP5kH`.
    pub trigger: String,
    /// The Trigger's name.
    pub name: String,
    /// The Trigger's description; null when it has none.
    pub description: Option<String>,
    /// The Trigger's page on Honeycomb.
    pub url: String,
    /// The Honeycomb environment the Trigger belongs to; null when the payload names none.
    pub environment: Option<String>,
    /// Honeycomb's one-line summary of the alert.
    pub summary: Option<String>,
    /// The id of this firing, the same across Honeycomb's retries.
    pub instance: String,
    /// The query result that fired the Trigger.
    pub result_url: Option<String>,
    /// The `X-Honeycomb-Webhook-Delivery-ID` of the delivery.
    pub delivery: Option<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A message posted in a Slack trigger's channel, outside a thread.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlackPosted {
    /// The channel id.
    pub channel: String,
    /// Slack's id of the message: seconds since the epoch, with microseconds.
    pub ts: String,
    /// The Slack user id of the author.
    pub user: Option<String>,
    /// The message, in Slack's markup.
    pub text: String,
    /// The names of the files uploaded with it; empty when none.
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A reply in a thread an open `slack_reply` wait names. It never names a trigger.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlackReplied {
    /// The channel id.
    pub channel: String,
    /// The `ts` of the thread's first message.
    pub thread_ts: String,
    /// Slack's id of the reply.
    pub ts: String,
    /// The Slack user id of the author.
    pub user: Option<String>,
    /// The author's display name, from Slack's `users.info`; the user id when Slack does not
    /// answer, as without the `users:read` scope; null with no user id.
    pub actor: Option<String>,
    /// The reply, in Slack's markup.
    pub text: String,
    /// The names of the files uploaded with it; empty when none.
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(flatten)]
    pub source: Source,
}

/// A schedule trigger's firing.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleFired {
    /// The trigger's name.
    pub trigger: String,
    /// When the firing was due, RFC 3339 in UTC.
    #[schemars(extend("format" = "date-time"))]
    pub scheduled: String,
}

/// A task asked for by hand with `priory request`.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CliRequested {
    /// The request, as typed.
    pub text: String,
    /// `$USER`, or the Slack user who sent the shortcut's form as `<username> (<user id>)`; null when
    /// `$USER` is unset.
    pub person: Option<String>,
    /// When the command ran, RFC 3339.
    #[schemars(extend("format" = "date-time"))]
    pub time: String,
    /// `--repo`, a GitHub `owner/name`: `priory request` refuses any other shape.
    pub repo: Option<String>,
}

/// A request for a task. Only this message creates a task.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskRequested {
    /// The workflow the task starts.
    pub workflow: String,
    /// Which events are the same work, such as `github:owner/name#42` or `linear:ENG-7`.
    pub key: String,
    /// The trigger's or the request's band.
    pub band: Option<String>,
    /// The trigger's or the request's lane; null when `lanes.toml` derives it.
    pub lane: Option<String>,
    /// The trigger's name, or `cli`.
    pub trigger: String,
    /// The `id` of the source event's message.
    #[schemars(extend("format" = "uuid"))]
    pub source: String,
    /// The Linear issue's team key. Linear sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// The Linear issue's project name, or null without one. Linear sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<Option<String>>,
    /// Where the Linear issue stands in its project, which orders the task in the queue; null
    /// without a project. Linear view and project sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<IssueOrder>,
    /// The repository the task works in: `priory request --repo` for a CLI request, the pull
    /// request's for a review request. Those sources only. A task whose `repo` is not a GitHub
    /// `owner/name` blocks before its first run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<Option<String>>,
    /// Files the task held: the trigger's `hold`, or `priory request --hold`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
    /// The task's TL;DR, from `priory request --tldr`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tldr: Option<String>,
    /// How the new task relates to existing ones, from `priory request --relates`. The daemon skips
    /// a relation to a task that does not exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relates: Option<Vec<crate::request::Relation>>,
    /// Who answers the task's asks, from `priory request` or `priory add --intervention`; the
    /// trigger's when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intervention: Option<priory_factory::factory::Intervention>,
    /// The lowest confidence at which the daemon answers the task's asks with their
    /// recommendation, from `priory request` or `priory add --auto-answer`: `high` or `medium`;
    /// the trigger's when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_answer: Option<priory_factory::factory::Confidence>,
    /// The task's labels, which runner filters match: the trigger's `labels`, or `priory request`
    /// or `priory add --label`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend(
        "propertyNames" = { "pattern": "^[a-z0-9][a-z0-9_.-]*$" },
        "additionalProperties" = { "type": "string", "pattern": "^[A-Za-z0-9_.:/-]+$" }
    ))]
    pub labels: Option<std::collections::BTreeMap<String, String>>,
    /// The batch a planning task plans, from `priory batch add`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<BatchRequest>,
    /// The outside session that added the task through `priory mcp --outside`: its hc terminal,
    /// then its Claude session id, or `outside` without an hc terminal. CLI sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// When the firing was due. Schedule sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("format" = "date-time"))]
    pub scheduled: Option<String>,
    /// The trigger's `prompt`, or null without one. Schedule sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Option<String>>,
    /// The task's title, in place of the source event's, from the trigger's `pre` script.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 200))]
    pub title: Option<String>,
    /// The task's request text, in place of the source event's, from the trigger's `pre` script.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text the trigger's `pre` script adds after the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// What the trigger's `pre` script did. Triggers with one only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre: Option<PreRan>,
}

/// What a trigger's `pre` script did to a request: its name alone when it changed nothing.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreRan {
    /// The script, `argv[0]` of the trigger's `[pre]`.
    pub script: String,
    /// The fields it changed: `title`, `text`, `context`, `band`, `lane` or `key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<Vec<String>>,
    /// The band before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<FromTo<Option<String>>>,
    /// The lane before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<FromTo<Option<String>>>,
    /// The key before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<FromTo<String>>,
    /// The title's length in characters before and after; null for none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<FromTo<Option<u64>>>,
    /// The request text's length in characters before and after; null for none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<FromTo<Option<u64>>>,
    /// The context's length in characters before and after; null for none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<FromTo<Option<u64>>>,
    /// How long it ran. With `changed` or `failed` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// How it failed, such as `exit 2: <stderr tail>`. The request is the one the trigger built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

/// A value before and after a `pre` script.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FromTo<T> {
    pub from: T,
    pub to: T,
}

/// A trigger's `pre` script dropped a request: it exited 1, or failed with `on_error = "drop"`.
/// It takes the id the request would have had, so one decision stands per event and trigger.
/// The source event is still published.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntakeDropped {
    /// The trigger's name.
    pub trigger: String,
    /// The `id` of the source event's message.
    #[schemars(extend("format" = "uuid"))]
    pub source: String,
    /// The key the request had.
    pub key: String,
    /// The script, `argv[0]` of the trigger's `[pre]`.
    pub script: String,
    /// Its exit code; null when it was killed, timed out or could not start.
    pub exit: Option<i32>,
    /// The first line it printed on stdout when it exited 1, if any.
    pub reason: Option<String>,
    /// How long it ran.
    pub seconds: f64,
    /// How it failed, when `on_error = "drop"` dropped the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

/// A runner claims a task's key: for a request no runner holds, to take over a waiting task of a
/// runner whose lease ended, or because a person gave it the task with `priory give`. The first
/// claim in log order that wins a key makes its runner the owner. Published by daemons of a factory
/// with several runners, and by `priory give`.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskClaimed {
    /// The task's key, such as `github:owner/name#42`.
    pub key: String,
    /// The runner that claims it.
    pub runner: String,
    /// The `id` of the task request the owner creates the task from.
    #[schemars(extend("format" = "uuid"))]
    pub request: String,
    /// The runner it takes the key from, for a takeover or a gift.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// True when a person gave the task with `priory give`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given: Option<bool>,
    /// True when the runner's task of the key ended before the factory had runners, so the claim
    /// only marks its request as handled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
}

/// What another runner may do with a task, as its owner says it each time it changes: `waiting`
/// (no run of it started, so it moves once its runner's lease ends), `kept` (started or asleep, so
/// it stays) or `ended` (done, failed or dropped).
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskHeld {
    /// The task's key.
    pub key: String,
    /// The runner that owns it.
    pub runner: String,
    /// `waiting`, `kept` or `ended`.
    pub hold: String,
    /// The task's generation: 1, then one more at each revive or reopen. A write from another
    /// machine names it, so the owner refuses one meant for an earlier generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
}

/// A runner renews its lease, once a minute while its daemon runs. Its lease ends 30 minutes after
/// this message's `time`.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunnerAlive {
    /// The runner, named by its hostname.
    pub runner: String,
}

/// A person answers an ask of another runner, with `priory answer RUNNER:ASK`. Only that runner
/// applies it, with the revision check of a local answer.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskAnswered {
    /// The runner whose ask it answers.
    pub runner: String,
    /// The ask's id on that runner.
    pub ask: i64,
    /// The question revision the person read.
    pub revision: i64,
    /// The answer.
    pub text: String,
    /// True when the person confirmed that the answer runs the ask's command.
    pub run: bool,
    /// Where the answer came from: `cli` or `serve`.
    pub by: String,
    /// The answer's own message id, which the runner's `priory.ask.applied` names.
    #[schemars(extend("format" = "uuid"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission: Option<String>,
}

/// A person dismisses an ask of another runner, with `priory dismiss RUNNER:ASK` or its endpoint.
/// Only that runner applies it, with the checks of a local dismiss.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskDismissed {
    /// The runner whose ask it dismisses.
    pub runner: String,
    /// The ask's id on that runner.
    pub ask: i64,
    /// The question revision the person read.
    pub revision: i64,
    /// The dismiss's own message id, which the runner's `priory.ask.applied` names.
    #[schemars(extend("format" = "uuid"))]
    pub submission: String,
}

/// A runner posts one of its asks each time any part of it that a view shows changes: asked,
/// revised, answered, refused, dismissed or withdrawn. Every other runner keeps the post with the
/// highest `version` and lists the ask as `<runner>:<ask>`.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskPosted {
    /// The runner that owns the ask.
    pub runner: String,
    /// The ask's id on that runner.
    pub ask: i64,
    /// Raised by every change the post shows. A refused answer raises it, never the question's
    /// `revision`.
    pub version: i64,
    /// `step`, `agent` or `outside`: who asked.
    pub kind: String,
    /// The ask as the runner's own `priory asks --json` lists it, with its `state`, `revision`,
    /// `revisions` and `refused`.
    pub listed: serde_json::Value,
}

/// A runner applied or refused an answer or dismiss that another machine sent to one of its asks.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskApplied {
    /// The runner that owns the ask.
    pub runner: String,
    /// The ask's id on that runner.
    pub ask: i64,
    /// The `id` of the `priory.ask.answered` or `priory.ask.dismissed` message.
    #[schemars(extend("format" = "uuid"))]
    pub submission: String,
    /// `answered`, `dismissed` or `refused`.
    pub outcome: String,
    /// Why it was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A person on one runner resumes, revives, cancels, releases or says to a task of another, with
/// `priory resume RUNNER:KEY` and its siblings or their endpoints. Only that runner applies it, with
/// the checks of a local action, and only while it holds the key at that generation.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskActed {
    /// The runner the person sent it to.
    pub runner: String,
    /// The task's key.
    pub key: String,
    /// The task's generation the person saw.
    pub generation: i64,
    /// `resume`, `revive`, `cancel`, `release` or `say`.
    pub act: String,
    /// What `say` tells the task's agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The step `revive` restarts at, instead of the one the task ended on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// Where the action came from: `cli` or `serve`.
    pub by: String,
    /// The action's own message id, which the runner's `priory.task.applied` names.
    #[schemars(extend("format" = "uuid"))]
    pub submission: String,
}

/// A runner applied or refused an action that another machine sent to one of its tasks.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskApplied {
    /// The runner the action was sent to.
    pub runner: String,
    /// The task's key.
    pub key: String,
    /// The generation the action named.
    pub generation: i64,
    /// The `id` of the `priory.task.acted` message.
    #[schemars(extend("format" = "uuid"))]
    pub submission: String,
    /// `applied` or `refused`.
    pub outcome: String,
    /// Why it was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A person drops the requests of a key that no runner claimed, with `priory cancel KEY`. Every
/// machine drops the ones it holds as `unaccepted`. A claim on its way still wins.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskDropped {
    /// The key.
    pub key: String,
    /// Where the drop came from: `cli`.
    pub by: String,
}

/// An agent saves a memory with the `remember` tool, or removes one with `forget`. Each message is
/// one candidate, known by its `id`: its own task's later runs get it at once, and other tasks'
/// runs only once a person accepts it. A later write of the same name is a new candidate.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryWritten {
    /// The memory's name, in lowercase letters, digits and hyphens.
    pub name: String,
    /// When an agent should read the memory. Absent for a forget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The fact. Absent for a forget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// True for a forget: the candidate removes the memory of its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forget: Option<bool>,
    /// The key of the task whose run wrote it.
    pub task: String,
    /// The runner that wrote it, or null in a factory of one runner.
    pub runner: Option<String>,
    /// A lesson's approval step, as `<workflow>/<step>`. Present only on a lesson, which the daemon
    /// proposes when a person rejects an approval with a note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

/// A person accepts a memory candidate with `priory memories accept`. Every runner gives it to
/// every task's later runs. Between two accepted candidates of one name, the later accept in log
/// order wins.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryAccepted {
    /// The `id` of the `priory.memory.written` message.
    #[schemars(extend("format" = "uuid"))]
    pub candidate: String,
}

/// A person drops a memory candidate with `priory memories drop`. No run gets it, not even its own
/// task's. A drop of an accepted candidate retires it.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryDropped {
    /// The `id` of the `priory.memory.written` message.
    #[schemars(extend("format" = "uuid"))]
    pub candidate: String,
    /// True when a person drops a candidate already accepted, which retires it. A drop without it
    /// applies only to a candidate no one decided yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retire: Option<bool>,
}

/// Where a stream stands after a poll that moved it; a missing marker means the stream is still
/// in its baseline.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    /// `owner/name`. GitHub streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The stream, such as `github.issues.labeled`, `github.review_requests:octocat`, `linear.comments`
    /// or `slack.channel:C0123`.
    pub stream: String,
    pub position: Position,
    /// Every issue in the view or the project, or every pull request asking the reviewer for a
    /// review. Linear view, Linear project and review request streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Members>,
    /// The ids of the events at the position's instant. Linear issue and comment streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen: Option<Vec<String>>,
    /// The view's filter, as Linear reported it at the poll, or the project trigger's filter.
    /// Linear view and project streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
}

/// An issue of a Linear view or project.
#[derive(JsonSchema)]
pub struct Member {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// Where the issue stands in its project; null without a project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<IssueOrder>,
}

/// The members of a set stream.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum Members {
    Issues(Vec<Member>),
    Pulls(Vec<RequestedPull>),
}

/// A GitHub id or number, a count of set changes, a Linear time or a Slack `ts`.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum Position {
    Number(u64),
    Text(String),
}

/// The issues a batch's planning task plans, as task keys, and the workflow of its entries' tasks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    /// Each issue, `linear:ENG-7` or `github:owner/name#42`.
    pub issues: Vec<String>,
    /// The workflow each entry's task runs.
    pub entries: String,
}

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
    /// The GitHub login who reviewed.
    pub actor: Option<String>,
    /// GitHub's review id.
    pub github_id: u64,
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
    /// The comment's Markdown, as written.
    pub body: String,
    /// GitHub's comment id.
    pub github_id: u64,
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
    /// skipped.
    pub failing: u64,
    /// GitHub's check run id.
    pub github_id: u64,
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
    /// Linear's id of what the event is about: the issue, history entry or comment.
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

/// An issue that entered or left a saved Linear view since the previous poll.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewChanged {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The view, as the trigger names it.
    pub view: String,
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
    #[serde(flatten)]
    pub source: Source,
}

/// An entry of a Linear issue's history that moves it from one state to another.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearStateChanged {
    #[serde(flatten)]
    pub issue: LinearIssue,
    /// The state it moved from.
    pub from: String,
    /// The state it moved to.
    pub to: String,
    /// The Linear display name of who moved it.
    pub actor: Option<String>,
    /// The identifiers of the open issues that block it; null past 20 relations.
    pub blockers: Option<Vec<String>>,
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
    /// The reply, in Slack's markup.
    pub text: String,
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
    /// `$USER`; null when it is unset.
    pub person: Option<String>,
    /// When the command ran, RFC 3339.
    #[schemars(extend("format" = "date-time"))]
    pub time: String,
    /// `--repo`, `owner/name`.
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
    /// The repository the task works in: `priory request --repo` for a CLI request, the pull
    /// request's for a review request. Those sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<Option<String>>,
    /// Files the task held: the trigger's `hold`, or `priory request --hold`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
    /// The task's TL;DR, from `priory request --tldr`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tldr: Option<String>,
    /// When the firing was due. Schedule sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("format" = "date-time"))]
    pub scheduled: Option<String>,
    /// The trigger's `prompt`, or null without one. Schedule sources only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Option<String>>,
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
    /// Every issue in the view, or every pull request asking the reviewer for a review. Linear view
    /// and review request streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Members>,
    /// The ids of the events at the position's instant. Linear issue and comment streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen: Option<Vec<String>>,
    /// The view's filter, as Linear reported it at the poll. Linear view streams only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
}

/// The members of a set stream.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum Members {
    Issues(Vec<LinearIssue>),
    Pulls(Vec<RequestedPull>),
}

/// A GitHub id or number, a count of set changes, a Linear time or a Slack `ts`.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum Position {
    Number(u64),
    Text(String),
}

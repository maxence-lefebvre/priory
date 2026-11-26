// Whether say reaches a task, by the daemon's own rule in tools.rs: an active model step with a run,
// or a done task, which the daemon lists for an open pull request only when say reopens it.

const ENDED = new Set(["failed", "dropped"]);

const decider = (flows, task) =>
  flows?.workflows?.find((workflow) => workflow.name === task.workflow)?.steps?.find((step) => step.id === task.step)?.decides;

/** Whether say reaches `task`, and the note under the buttons. */
export function sayRoute(task, flows) {
  const name = `Task ${task.id}`;
  if (ENDED.has(task.state)) return { enabled: false, note: `${name} is ${task.state}.` };
  if (task.state === "done") return { enabled: true, note: `${name} is done. Say reopens it: its agent resumes its session, works and pushes.` };
  const decides = decider(flows, task);
  if (!decides) return { enabled: false, note: `${name} is at ${task.step}. Priory has not said yet who decides that step.` };
  if (decides !== "model") return { enabled: false, note: `${name} waits at ${task.step}, a ${decides} step. Say reaches it once it moves.` };
  if (!task.runs?.length) return { enabled: false, note: `${name} has no run yet. Say reaches it once its agent started.` };
  const open = (task.asks ?? []).filter((ask) => ask.state === "open").map((ask) => `Ask ${ask.id}`);
  const wakes = task.state === "asleep" ? "It wakes, works" : "It works";
  const stays = open.length ? ` ${open.join(", ")} ${open.length > 1 ? "stay" : "stays"} open.` : "";
  return { enabled: true, note: `Sent with say to task ${task.id}'s agent. ${wakes} in its worktree and pushes.${stays}` };
}

/** The body of say: a done task can hold several pull requests, and the daemon reopens it on the one on screen. */
export const sayBody = (pr, text) => ({ text, pull_request: pr.resource });

/**
 * The branch and base Update branch and Resolve conflicts name, or null when `holds` says the task
 * does not own the pull request's branch: it only waits on, reviews or subscribes to it.
 */
export function fixBranch(task, holds, pr) {
  if (!holds?.includes("key") && !holds?.includes("stack")) return null;
  // GitHub's owner and repository names ignore case, so a page's URL may not match Priory's.
  const same = (a, b) => a?.toLowerCase() === b.toLowerCase();
  const pull = task.stack?.find((entry) => same(entry.pull_request, pr.resource)) ?? (same(task.pull_request?.url, pr.url) ? task.pull_request : null);
  return pull?.branch ? { branch: pull.branch, base: pull.base ?? "main" } : null;
}

/** The fixed text Update branch or Resolve conflicts says: a merge of the base, so no force push. */
export function fixText(kind, branch, base) {
  const update = `Bring ${branch} up to date with ${base} by merging it, resolve any conflicts, run the tests and push.`;
  return kind === "conflicts" ? `GitHub reports conflicts between ${branch} and ${base}. ${update}` : update;
}

/** The fixed text Generate guide says: the review guide block the Guide tab reads, written into the description. */
export const guideText = (pr) =>
  `Write a review guide for ${pr.url} into its description: the <!-- priory-guide --> block that ` +
  "https://github.com/maxence-lefebvre/priory/blob/main/docs/adapters/github.md#the-review-guide-block describes, " +
  "with head set to the pull request's head commit. Keep the rest of the description as it is.";

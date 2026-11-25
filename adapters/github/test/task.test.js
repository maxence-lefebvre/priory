import { test } from "node:test";
import assert from "node:assert/strict";
import { fixBranch, fixText, said, sayRoute } from "../lib/task.js";

const FLOWS = {
  workflows: [{
    name: "implement",
    steps: [
      { id: "implement", decides: "model" },
      { id: "approve-pr", decides: "human" },
      { id: "merge", decides: "code" },
    ],
  }],
};
const TASK = { id: 61, workflow: "implement", step: "implement", state: "asleep", runs: [{ id: 1 }], asks: [{ id: 88, state: "open" }] };

test("a task at a model step with a run hears say, and the note says so before the tap", () => {
  assert.deepEqual(sayRoute(TASK, FLOWS), {
    enabled: true,
    note: "Sent with say to task 61's agent. It wakes, works in its worktree and pushes. Ask 88 stays open.",
  });
  assert.equal(sayRoute({ ...TASK, state: "running", asks: [] }, FLOWS).note, "Sent with say to task 61's agent. It works in its worktree and pushes.");
});

test("a task at a human or code step, with no run yet, or ended cannot hear say, and says why", () => {
  const cases = [
    [{ ...TASK, step: "approve-pr" }, "Task 61 waits at approve-pr, a human step. Say reaches it once it moves."],
    [{ ...TASK, step: "merge" }, "Task 61 waits at merge, a code step. Say reaches it once it moves."],
    [{ ...TASK, runs: [], state: "queued" }, "Task 61 has no run yet. Say reaches it once its agent started."],
    [{ ...TASK, state: "dropped" }, "Task 61 is dropped."],
  ];
  for (const [task, note] of cases) assert.deepEqual(sayRoute(task, FLOWS), { enabled: false, note });
});

test("a done task the daemon lists reopens with say, whatever step it ended at", () => {
  assert.deepEqual(sayRoute({ ...TASK, state: "done", step: "merge" }, FLOWS), {
    enabled: true,
    note: "Task 61 is done. Say reopens it: its agent resumes its session, works and pushes.",
  });
});

test("say to a done task names the pull request on screen", () => {
  assert.equal(said({ state: "done" }, PR, "Fix it."), "On https://github.com/owner/repo/pull/7: Fix it.");
  assert.equal(said({ state: "asleep" }, PR, "Fix it."), "Fix it.");
});

test("without the factory's flows, say is off and the note does not guess the step's kind", () => {
  assert.deepEqual(sayRoute(TASK, null), { enabled: false, note: "Task 61 is at implement. Priory has not said yet who decides that step." });
});

test("Update branch merges the base; Resolve conflicts says GitHub reports them first", () => {
  assert.equal(fixText("update", "priory/61-herdr", "main"), "Bring priory/61-herdr up to date with main by merging it, resolve any conflicts, run the tests and push.");
  assert.equal(fixText("conflicts", "priory/61-herdr", "main"), "GitHub reports conflicts between priory/61-herdr and main. Bring priory/61-herdr up to date with main by merging it, resolve any conflicts, run the tests and push.");
});

const PR = { resource: "github:owner/repo#7", url: "https://github.com/owner/repo/pull/7" };

test("Update branch names only the branch of a task that owns the pull request, by key or stack", () => {
  const stacked = { branch: "priory/12", stack: [{ pull_request: "github:owner/repo#6", branch: "priory/12-a", base: "main" }, { pull_request: PR.resource, branch: "priory/12-b", base: "priory/12-a" }] };
  const keyed = { branch: "priory/13", stack: [], pull_request: { url: PR.url, branch: "fix-docs", base: "main" } };

  assert.deepEqual(fixBranch(stacked, ["stack", "subscription"], PR), { branch: "priory/12-b", base: "priory/12-a" });
  assert.deepEqual(fixBranch(keyed, ["key"], PR), { branch: "fix-docs", base: "main" });
});

test("a page that writes the repository in another case still finds the task's stack entry", () => {
  const upper = { resource: "github:Owner/Repo#7", url: "https://github.com/Owner/Repo/pull/7" };
  const stacked = { stack: [{ pull_request: PR.resource, branch: "priory/12", base: "main" }] };
  const keyed = { stack: [], pull_request: { url: PR.url, branch: "fix-docs", base: "main" } };

  assert.deepEqual(fixBranch(stacked, ["stack"], upper), { branch: "priory/12", base: "main" });
  assert.deepEqual(fixBranch(keyed, ["key"], upper), { branch: "fix-docs", base: "main" });
});

test("a task that only waits on, reviews or subscribes to the pull request gets no branch, nor its own", () => {
  const own = { branch: "priory/12", stack: [], pull_request: { url: "https://github.com/owner/repo/pull/9", branch: "priory/12", base: "main" } };

  for (const holds of [["wait"], ["review"], ["subscription"], ["owns"]]) assert.equal(fixBranch(own, holds, PR), null, holds[0]);
  assert.equal(fixBranch(own, ["key"], PR), null);
});

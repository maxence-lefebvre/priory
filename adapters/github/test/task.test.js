import { test } from "node:test";
import assert from "node:assert/strict";
import { fixText, sayRoute } from "../lib/task.js";

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
    [{ ...TASK, state: "done" }, "Task 61 is done."],
  ];
  for (const [task, note] of cases) assert.deepEqual(sayRoute(task, FLOWS), { enabled: false, note });
});

test("without the factory's flows, say is off and the note does not guess the step's kind", () => {
  assert.deepEqual(sayRoute(TASK, null), { enabled: false, note: "Task 61 is at implement. Priory has not said yet who decides that step." });
});

test("Update branch merges the base; Resolve conflicts says GitHub reports them first", () => {
  assert.equal(fixText("update", "priory/61-herdr", "main"), "Bring priory/61-herdr up to date with main by merging it, resolve any conflicts, run the tests and push.");
  assert.equal(fixText("conflicts", "priory/61-herdr", "main"), "GitHub reports conflicts between priory/61-herdr and main. Bring priory/61-herdr up to date with main by merging it, resolve any conflicts, run the tests and push.");
});

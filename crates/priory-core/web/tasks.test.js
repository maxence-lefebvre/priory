import { test } from "node:test";
import assert from "node:assert/strict";
import { TASK_FACETS, TASK_VIEWS, elsewhere, filtered, olderPart, taskFacets, taskViews, triageWords } from "./tasks.js";

// `/api/tasks` as the server sends it: in start order, ended tasks in it too.
const tasks = [
  { id: 3, key: "ENG-224", workflow: "implement", step: "implement", state: "blocked", lane: "search", band: "now", since: "2026-11-21T07:00:00Z" },
  { id: 12, key: "ENG-244", workflow: "implement", step: "review", state: "running", lane: "payments", band: "now", since: "2026-11-21T09:40:00Z" },
  { id: 5, key: "ENG-201", workflow: "implement", step: "done", state: "done", lane: "payments", band: "today", since: "2026-11-20T07:10:00Z", batch: 1 },
  { id: 6, key: "ENG-238", workflow: "plan-batch", step: "done", state: "done", lane: "payments", band: "later", since: "2026-11-21T06:00:00Z" },
  { id: 7, key: "ENG-239", workflow: "implement", step: "implement", state: "held", lane: "payments", band: "later", since: "2026-11-21T06:00:00Z", batch: 1 },
  { id: 10, key: "ENG-205", workflow: "implement", step: "implement", state: "failed", lane: null, band: "today", since: "2026-11-20T06:02:00Z" },
];
const ids = (list) => list.map((task) => task.id);

test("the saved views split the tasks into active in start order, blocked, and ended newest first", () => {
  assert.deepEqual(TASK_VIEWS, ["active", "blocked", "ended"]);
  const views = taskViews(tasks);
  assert.deepEqual(ids(views.active), [3, 12, 7]);
  assert.deepEqual(ids(views.blocked), [3]);
  assert.deepEqual(ids(views.ended), [6, 5, 10]);
});

test("the facets filter by state, lane and workflow, and a task with no lane counts under none", () => {
  assert.deepEqual(TASK_FACETS.map(([key, label]) => [key, label]), [["state", "State"], ["lane", "Lane"], ["workflow", "Workflow"]]);
  assert.deepEqual(ids(filtered(tasks, new Map([["lane", "payments"]]))), [12, 5, 6, 7]);
  assert.deepEqual(ids(filtered(tasks, new Map([["lane", "none"]]))), [10]);
  assert.deepEqual(filtered([{ id: null, workflow: null }], new Map([["workflow", "none"]])).length, 1, "another runner's task may have no workflow");
  assert.deepEqual(ids(filtered(tasks, new Map([["workflow", "implement"], ["state", "done"]]))), [5]);
  assert.deepEqual(ids(filtered(tasks, new Map())), ids(tasks));
});

test("a list's older rows count only without a filter, yet still load in their view while one is set", () => {
  assert.deepEqual(olderPart({ view: "ended", history: "ended", older: 23, picked: false }), { counted: 23, unloaded: 23 });
  assert.deepEqual(olderPart({ view: "ended", history: "ended", older: 23, picked: true }), { counted: 0, unloaded: 23 });
  assert.deepEqual(olderPart({ view: "active", history: "ended", older: 23, picked: true }), { counted: 0, unloaded: 0 });
  assert.deepEqual(olderPart({ view: "recent", history: "recent", older: undefined, picked: false }), { counted: 0, unloaded: 0 });
});

test("another runner's task shows in Active, under the state elsewhere, with its runner and last alive time", () => {
  const remote = { id: null, key: "ENG-250", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-21T08:00:00Z", runner: "a", hold: "kept", last_seen: "2026-11-21T09:48:00Z" };
  const all = [...tasks, remote];
  assert.deepEqual(taskViews(all).active.map((task) => task.key), ["ENG-224", "ENG-244", "ENG-239", "ENG-250"]);
  assert.deepEqual(filtered(all, new Map([["state", "elsewhere"]])), [remote]);
  assert.equal(elsewhere(remote, (iso) => (iso === remote.last_seen ? "12 min ago" : "?")), "On runner a, last seen 12 min ago");
  assert.equal(elsewhere({ ...remote, last_seen: null }, () => "?"), "On runner a, never seen alive");
  assert.equal(elsewhere({ ...tasks[0], runner: "b", hold: "kept" }, () => "?"), null);
});

test("a feedback item's class reads in plain words", () => {
  assert.equal(triageWords("trivial"), "trivial");
  assert.equal(triageWords("nothing-to-do"), "nothing to do");
  assert.equal(triageWords("persons-call"), "your call");
});

test("in a factory of several runners a Runner facet filters by runner and marks this runner (here)", () => {
  assert.deepEqual(taskFacets(tasks), TASK_FACETS, "one runner: no Runner facet");
  const here = tasks.map((task) => ({ ...task, runner: "b", hold: "kept" }));
  const remote = { id: null, key: "ENG-250", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-21T08:00:00Z", runner: "a", hold: "kept", last_seen: null };
  const all = [...here, remote];
  const facets = taskFacets(all);
  assert.deepEqual(facets.map(([key, label]) => [key, label]), [["state", "State"], ["lane", "Lane"], ["workflow", "Workflow"], ["runner", "Runner"]]);
  const [, , , , named] = facets[3];
  assert.equal(named("b"), "b (here)");
  assert.equal(named("a"), "a");
  assert.deepEqual(filtered(all, new Map([["runner", "a"]]), facets), [remote]);
  assert.equal(filtered(all, new Map([["runner", "b"]]), facets).length, tasks.length);
});

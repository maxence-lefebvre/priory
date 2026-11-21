import { test } from "node:test";
import assert from "node:assert/strict";
import { TASK_FACETS, TASK_VIEWS, filtered, taskViews } from "./tasks.js";

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
  assert.deepEqual(ids(filtered(tasks, new Map([["workflow", "implement"], ["state", "done"]]))), [5]);
  assert.deepEqual(ids(filtered(tasks, new Map())), ids(tasks));
});

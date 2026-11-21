import { test } from "node:test";
import assert from "node:assert/strict";
import { RUN_FACETS, RUN_VIEWS, duration, runState, runViews, tokenText } from "./runs.js";
import { filtered } from "./tasks.js";

// `/api/runs` as the server sends it: newest first.
const runs = [
  { run: 31, task: 12, key: "ENG-244", step: "review", agent: "reviewer", started: "2026-11-21T09:40:00Z", ended: null, end: null, outcome: null, model: null, tokens: null },
  { run: 30, task: 12, key: "ENG-244", step: "implement", agent: "coder", started: "2026-11-21T08:40:00Z", ended: "2026-11-21T09:22:00Z", end: "moved", outcome: "done", model: "claude-sonnet-5-5", tokens: { input: 300000, output: 12000, cache_read: 0, cache_write: 0 } },
  { run: 29, task: 3, key: "ENG-224", step: "implement", agent: "coder", started: "2026-11-21T06:00:00Z", ended: "2026-11-21T06:26:00Z", end: "blocked", outcome: "blocked", model: "claude-opus-5-5", tokens: null },
  { run: 25, task: 10, key: "ENG-205", step: "implement", agent: null, started: "2026-11-20T05:54:00Z", ended: "2026-11-20T06:02:00Z", end: null, outcome: null, model: null, tokens: null },
];
const ids = (list) => list.map((run) => run.run);

test("a run's state is running while live, else how it ended", () => {
  assert.deepEqual(runs.map(runState), ["running", "moved", "blocked", "ended"]);
});

test("the saved views list the recent runs newest first, or only the live ones", () => {
  assert.deepEqual(RUN_VIEWS, ["recent", "live"]);
  assert.deepEqual(ids(runViews(runs).recent), [31, 30, 29, 25]);
  assert.deepEqual(ids(runViews(runs).live), [31]);
});

test("the facets filter by outcome, agent and step; a run's outcome is both how it ended and its result's outcome", () => {
  assert.deepEqual(RUN_FACETS.map(([key, label]) => [key, label]), [["outcome", "Outcome"], ["agent", "Agent"], ["step", "Step"]]);
  assert.deepEqual(ids(filtered(runs, new Map([["outcome", "moved"]]), RUN_FACETS)), [30]);
  assert.deepEqual(ids(filtered(runs, new Map([["outcome", "done"]]), RUN_FACETS)), [30]);
  assert.deepEqual(ids(filtered(runs, new Map([["outcome", "blocked"]]), RUN_FACETS)), [29]);
  assert.deepEqual(ids(filtered(runs, new Map([["outcome", "running"]]), RUN_FACETS)), [31]);
  assert.deepEqual(ids(filtered(runs, new Map([["agent", "none"]]), RUN_FACETS)), [25]);
  assert.deepEqual(ids(filtered(runs, new Map([["step", "implement"], ["agent", "coder"]]), RUN_FACETS)), [30, 29]);
});

test("a run's duration counts in minutes, then hours, up to now while it is live", () => {
  const now = Date.parse("2026-11-21T10:00:00Z");
  assert.deepEqual(runs.map((run) => duration(run, now)), ["20 min", "42 min", "26 min", "8 min"]);
  assert.equal(duration({ started: "2026-11-21T07:15:00Z", ended: "2026-11-21T09:20:30Z" }, now), "2 h 5 min");
  assert.equal(duration({ started: "2026-11-21T09:59:40Z", ended: null }, now), "0 min");
});

test("a run's tokens add up its four counts in thousands or millions, and none shows blank", () => {
  assert.equal(tokenText(runs[1].tokens), "312k");
  assert.equal(tokenText({ input: 1200, output: 300, cache_read: 0, cache_write: 0 }), "1.5k");
  assert.equal(tokenText({ input: 640, output: 0 }), "640");
  assert.equal(tokenText({ input: 1_100_000, output: 150_000, cache_read: 2_000_000, cache_write: 0 }), "3.3M");
  assert.equal(tokenText(null), "");
});

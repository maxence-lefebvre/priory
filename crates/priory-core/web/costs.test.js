import { test } from "node:test";
import assert from "node:assert/strict";
import { attentionView, costTiles, dayBars, money, scoreTable, splitTable } from "./costs.js";

test("money rounds half a cent up as written in decimal, groups thousands, and shows no cost as -", () => {
  // What Intl.NumberFormat("en", { style: "currency", currency: "USD" }) prints, which Herdr's engine lacks.
  assert.deepEqual([1.005, 2.675, 1234567.891, 0, -3, null].map(money), ["$1.01", "$2.68", "$1,234,567.89", "$0.00", "-$3.00", "-"]);
});

process.env.TZ = "UTC";

const tokens = (input = 0, output = 0) => ({ input, output, cache_read: 0, cache_write: 0 });
const total = (cost_usd, merges = 0, extra = {}) => ({ cost_usd, tokens: tokens(), unpriced: [], unknown: 0, merges, ...extra });
// `/api/costs` as trace.md's Costs example prints it, by workflow.
const costs = (current, previous, rest = {}) => ({
  by: "workflow",
  current: { since: "2026-11-15T23:00:00.000Z", since_day: "2026-11-16", until: "2026-11-17T11:00:00.000Z", ...current },
  previous: { since: "2026-11-08T23:00:00.000Z", since_day: "2026-11-09", until: "2026-11-15T23:00:00.000Z", ...previous },
  days: [],
  split: [],
  ...rest,
});

test("this week's cost and merges compare with last week's as a signed percent", () => {
  const tiles = costTiles(costs(total(2, 3), total(5, 2)));
  assert.deepEqual(tiles.map(({ label, value, delta }) => [label, value, delta]), [
    ["Cost this week", "$2.00", { text: "-60% vs last week", direction: "down" }],
    ["Merges", "3", { text: "+50% vs last week", direction: "up" }],
    ["Cost per merge", "$0.67", { text: "-73% vs last week", direction: "down" }],
  ]);
});

test("a week with no priced token shows no cost and no delta, never $0", () => {
  const unpriced = [{ model: "m2", tokens: tokens(1000, 500) }];
  const [cost, merges, perMerge] = costTiles(costs(total(null, 0, { unpriced }), total(5, 0)));
  assert.deepEqual([cost.value, cost.delta, merges.delta, perMerge.value], ["-", null, null, "-"]);
});

test("each day's bar is its cost over the busiest day's, labelled with its cost and merges", () => {
  const day = (date, cost_usd, merges) => ({ day: date, ...total(cost_usd, merges) });
  const days = [day("2026-11-14", 2, 0), day("2026-11-15", 0, 1), day("2026-11-16", 4, 2), day("2026-11-17", null, 0)];
  const bars = dayBars(costs(total(4), total(2), { days }));
  assert.deepEqual(bars.map(({ weekday, date, fraction, text }) => [`${weekday} ${date}`, fraction, text]), [
    ["Sat 14", 0.5, "Sat, Nov 14: $2.00, no merge"],
    ["Sun 15", 0, "Sun, Nov 15: $0.00, 1 merge"],
    ["Mon 16", 1, "Mon, Nov 16: $4.00, 2 merges"],
    ["Tue 17", 0, "Tue, Nov 17: no priced tokens, no merge"],
  ]);
});

test("the split names its dimension and gives each value its cost, tokens and what has no cost", () => {
  // The by-model split of tests/costs.rs.
  const both = (input, output, cache_read) => ({ input, output, cache_read, cache_write: 0 });
  const split = [
    { model: "m1", cost_usd: 1.1, tokens: both(1_000_000, 0, 1_000_000), unpriced: [], unknown: 1 },
    { model: "m2", cost_usd: null, tokens: both(1_000, 500, 0), unpriced: [{ model: "m2", tokens: both(1_000, 500, 0) }], unknown: 0 },
  ];
  const current = total(1.1, 0, { unpriced: split[1].unpriced, unknown: 1 });
  const table = splitTable(costs(current, total(0), { by: "model", split }));
  assert.equal(table.dimension, "Model");
  assert.deepEqual(table.rows, [
    { name: "m1", cost: "$1.10", tokens: "2M", note: "tokens unknown for 1 record" },
    { name: "m2", cost: "-", tokens: "1.5k", note: "price missing" },
  ]);
  assert.deepEqual(table.unpriced, [{ model: "m2", tokens: "1.5k" }]);
  assert.equal(table.unknown, "1 record has no tokens");
});

// `/api/attention` as trace.md's Attention example prints it.
const attention = (rest = {}) => ({
  since: "2026-11-16T00:00:00.000Z", until: "2026-11-17T11:00:00.000Z", by: "workflow",
  acts: 4, person_minutes: 16, merges: 2, person_minutes_per_merge: 8,
  waits: { acted: 4, open: 0, median_minutes: 45.5, p90_minutes: 61, median_present_minutes: 45.5, p90_present_minutes: 61 },
  split: [], tasks: [],
  ...rest,
});

test("the headline is the person's minutes per merged pull request, with what it divides", () => {
  assert.deepEqual(attentionView(attention()).headline, { label: "minutes of your attention per merge", value: "8", note: "16 min over 2 merges, 4 acts" });
  const none = attentionView(attention({ merges: 0, person_minutes_per_merge: null })).headline;
  assert.deepEqual(none, { label: "minutes of your attention per merge", value: "-", note: "16 min, no merge yet, 4 acts" });
});

test("ask waits show median and p90, with and without the person's away time", () => {
  const waits = { acted: 4, open: 1, median_minutes: 22.5, p90_minutes: 300, median_present_minutes: 15, p90_present_minutes: 25 };
  assert.deepEqual(attentionView(attention({ waits })).waits, [
    { label: "Ask wait, median", value: "23 min", note: "15 min while you were present" },
    { label: "Ask wait, p90", value: "5 h", note: "25 min while you were present" },
    { label: "Asks acted on", value: "4", note: "1 still open" },
  ]);
});

test("each task's wall time splits into working, waiting on you, on the world and queued", () => {
  const task = { id: 1, key: "issue-1", workflow: "implement", lane: "lane-a", state: "done", since: "2026-11-16T08:00:00Z", until: "2026-11-16T12:00:00Z",
    wall_minutes: 240, working_minutes: 110, person_wait_minutes: 30, world_wait_minutes: 90, queued_minutes: 10 };
  const empty = { ...task, id: 2, key: "issue-2", state: "running", until: null, wall_minutes: 0, working_minutes: 0, person_wait_minutes: 0, world_wait_minutes: 0, queued_minutes: 0 };
  const [row, idle] = attentionView(attention({ tasks: [task, empty] })).tasks;
  assert.deepEqual(row, {
    id: 1, key: "issue-1", workflow: "implement", state: "done", wall: "4 h",
    parts: [
      { part: "working", label: "Working", fraction: 110 / 240, value: "1 h 50 min" },
      { part: "person", label: "Waiting on you", fraction: 30 / 240, value: "30 min" },
      { part: "world", label: "Waiting on the world", fraction: 90 / 240, value: "1 h 30 min" },
      { part: "queued", label: "Queued", fraction: 10 / 240, value: "10 min" },
    ],
    text: "Task issue-1, 4 h: working 1 h 50 min, waiting on you 30 min, waiting on the world 1 h 30 min, queued 10 min",
  });
  assert.deepEqual(idle.parts.map((part) => part.fraction), [0, 0, 0, 0]);
});

test("a day is this week's from the local date the current window starts on", () => {
  const day = (date) => ({ day: date, ...total(1) });
  const window = { since: "2026-11-16T00:00:00.000Z", since_day: "2026-11-16", until: "2026-11-17T11:00:00.000Z" };
  const bars = dayBars(costs({ ...total(1), ...window }, total(1), { days: [day("2026-11-15"), day("2026-11-16"), day("2026-11-17")] }));
  assert.deepEqual(bars.map((bar) => bar.current), [false, true, true]);
});

test("a daemon past UTC+12 starts its week on the local date it names", () => {
  // Kiritimati is UTC+14: Monday 00:00 local is Sunday 10:00 UTC, past the reach of a 12-hour shift.
  const day = (date) => ({ day: date, ...total(1) });
  const window = { since: "2026-11-15T10:00:00.000Z", since_day: "2026-11-16", until: "2026-11-17T11:00:00.000Z" };
  const bars = dayBars(costs({ ...total(1), ...window }, total(1), { days: [day("2026-11-15"), day("2026-11-16")] }));
  assert.deepEqual(bars.map((bar) => bar.current), [false, true]);
});

test("a task's time over a day reads in days and hours", () => {
  const task = { id: 1, key: "issue-1", workflow: "implement", lane: "lane-a", state: "running", since: "2026-11-16T08:00:00Z", until: null,
    wall_minutes: 6884.7, working_minutes: 0, person_wait_minutes: 5204.7, world_wait_minutes: 0, queued_minutes: 1680 };
  const [row] = attentionView(attention({ tasks: [task] })).tasks;
  assert.deepEqual([row.wall, row.parts[1].value, row.parts[3].value], ["4 d 18 h", "3 d 14 h", "1 d 4 h"]);
});

test("the week a day falls in follows the daemon's timezone, not the browser's", () => {
  // A Paris daemon starts the week on Monday 00:00 local, Sunday 23:00 UTC; this browser runs in UTC.
  const day = (date) => ({ day: date, ...total(1) });
  const bars = dayBars(costs(total(1), total(1), { days: [day("2026-11-15"), day("2026-11-16")] }));
  assert.deepEqual(bars.map((bar) => [bar.day, bar.current]), [["2026-11-15", false], ["2026-11-16", true]]);
});

test("a scorer's pass rate is pass over pass and fail, with errors and not applicable counted apart", () => {
  const scores = [
    { workflow: "implement", scorer: "gate", pass: 11, fail: 3, error: 1, not_applicable: 1 },
    { workflow: "plan-batch", scorer: "gate", pass: 0, fail: 0, error: 2, not_applicable: 4 },
  ];
  assert.deepEqual(scoreTable(costs(total(0), total(0), { scores })), {
    dimension: "Workflow",
    rows: [
      { name: "implement", scorer: "gate", rate: "79%", pass: 11, fail: 3, error: 1, notApplicable: 1, line: "pass rate 79%, 11 pass, 3 fail, 1 error, 1 not applicable" },
      { name: "plan-batch", scorer: "gate", rate: "-", pass: 0, fail: 0, error: 2, notApplicable: 4, line: "no pass rate, 2 error, 4 not applicable" },
    ],
  });
});

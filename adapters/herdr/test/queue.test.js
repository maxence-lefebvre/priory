import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { moveTargets, queueList, reorder, upNext } from "../mods/priory-queue/view.js";

process.env.TZ = "UTC";

// Captured from `priory queue DIR --json` on a scratch factory: ten tasks requested with
// `priory request --band` and `--hold`, one `daemon --once`, then two tasks marked running with a
// live run and two ended, by SQL. queue-quiet.json is the same state after three queued tasks were
// dropped, so up next reaches the held task and the schedule.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const queue = fixture("queue.json");
const now = Date.parse("2026-11-20T21:20:40Z");
const ids = (section) => section.rows.map((row) => row.id);

test("Next lists the running runs, then each band in start order, beside every view's count", () => {
  const list = queueList(queue, { view: "next", now });
  assert.equal(list.waiting, 4);
  assert.deepEqual(list.views, [
    { id: "next", label: "Next", count: 4 },
    { id: "held", label: "Held", count: 2 },
    { id: "ended", label: "Last day", count: 2 },
    { id: "all", label: "All", count: 10 },
  ]);
  assert.deepEqual(list.sections.map((section) => [section.title, section.note, ids(section)]), [
    ["Running", "2", [1, 2]],
    ["Blocker", "1", [3]],
    ["Now", "2", [4, 5]],
    ["Today", "1", [6]],
  ]);
});

test("each waiting row shows its place in its band and why it has not started", () => {
  const rows = queueList(queue, { view: "next", now }).sections.slice(1).flatMap((section) => section.rows);
  assert.deepEqual(rows.map((row) => [row.position, row.key, row.step, row.tile, row.why]), [
    [1, "cli:PRI-72", "implement", "queued", { text: "Starts when the runner has room.", tone: "muted" }],
    [1, "cli:PRI-73", "implement", "queued", { text: "Starts after cli:PRI-72, the band above.", tone: "muted" }],
    [2, "cli:PRI-74", "implement", "queued", { text: "Starts after cli:PRI-73.", tone: "muted" }],
    [1, "cli:PRI-75", "implement", "queued", { text: "Starts after cli:PRI-74, the band above.", tone: "muted" }],
  ]);
});

test("the task a stall names waits for room, and for a person while starts are paused", () => {
  const stalled = queueList(queue, { view: "next", now, room: { banner: "stalled since 2026-11-20T21:02:00Z: short of disk by 4.1 GB; task 3 waits for room" } });
  assert.deepEqual(stalled.sections[1].rows[0].why, { text: "Waits for room: short of disk by 4.1 GB since 21:02.", tone: "warn" });
  const paused = queueList(queue, { view: "next", now, room: { paused: "paused since 2026-11-20T21:20:08Z: the disk is low" } });
  assert.deepEqual(paused.sections[1].rows[0].why, { text: "Starts are paused: the disk is low", tone: "warn" });
  assert.equal(paused.paused, "Starts are paused since 21:20: the disk is low");
});

test("a running row shows its run's step and age", () => {
  const [running] = queueList(queue, { view: "next", now }).sections;
  assert.deepEqual(running.rows, [
    { id: 1, tile: "running", key: "cli:PRI-66", step: "implement", run: 1, age: "18 min" },
    { id: 2, tile: "running", key: "cli:PRI-67", step: "implement", run: 2, age: "6 min" },
  ]);
});

test("Held lists each hold's reason and each of its conditions, met or not", () => {
  const [held] = queueList(queue, { view: "held", now }).sections;
  assert.deepEqual(held.rows.map((row) => [row.id, row.tile, row.band, row.why, row.conditions]), [
    [7, "held", "Later", { text: "Held: waits for #212, until task 3 ends", tone: "held" }, [{ text: "Until cli:PRI-72 ends", met: false }]],
    [8, "held", "Today", { text: "Held: after the freeze, until 2026-11-21T08:30:00.000Z", tone: "held" }, [{ text: "Until Sat 08:30", met: false }]],
  ]);
  // Hand-built: every other kind of condition, two of them met.
  const conditions = [
    { kind: "person", until: null, task: null, resource: null, met: null, met_as: null },
    { kind: "issue", until: null, task: null, resource: "linear:ENG-7", met: "2026-11-20T10:00:00Z", met_as: "ENG-7 is Done" },
    { kind: "merge", until: null, task: null, resource: "github:owner/repo#212", met: null, met_as: null },
    { kind: "blockers", until: null, task: null, resource: "linear:ENG-9", met: null, met_as: null },
    { kind: "time", until: "2026-11-20T23:00:00Z", task: null, resource: null, met: "2026-11-20T23:00:00Z", met_as: "the time came" },
  ];
  const custom = { ...queue, waiting: [{ ...queue.waiting[4], conditions }] };
  assert.deepEqual(queueList(custom, { view: "held", now }).sections[0].rows[0].conditions, [
    { text: "Until a person releases it", met: false },
    { text: "ENG-7 is Done", met: true },
    { text: "Until github:owner/repo#212 merges", met: false },
    { text: "Until the blockers of linear:ENG-9 are done", met: false },
    { text: "the time came", met: true },
  ]);
});

test("Last day lists the ended tasks latest first, and All shows every section", () => {
  const [ended] = queueList(queue, { view: "ended", now }).sections;
  assert.deepEqual(ended.rows, [
    { id: 10, tile: "failed", key: "cli:PRI-61", step: "implement", age: "0 min", why: { text: "checks failed twice", tone: "muted" } },
    { id: 9, tile: "done", key: "cli:PRI-60", step: "implement", age: "0 min", why: { text: "merged #210", tone: "muted" } },
  ]);
  assert.deepEqual(queueList(queue, { view: "all", now }).sections.map((section) => section.title),
    ["Running", "Blocker", "Now", "Today", "Held", "Last day"]);
});

test("Up next says when each of the next three runs starts, and why", () => {
  assert.deepEqual(upNext(fixture("queue-quiet.json"), now), [
    { task: 3, when: "now", sub: "on room", label: "cli:PRI-72", why: "First in the queue. It starts once there is room." },
    { task: 8, when: "08:30", sub: "Sat", label: "cli:PRI-81", why: "Its hold ends." },
    { task: null, when: "08:30", sub: "Nov 27", label: "bonjour, schedule", why: "The schedule starts a task." },
  ]);
  assert.deepEqual(upNext(queue, now).map((next) => next.why), [
    "First in the queue. It starts once there is room.",
    "First in its band, after the band above.",
    "3rd in the queue.",
  ]);
});

test("a lane or band facet narrows the waiting and held tasks, and offers each value with its count", () => {
  const list = queueList(queue, { view: "all", now, facets: { band: "today" } });
  assert.deepEqual(list.sections.map((section) => [section.title, ids(section)]), [["Today", [6]], ["Held", [8]]]);
  assert.deepEqual(list.views.map((view) => view.count), [1, 1, 0, 2]);
  assert.deepEqual(list.facets, {
    lane: [["none/misc", 6]],
    band: [["blocker", 1], ["now", 2], ["today", 2], ["later", 1]],
  });
  assert.deepEqual(queueList(queue, { view: "next", now, facets: { lane: "none/misc" } }).sections.map((section) => section.title), ["Blocker", "Now", "Today"]);
});

test("reordering moves a task before or after its neighbour in the band, never out of it", () => {
  assert.deepEqual(reorder(queue, 5, -1), { before: 4 });
  assert.deepEqual(reorder(queue, 4, 1), { after: 5 });
  assert.equal(reorder(queue, 4, -1), null);
  assert.equal(reorder(queue, 7, 1), null);
  assert.deepEqual(reorder(queue, 5, { onto: 4 }), { before: 4 });
  assert.deepEqual(reorder(queue, 4, { onto: 5 }), { after: 5 });
  assert.equal(reorder(queue, 4, { onto: 3 }), null);
  assert.equal(reorder(queue, 4, { onto: 4 }), null);
});

test("Move offers the top of the task's band and each other band", () => {
  assert.deepEqual(moveTargets(queue, 5), [
    { label: "To the top of Now", move: { top: true } },
    { label: "To Blocker", move: { band: "blocker" } },
    { label: "To Today", move: { band: "today" } },
    { label: "To Later", move: { band: "later" } },
  ]);
  assert.deepEqual(moveTargets(queue, 3).map((target) => target.label), ["To Now", "To Today", "To Later"]);
});

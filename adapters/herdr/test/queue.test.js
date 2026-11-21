import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { moveTargets, queueList, reorder, upNext } from "../mods/priory-queue/view.js";

process.env.TZ = "UTC";

// Captured from `priory queue DIR --json` on a scratch factory: ten tasks requested with
// `priory request --band` and `--hold`, one `daemon --once`, then two tasks marked running with a
// live run and two ended, by SQL. queue-quiet.json is the same state after three queued tasks were
// dropped, so up next reaches the held task and the schedule. Each row's `title` and the
// queue's `bands` were added by hand once the queue carried them.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const queue = fixture("queue.json");
const now = Date.parse("2026-11-20T21:20:40Z");
const ids = (section) => section.rows.map((row) => row.id);

test("Next lists the running runs, each band in start order, Held and Last day, beside every view's count", () => {
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
    ["Held", "2", [7, 8]],
    ["Last day", "2", [10, 9]],
  ]);
});

test("each waiting row shows its place in its band and why it has not started", () => {
  const rows = queueList(queue, { view: "next", now }).sections.slice(1, 4).flatMap((section) => section.rows);
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
    { id: 1, tile: "running", key: "cli:PRI-66", title: "Retry the flaky upload test", titleSpans: [{ text: "Retry the flaky upload test" }], step: "implement", run: 1, age: "18 min", prs: null },
    { id: 2, tile: "running", key: "cli:PRI-67", title: "Speed up the asks listing", titleSpans: [{ text: "Speed up the asks listing" }], step: "implement", run: 2, age: "6 min", prs: null },
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
    { id: 10, tile: "failed", key: "cli:PRI-61", title: "Pin the Actions versions", titleSpans: [{ text: "Pin the Actions versions" }], step: "implement", age: "0 min", why: { text: "checks failed twice", tone: "muted" }, prs: null },
    { id: 9, tile: "done", key: "cli:PRI-60", title: "Bump the Rust toolchain", titleSpans: [{ text: "Bump the Rust toolchain" }], step: "implement", age: "0 min", why: { text: "merged #210", tone: "muted" }, prs: null },
  ]);
  assert.deepEqual(queueList(queue, { view: "all", now }).sections.map((section) => section.title),
    ["Running", "Blocker", "Now", "Today", "Held", "Last day"]);
});

test("Up next says when each of the next three runs starts, and why", () => {
  assert.deepEqual(upNext(fixture("queue-quiet.json"), now), [
    { task: 3, when: "now", sub: "on room", label: "cli:PRI-72", title: "Free the disk before the nightly build", why: "First in the queue. It starts once there is room." },
    { task: 8, when: "08:30", sub: "Sat", label: "cli:PRI-81", title: "Rotate the webhook secret", why: "Its hold ends." },
    { task: null, when: "08:30", sub: "Nov 27", label: "bonjour, schedule", title: null, why: "The schedule starts a task." },
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
  assert.deepEqual(queueList(queue, { view: "next", now, facets: { lane: "none/misc" } }).sections.map((section) => section.title), ["Blocker", "Now", "Today", "Held"]);
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

test("Move offers the top of the task's band and each other band of the factory, even an empty one", () => {
  const banded = { ...queue, bands: ["blocker", "now", "today", "later", "someday"] };
  assert.deepEqual(moveTargets(banded, 5), [
    { label: "To the top of Now", move: { top: true } },
    { label: "To Blocker", move: { band: "blocker" } },
    { label: "To Today", move: { band: "today" } },
    { label: "To Later", move: { band: "later" } },
    { label: "To Someday", move: { band: "someday" } },
  ]);
  assert.deepEqual(moveTargets(banded, 3).map((target) => target.label), ["To Now", "To Today", "To Later", "To Someday"]);
});

test("each row and up next shows its task's title", () => {
  const list = queueList(queue, { view: "all", now });
  assert.deepEqual(list.sections.map((section) => section.rows.map((row) => [row.id, row.title])), [
    [[1, "Retry the flaky upload test"], [2, "Speed up the asks listing"]],
    [[3, "Free the disk before the nightly build"]],
    [[4, "Add a title to each queue row"], [5, "List the factory's bands in the queue"]],
    [[6, "Guard queue verbs with the revision"]],
    [[7, "Ship the release notes"], [8, "Rotate the webhook secret"]],
    [[10, "Pin the Actions versions"], [9, "Bump the Rust toolchain"]],
  ]);
  assert.deepEqual(upNext(queue, now).map((next) => next.title),
    ["Free the disk before the nightly build", "Add a title to each queue row", "List the factory's bands in the queue"]);
});

test("a task's title in each section turns the factory's keys into link spans", () => {
  const { links } = fixture("links.json");
  const linked = (task) => ({ ...task, title: `Fix ENG-${task.id ?? task.task}` });
  const titled = { ...queue, waiting: queue.waiting.map(linked), running: queue.running.map(linked), ended: queue.ended.map(linked) };
  const spans = (id) => [{ text: "Fix " }, { text: `ENG-${id}`, url: `https://linear.app/acme/issue/ENG-${id}` }];
  for (const section of queueList(titled, { view: "all", now, links }).sections) {
    for (const row of section.rows) assert.deepEqual(row.titleSpans, spans(row.id), section.id);
  }
});

test("a row whose task owns a stack of two or more says how many PRs, from `priory tasks`; one PR says nothing", () => {
  const pull = (number, base, state) => ({ pull_request: `github:acme/app#${number}`, number, branch: `b${number}`, base, state });
  const tasks = [
    { id: 1, stack: [pull(57, "main", "open"), pull(58, "b57", "open")] },
    { id: 4, stack: [pull(60, "main", "open"), pull(61, "b60", "open"), pull(62, "b61", "open")] },
    { id: 5, stack: [pull(63, "main", "open")] },
  ];
  const rows = queueList(queue, { view: "all", now, tasks }).sections.flatMap((section) => section.rows);
  assert.deepEqual(rows.filter((row) => row.prs).map((row) => [row.key, row.prs]), [["cli:PRI-66", "2 PRs"], ["cli:PRI-73", "3 PRs"]]);
  assert.equal(rows.find((row) => row.key === "cli:PRI-74").prs, null);
});

test("an opened row's Open task opens the task in the Task panel", async () => {
  const { context, draw, herdr, press, settle } = await import("./kit.js");
  const { default: Queue } = await import("../mods/priory-queue/main.js");
  herdr.reset();
  herdr.replies = { queue, desk: { items: [] }, tasks: { tasks: [] }, links: { links: [] } };
  const panel = new Queue();
  const cx = context();
  panel.init({}, cx);
  await settle();
  const task = queueList(queue, { view: "next", now }).sections[0].rows[0].id;
  panel.select(task, cx);

  press(draw(panel, cx), `open-${task}`);
  assert.equal(herdr.shared.get("priory.task.open").task, task);
  assert.equal(herdr.shared.get("priory.attach"), undefined);
});

test("a queued task with its own wait reason says that reason, in its row and in Up next", () => {
  const claiming = "waits for its claim to reach the log";
  const waiting = queue.waiting.map((task) => (task.position === 1 && task.band_position === 1 && task.key === "cli:PRI-72" ? { ...task, reason: claiming } : task));
  const upNextReason = queue.up_next.map((next, at) => (at === 0 ? { ...next, why: claiming } : next));
  const reasoned = { ...queue, waiting, up_next: upNextReason };
  const [first] = queueList(reasoned, { view: "next", now }).sections[1].rows;
  assert.deepEqual(first.why, { text: "Waits for its claim to reach the log.", tone: "muted" });
  assert.deepEqual(upNext(reasoned, now)[0], { task: 3, when: "now", sub: "", label: "cli:PRI-72", title: upNext(queue, now)[0].title, why: "Waits for its claim to reach the log." });
});

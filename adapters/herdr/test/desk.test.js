import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { deskModel, mergeSince, tomorrowAt } from "../mods/priory-desk/view.js";

process.env.TZ = "UTC";

// Captured from `priory desk --all --json`, `asks --all --json`, `tasks --json` and `queue DIR --json`
// on a scratch state file seeded by SQL, with PRIORY_NOW=2026-11-20T12:00:00Z.
const fixture = (/** @type {string} */ name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));
const data = { desk: fixture("desk"), asks: fixture("desk-asks"), tasks: fixture("desk-tasks"), queue: fixture("desk-queue") };
const now = Date.parse("2026-11-20T12:00:00Z");

test("counts the open asks that are not snoozed, the oldest wait, the stopped and the snoozed", () => {
  assert.deepEqual(deskModel(data, { now }).count, {
    number: 3,
    label: "asks need you",
    line: "Oldest waits 42 min. 1 stopped. 2 snoozed.",
  });
});

test("the next thing is the desk's next, so an alarm comes before every ask", () => {
  assert.deepEqual(deskModel(data, { now }).next, {
    ask: null,
    snooze: { kind: "blocked", id: 4 },
    revision: null,
    task: 4,
    key: "cli:PRI-55",
    step: "triage",
    age: "31 min",
    why: "an alarm: work stopped and waits on a person",
    question: "Permission prompt denied: write to ~/.ssh/config, outside the worktree.",
    questionSpans: [{ text: "Permission prompt denied: write to ~/.ssh/config, outside the worktree." }],
    recommendation: "Resume",
    tradeoff: null,
    progress: null,
  });
});

test("an ask as the next thing carries its recommendation and its task's progress", () => {
  const desk = { ...data.desk, next: data.desk.ranked[1] };
  assert.deepEqual(deskModel({ ...data, desk }, { now }).next, {
    ask: 1,
    snooze: { kind: "ask", id: 1 },
    revision: 1,
    task: 1,
    key: "cli:PRI-61",
    step: "approve",
    age: "42 min",
    why: "its answer lets an irreversible step run",
    question: "Merge #212 into main? Checks are green at a1f3c9e.",
    questionSpans: [{ text: "Merge #212 into main? Checks are green at a1f3c9e." }],
    recommendation: "merge",
    tradeoff: "The diff stays inside priory-core and the docs.",
    progress: { latestChange: "Checks are green at a1f3c9e", nextAction: "Squash-merge #212 once a person says Merge" },
  });
});

test("an empty desk has no next thing and says what appears here", () => {
  const model = deskModel({ desk: fixture("desk-empty"), asks: { asks: [] }, tasks: { tasks: [] }, queue: { up_next: [] } }, { now });
  assert.equal(model.next, null);
  assert.deepEqual(model.count, { number: 0, label: "asks need you", line: "" });
  assert.equal(model.empty, "No open asks. Agents ask here when they need you.");
});

test("Needs me lists the asks oldest first, then the stopped tasks, then what needs no action", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.views.map(({ id, label, count }) => [id, label, count]), [
    ["needs", "Needs me", 4], ["running", "Running", 1], ["asleep", "Asleep", 4], ["snoozed", "Snoozed", 2], ["all", "All tasks", 9],
  ]);
  const sections = model.sections("needs");
  assert.deepEqual(sections.map(({ title, note }) => [title, note]), [["Asks", "3, oldest first"], ["Stopped", "1"], ["Later", "no action needed"]]);
  assert.deepEqual(sections[0].rows.map((row) => [row.kind, row.key, row.age, row.text, row.recommends, row.note]), [
    ["ask", "cli:PRI-61", "42 min", "Merge #212 into main? Checks are green at a1f3c9e.", "merge", null],
    ["ask", "rayon-desk", "20 min", "Ship the <b>release</b> notes?", "Ship", null],
    ["ask", "cli:PRI-62", "6 min", "Use the backoff crate for retries, or keep the 20-line loop?", "Use backoff", "Back early: new revision 2"],
  ]);
  assert.deepEqual(sections[1].rows.map((row) => [row.kind, row.key, row.text, row.recommends, row.snooze]), [
    ["blocked", "cli:PRI-55", "Permission prompt denied: write to ~/.ssh/config, outside the worktree.", "Resume", { kind: "blocked", id: 4 }],
  ]);
  assert.deepEqual(sections[2].rows.map((row) => [row.kind, row.text]), [
    ["asleep", "4 tasks asleep on waits"],
    ["report", "ci: 1 report to read: merges this week"],
    ["held", "held: waits for the release on Monday"],
  ]);
});

test("the room and intake banners, Since you looked and Up next come from the desk and the queue", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.banners, [
    "stalled since 2026-11-20T11:02:00Z: short of disk by 4.1 GB; task 7 waits for room",
    "the intake fails to poll github since 2026-11-20T10:58:00Z: gh api repos/owner/repo/issues/events?per_page=100&page=1 failed: HTTP 401: Bad credentials",
  ]);
  assert.equal(model.since.from, "09:12");
  assert.deepEqual(model.since.rows.slice(0, 2), [
    { key: "cli:PRI-66", task: 6, what: "factory.run.started triage", time: "11:02" },
    { key: "cli:PRI-63", task: 3, what: "factory.step.ended approve: merged #210", time: "10:41" },
  ]);
  assert.deepEqual(model.upNext, [
    { when: "now", key: "cli:PRI-72", task: 7, why: "first in the queue" },
    { when: "Nov 21, 02:00", key: "schedule:nightly", task: null, why: "schedule nightly fires" },
  ]);
});

test("an empty desk says nothing is queued or scheduled to run next", () => {
  const model = deskModel({ desk: fixture("desk-empty"), asks: { asks: [] }, tasks: { tasks: [] }, queue: { up_next: [] } }, { now });
  assert.deepEqual(model.banners, []);
  assert.equal(model.since.from, null);
  assert.deepEqual(model.upNext, []);
  assert.equal(model.upNextEmpty, "Nothing is queued or scheduled to run next.");
});

test("Snoozed lists each snoozed item with when it comes back, and Running shows each task's progress", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.sections("snoozed")[0].rows.map((row) => [row.kind, row.key, row.note, row.snooze, row.snoozed]), [
    ["failed", "cli:PRI-70", "Snoozed until Nov 21, 09:00", { kind: "failed", id: 5 }, true],
    ["ask", "cli:PRI-63", "Snoozed until Nov 21, 12:05", { kind: "ask", id: 3 }, true],
  ]);
  assert.equal(model.sections("needs")[0].rows[2].snoozed, false);
  assert.deepEqual(model.sections("running")[0].rows.map((row) => [row.key, row.text, row.progress]), [
    ["cli:PRI-66", "Retry with backoff", ["Goal: Retry failed fetches", "Latest: Added a backoff test", "Next: Wire the crate into the fetcher"]],
  ]);
});


test("Since you looked keeps the events the panel opened on after its own priory seen", () => {
  const opened = fixture("desk").since_you_looked;
  const later = [{ time: "2026-11-20T12:10:00Z", task: 1, key: "cli:PRI-61", what: "factory.step.ended approve: merge" }];
  const kept = mergeSince(mergeSince([], opened), later);
  assert.equal(kept.length, opened.length + 1);
  assert.deepEqual(kept[0], later[0]);
  assert.deepEqual(mergeSince(kept, later), kept);
});

test("a prompt or delivery row keeps the desk's own text; blocked and failed show the task's reason", () => {
  const items = [
    { kind: "prompt", id: 9, task: 4, key: "cli:PRI-55", step: "triage", lane: null, since: "2026-11-20T11:00:00Z", what: "Allow write to ~/.ssh/config?", generation: 1, snoozed_until: null, snoozed_at: null, back_early: null },
    { kind: "delivery", id: 4, task: null, key: null, step: null, lane: null, since: "2026-11-20T11:00:00Z", what: "the answer did not reach t3wc after 3 tries: gone", generation: null, snoozed_until: null, snoozed_at: null, back_early: null },
  ];
  const model = deskModel({ ...data, desk: { ...data.desk, items } }, { now });
  assert.deepEqual(model.sections("needs")[0].rows.map((row) => [row.kind, row.text, row.snooze]), [
    ["prompt", "Allow write to ~/.ssh/config?", { kind: "prompt", id: 9 }],
    ["delivery", "the answer did not reach t3wc after 3 tries: gone", null],
  ]);
});

test("tomorrow at 09:00 is local time, for priory snooze --until", () => {
  assert.equal(tomorrowAt(9, Date.parse("2026-11-20T23:30:00Z")), "2026-11-21T09:00:00.000Z");
});

test("an ask's question and a task's title turn the factory's keys into link spans", () => {
  const { links } = fixture("links");
  const asks = { asks: data.asks.asks.map((ask) => (ask.id === 1 ? { ...ask, repo: "acme/app" } : ask)) };
  const tasks = { tasks: data.tasks.tasks.map((task) => (task.key === "cli:PRI-66" ? { ...task, tldr: "Retry ENG-7 with backoff" } : task)) };
  const desk = { ...data.desk, next: data.desk.ranked[1] };
  const model = deskModel({ ...data, desk, asks, tasks }, { now, links });
  const pr = [{ text: "Merge " }, { text: "#212", url: "https://github.com/acme/app/issues/212" }, { text: " into main? Checks are green at a1f3c9e." }];
  assert.deepEqual(model.next.questionSpans, pr);
  assert.deepEqual(model.sections("needs")[0].rows[0].textSpans, pr);
  assert.deepEqual(model.sections("running")[0].rows[0].textSpans,
    [{ text: "Retry " }, { text: "ENG-7", url: "https://linear.app/acme/issue/ENG-7" }, { text: " with backoff" }]);
});

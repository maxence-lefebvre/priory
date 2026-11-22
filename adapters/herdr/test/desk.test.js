import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { deskModel, dueText, mergeSince, tomorrowAt } from "../mods/priory-desk/view.js";

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

test("the count line ends with a calm count of the events that reached no task in 24 h", () => {
  const desk = { ...data.desk, counts: { ...data.desk.counts, unrouted: 3 } };
  assert.equal(deskModel({ ...data, desk }, { now }).count.line, "Oldest waits 42 min. 1 stopped. 2 snoozed. 3 events reached no task in 24 h.");
  const one = { ...data.desk, counts: { ...data.desk.counts, unrouted: 1 } };
  assert.match(deskModel({ ...data, desk: one }, { now }).count.line, / 1 event reached no task in 24 h\.$/);
});

test("the count line ends with a calm count of the events no agent read", () => {
  const desk = { ...data.desk, counts: { ...data.desk.counts, unrouted: 3, unheard: 2 } };
  assert.equal(deskModel({ ...data, desk }, { now }).count.line, "Oldest waits 42 min. 1 stopped. 2 snoozed. 3 events reached no task in 24 h. 2 events no agent read.");
  const one = { ...data.desk, counts: { ...data.desk.counts, unheard: 1 } };
  assert.match(deskModel({ ...data, desk: one }, { now }).count.line, / snoozed\. 1 event no agent read\.$/);
});

test("the next thing is the desk's next, so an alarm comes before every ask", () => {
  assert.deepEqual(deskModel(data, { now }).next, {
    ask: null,
    snooze: { kind: "blocked", id: 4 },
    revision: null,
    todo: null,
    task: 4,
    key: "cli:PRI-55",
    step: "triage",
    age: "31 min",
    why: "work stopped and waits on a person",
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
    todo: null,
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

test("a report row carries its sequence number, so its Read button marks it read", () => {
  const rows = deskModel(data, { now }).sections("needs").flatMap((section) => section.rows);
  assert.deepEqual(rows.filter((row) => row.report != null).map((row) => [row.kind, row.report]), [["report", 11]]);
});

test("Needs me lists the to-dos after the stopped tasks, each with Done", () => {
  const todo = { since: "2026-11-20T11:30:00Z", kind: "todo", id: 2, task: null, key: null, step: null, lane: null, what: "Renew the TLS cert, due 2026-12-01T09:00:00Z", generation: null, snoozed_until: null, snoozed_at: null, back_early: null };
  const desk = { ...data.desk, items: [...data.desk.items, todo] };
  const model = deskModel({ ...data, desk }, { now });
  const sections = model.sections("needs");
  assert.deepEqual(sections.map(({ title }) => title), ["Asks", "Stopped", "To-dos", "Later"]);
  assert.deepEqual(sections[2].rows.map((row) => [row.kind, row.age, row.text, row.recommends, row.todo, row.snooze]), [
    ["todo", "30 min", "Renew the TLS cert, due Dec 1, 09:00 (in 11 days)", "Done", 2, null],
  ]);
  assert.equal(model.views[0].count, 5);
});

test("a to-do's due time reads in the panel's local time with a relative hint", () => {
  process.env.TZ = "America/New_York";
  try {
    const todo = { since: "2026-11-20T11:30:00Z", kind: "todo", id: 2, task: null, key: null, step: null, lane: null, what: "Renew the TLS cert, due 2026-12-01T14:00:00Z", generation: null, snoozed_until: null, snoozed_at: null, back_early: null };
    const model = deskModel({ ...data, desk: { ...data.desk, items: [...data.desk.items, todo] } }, { now });
    const rows = model.sections("needs").find(({ title }) => title === "To-dos").rows;
    assert.deepEqual(rows.map((row) => row.text), ["Renew the TLS cert, due Dec 1, 09:00 (in 11 days)"]);
    assert.equal(dueText("Fix it, due 2026-11-21T14:00:00Z", now), "Fix it, due tomorrow 09:00");
    assert.equal(dueText("Fix it, due 2026-13-20T20:00:00Z", now), "Fix it, due 2026-13-20T20:00:00Z");
  } finally {
    process.env.TZ = "UTC";
  }
});

test("the room and intake banners, Since you looked and Up next come from the desk and the queue", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.banners, [
    "stalled 58 min ago: short of disk by 4.1 GB; task 7 waits for room",
    "the intake fails to poll github since 2026-11-20T10:58:00Z: gh api repos/owner/repo/issues/events?per_page=100&page=1 failed: HTTP 401: Bad credentials",
  ]);
  assert.equal(model.since.from, "09:12");
  assert.deepEqual(model.since.rows.slice(0, 2), [
    { key: "cli:PRI-66", task: 6, what: "run started triage", time: "58 min" },
    { key: "cli:PRI-63", task: 3, what: "step ended approve: merged #210", time: "1 h" },
  ]);
  assert.deepEqual(model.upNext, [
    { when: "now", key: "cli:PRI-72", task: 7, why: "first in the queue" },
    { when: "Nov 21, 02:00", key: "schedule:nightly", task: null, why: "schedule nightly fires" },
  ]);
});

test("the pause and stall banners say when in the web Desk's relative words, not a raw time", () => {
  const banners = (/** @type {any} */ fields) => deskModel({ ...data, desk: { ...data.desk, ...fields } }, { now }).banners.slice(0, 2);
  assert.deepEqual(banners({ paused: "paused since 2026-11-18T08:00:00Z: freeze since 2026-11-17T00:00:00Z", banner: "stalled since 2026-11-20T11:59:40Z" }), [
    "paused on Nov 18, 08:00: freeze since 2026-11-17T00:00:00Z",
    "stalled just now",
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

/** A stack entry as `tasks --json` lists it. @param {number} number @param {string} branch @param {string} base @param {string} state */
const pull = (number, branch, base, state) => ({ pull_request: `github:acme/app#${number}`, number, branch, base, state });

test("a task row with a stack of two or more says how many PRs; one PR or none says nothing", () => {
  const stack = [pull(57, "pri-66-a", "main", "merged"), pull(58, "pri-66-b", "pri-66-a", "open"), pull(59, "pri-66-c", "pri-66-b", "open")];
  const tasks = data.tasks.tasks.map((/** @type {any} */ task) => ({ ...task, stack: task.id === 6 ? stack : task.id === 4 ? [pull(60, "pri-55", "main", "open")] : [] }));
  const model = deskModel({ ...data, tasks: { tasks } }, { now });
  assert.deepEqual(model.sections("running")[0].rows.map((row) => [row.key, row.prs]), [["cli:PRI-66", "3 PRs"]]);
  assert.deepEqual(model.sections("needs")[1].rows.map((row) => [row.key, row.prs]), [["cli:PRI-55", null]]);
  assert.deepEqual(model.sections("all")[0].rows.map((row) => row.prs).filter(Boolean), ["3 PRs"]);
});

test("an ask routed to a lead shows with the lead in its row and as the next thing, and an unrouted row is unchanged", () => {
  const items = data.desk.items.map((item) => (item.kind === "ask" && item.id === 1 ? { ...item, routed_to: "lead" } : item));
  const desk = { ...data.desk, items, next: { ...data.desk.ranked[1], routed_to: "lead" } };
  const model = deskModel({ ...data, desk }, { now });
  const rows = model.sections("needs").flatMap((section) => section.rows).filter((row) => row.kind === "ask");

  assert.deepEqual(rows.map((row) => [row.id, row.routed]), [["ask:1", "with lead"], ["ask:4", undefined], ["ask:2", undefined]]);
  assert.equal(model.next.routed, "with lead");
  assert.equal("routed" in deskModel(data, { now }).next, false);
});

/** The Desk panel drawn from the fixtures, in Node, with the fake kit. */
async function deskPanel(links = /** @type {{ pattern: string, url: string }[]} */ ([])) {
  const { context, herdr, settle } = await import("./kit.js");
  const { default: Desk } = await import("../mods/priory-desk/main.js");
  herdr.reset();
  herdr.replies = { desk: data.desk, asks: data.asks, tasks: data.tasks, queue: data.queue, links: { links } };
  const panel = new Desk();
  const cx = context();
  panel.init({}, cx);
  await settle();
  return { panel, cx, herdr };
}

test("an Up next line opens its task in the Task panel, and a schedule's line opens nothing", async () => {
  const { draw, press } = await import("./kit.js");
  const { panel, cx, herdr } = await deskPanel();

  press(draw(panel, cx), "up-next-1");
  assert.equal(herdr.shared.get("priory.task.open"), undefined);
  press(draw(panel, cx), "up-next-0");
  assert.equal(herdr.shared.get("priory.task.open").task, 7);
});

test("Answer opens the ask in the Asks aside, shown", async () => {
  const { draw, press } = await import("./kit.js");
  const { layout } = await import("../lib/layout.js");
  const { panel, cx, herdr } = await deskPanel();
  const frame = JSON.parse(readFileSync(new URL("../layouts/frame.json", import.meta.url), "utf8"));
  const tabs = ["desk", "asks", "tasks"].map((panel) => ({ panel }));
  herdr.layout = layout("default", { layout: { tabs } }, frame);

  press(draw(panel, cx), "row-ask:2");
  press(draw(panel, cx), "answer-ask:2");

  assert.equal(herdr.shared.get("priory.asks.open").ask, 2);
  assert.deepEqual(herdr.runs, [["layout.show", { id: "aside-asks" }]]);
});

test("Answer opens an Asks tab when the view has no Asks block", async () => {
  const { draw, press } = await import("./kit.js");
  const { layout } = await import("../lib/layout.js");
  const { panel, cx, herdr } = await deskPanel();
  const frame = JSON.parse(readFileSync(new URL("../layouts/frame.json", import.meta.url), "utf8"));
  herdr.layout = layout("default", { layout: { panel: "desk" } }, frame);

  press(draw(panel, cx), "row-ask:2");
  press(draw(panel, cx), "answer-ask:2");

  assert.equal(herdr.shared.get("priory.asks.open").ask, 2);
  assert.deepEqual(herdr.runs, [["tab.new", { space: "space-1", name: "Asks", mod: "priory-asks" }]]);
});

test("Open task opens the Task panel, and Attach stays its own action", async () => {
  const { draw, press } = await import("./kit.js");
  const { panel, cx, herdr } = await deskPanel();

  press(draw(panel, cx), "next-open");
  assert.equal(herdr.shared.get("priory.task.open").task, 4);
  assert.equal(herdr.shared.get("priory.attach"), undefined);
  press(draw(panel, cx), "next-attach");
  assert.equal(herdr.shared.get("priory.attach").task, 4);

  press(draw(panel, cx), "row-ask:2");
  press(draw(panel, cx), "open-ask:2");
  assert.equal(herdr.shared.get("priory.task.open").task, 2);
});

test("a going batch is one Later row with its whole line, which no action or snooze takes", () => {
  const batch = {
    since: "2026-11-20T07:00:00Z", kind: "batch", id: 4, task: 30, key: "batch:2026-11-21-1", step: "close", lane: "billing/exports",
    what: "Invoice exports: 1 merged, 3 started, 2 planned. Next: tasks 36 and 37 start when task 34 merges.",
    generation: 1, snoozed_until: null, snoozed_at: null, back_early: null,
  };
  const desk = { ...data.desk, items: [...data.desk.items, batch] };
  const later = deskModel({ ...data, desk }, { now }).sections("needs")[2].rows;
  const row = later.find((/** @type {any} */ row) => row.kind === "batch");
  assert.deepEqual([row.key, row.text, row.recommends, row.snooze], [
    "batch:2026-11-21-1", "Invoice exports: 1 merged, 3 started, 2 planned. Next: tasks 36 and 37 start when task 34 merges.", null, null,
  ]);
});

test("a press on a row's link opens the link and leaves the selection where it was", async () => {
  const { draw, press } = await import("./kit.js");
  const { panel, cx } = await deskPanel([{ pattern: "backoff", url: "https://crates.io/crates/backoff" }]);
  const tree = draw(panel, cx);
  const before = panel.selected;

  press(tree, "text-ask:2");
  assert.notEqual(before, "ask:2");
  assert.equal(panel.selected, before);
  press(draw(panel, cx), "row-ask:2");
  assert.equal(panel.selected, "ask:2");
});

test("another runner's ask names its runner and when it was last seen, says what an answer sent to it became, and offers no snooze", () => {
  const since = new Date(now - 30 * 60_000).toISOString();
  const item = { kind: "ask", id: "desktop:12", task: null, key: "ENG-260", step: "approve-pr", since, what: "Merge?", runner: "desktop", last_seen: new Date(now - 40 * 60_000).toISOString(), snoozed_until: null };
  const ask = { id: "desktop:12", task: 4, key: "ENG-260", question: "Merge?", options: ["approved"], revision: 1, runner: "desktop", last_seen: item.last_seen, submissions: [{ verb: "answer", text: "approved", outcome: null }] };
  const model = deskModel({ ...data, desk: { ...data.desk, items: [item] }, asks: { ...data.asks, asks: [ask] } }, { now });
  const [row] = model.sections("needs").flatMap((section) => section.rows);

  assert.equal(row.ask, "desktop:12");
  assert.equal(row.runner, "desktop");
  assert.equal(row.away, "last seen 40 min ago");
  assert.deepEqual(row.sent, { text: "Sent to desktop: approved", refused: false });
  assert.equal(row.snooze, null);
});

test("another runner's ask as the next thing names its runner and offers no snooze", () => {
  const item = { kind: "ask", id: "desktop:12", task: null, key: "ENG-260", step: "approve-pr", since: new Date(now - 60_000).toISOString(), what: "Merge?", why: "oldest ask", runner: "desktop", last_seen: null, snoozed_until: null };
  const ask = { id: "desktop:12", task: 4, key: "ENG-260", question: "Merge?", options: ["approved"], revision: 1, runner: "desktop", last_seen: null, submissions: [] };
  const model = deskModel({ ...data, desk: { ...data.desk, items: [item], next: item }, asks: { ...data.asks, asks: [ask] } }, { now });

  assert.equal(model.next.snooze, null);
  assert.equal(model.next.runner, "desktop");
  assert.equal(model.next.away, "never seen");
});

test("the memories row lists each candidate with its full text, its tag and the text it replaces", () => {
  const item = { kind: "memories", id: "memories", what: "2 memory candidates wait for a person: release-day, kit-fact; see them with `priory memories`", since: "2026-11-20T10:00:00Z", candidates: [] };
  const desk = { ...data.desk, items: [...data.desk.items, item] };
  // `priory memories DIR --json`: each candidate with the memory of its name runs get now.
  const memories = { candidates: [
    { id: "c-1", name: "release-day", description: "When releases go out.", text: "Wednesdays.", forget: false, task: "ENG-212", runner: "laptop", replaces: true, current_text: "Tuesdays." },
    { id: "c-2", name: "kit-fact", description: null, text: null, forget: true, task: "ENG-224", runner: null, replaces: false, current_text: null },
    // Read without the factory folder, or where the kit is not checked out: unknown, never "new".
    { id: "c-3", name: "search-staging", description: "Which cluster.", text: "Use b.", forget: false, task: "ENG-224", runner: null },
  ] };
  const row = deskModel({ ...data, desk, memories }, { now }).sections("needs").flatMap((section) => section.rows).find((each) => each.kind === "memories");
  assert.equal(row.text, "3 memory candidates: release-day, kit-fact, search-staging");
  assert.equal(row.age, "2 h");
  assert.deepEqual(row.memories, [
    { id: "c-1", name: "release-day", tag: "replaces a memory runs get", warn: true, description: "When releases go out.", text: "Wednesdays.", current: "Tuesdays.", by: "Written by ENG-212 on laptop, candidate c-1" },
    { id: "c-2", name: "kit-fact", tag: "forget", warn: false, description: null, text: "Forgets kit-fact: runs stop getting it.", current: null, by: "Written by ENG-224, candidate c-2" },
    { id: "c-3", name: "search-staging", tag: "may replace a memory runs get", warn: true, description: "Which cluster.", text: "Use b.", current: null, by: "Written by ENG-224, candidate c-3" },
  ]);
});

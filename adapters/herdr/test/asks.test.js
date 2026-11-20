import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { askDetail, asksList, press, refusal, tomorrowAt } from "../mods/priory-asks/view.js";

process.env.TZ = "UTC";

// Captured from `priory asks --all --json` on a scratch factory, its asks seeded by SQL as
// crates/priory/tests/serve.rs seeds them. asks-revised.json is the same state after ask 1 moved
// to revision 3 and ask 2, snoozed, to revision 2.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));
const { asks } = fixture("asks.json");
const now = Date.parse("2026-11-20T20:50:00Z");
const ids = (section) => section.rows.map((row) => row.id);

test("the Open view lists the open asks that are not snoozed, oldest first, beside every view's count", () => {
  const list = asksList(asks, { view: "open", now });
  assert.deepEqual(list.views, [
    { id: "open", label: "Open", count: 2 },
    { id: "snoozed", label: "Snoozed", count: 1 },
    { id: "answered", label: "Answered", count: 1 },
    { id: "all", label: "All", count: 5 },
  ]);
  assert.deepEqual(list.sections.map((section) => [section.title, section.note, ids(section)]), [
    ["Open", "2, oldest first", [1, "review-7"]],
  ]);
});

test("an open row shows its tile, task, step, age, question and what it recommends", () => {
  const [open] = asksList(asks, { view: "open", now }).sections;
  assert.deepEqual(open.rows, [
    {
      id: 1, tile: "ask", key: "cli:PRI-docs-1", step: "implement", age: "10 h", command: true,
      text: "Merge #212 into main? Checks are green at a1f3c9e.", meta: ["Recommends", "Merge"], snoozed: null, backEarly: null,
    },
    {
      id: "review-7", tile: "ask", key: "REL-7", step: null, age: "5 min", command: true,
      text: "Ship the release notes as drafted?", meta: ["Recommends", "Ship"], snoozed: null, backEarly: null,
    },
  ]);
});

test("a snoozed ask leaves Open for Snoozed, and a new revision brings it back early", () => {
  const [snoozedSection] = asksList(asks, { view: "snoozed", now }).sections;
  assert.deepEqual([snoozedSection.title, snoozedSection.note, ids(snoozedSection)], ["Snoozed", "1", [2]]);
  assert.equal(snoozedSection.rows[0].snoozed, "Snoozed until Nov 21, 08:00");

  const revised = fixture("asks-revised.json").asks;
  const back = asksList(revised, { view: "snoozed", now });
  assert.deepEqual(back.views.map((view) => view.count), [3, 0, 1, 5]);
  assert.deepEqual(back.sections.map((section) => [section.title, section.note, ids(section)]), [["Back early today", "1", [2]]]);
  assert.equal(back.sections[0].rows[0].backEarly, "Back early: new revision 2");
  assert.equal(asksList(revised, { view: "open", now }).sections[0].rows[1].backEarly, "Back early: new revision 2");
});

test("Answered splits today from earlier, newest first, and All adds the snoozed and withdrawn asks", () => {
  // Hand-built: the captured answered ask, answered two days earlier.
  const earlier = { ...asks[2], id: 9, answered_at: "2026-11-18T14:02:00Z" };
  const answered = asksList([...asks, earlier], { view: "answered", now }).sections;
  assert.deepEqual(answered.map((section) => [section.title, ids(section)]), [["Answered today", [3]], ["Answered earlier", [9]]]);
  assert.deepEqual([answered[0].rows[0].tile, answered[0].rows[0].age, answered[0].rows[0].meta], ["answered", "09:41", ["Answered", "Per revision"]]);
  assert.equal(answered[1].rows[0].age, "Nov 18, 14:02");

  const all = asksList(asks, { view: "all", now }).sections;
  assert.deepEqual(all.map((section) => [section.title, ids(section)]), [
    ["Open", [1, 2, "review-7"]],
    ["Answered today", [3]],
    ["Withdrawn", [4]],
  ]);
  assert.deepEqual([all[2].rows[0].tile, all[2].rows[0].meta], ["withdrawn", ["Withdrawn", "revived as generation 2"]]);
});

test("facets narrow every view and offer each lane and task with its count", () => {
  const byLane = asksList(asks, { view: "all", now, facets: { lane: "none/misc" } });
  assert.deepEqual(byLane.views.map((view) => view.count), [1, 1, 1, 4]);
  assert.deepEqual(asksList(asks, { view: "open", now, facets: { task: "REL-7" } }).sections.map(ids), [["review-7"]]);
  assert.deepEqual(asksList(asks, { view: "all", now, facets: { command: true } }).sections.map(ids), [[1, "review-7"]]);
  assert.deepEqual(byLane.facets, {
    lane: [["none/misc", 4]],
    task: [["cli:PRI-docs-1", 1], ["cli:PRI-docs-2", 1], ["cli:PRI-docs-3", 2], ["REL-7", 1]],
  });
});

test("an open command ask's detail puts the decision first, then Why, History and the command word by word", () => {
  const detail = askDetail(asks[0], { now, armed: null });
  assert.deepEqual(detail.context, ["cli:PRI-docs-1", "implement", "none/misc", "Command", "10 h", "revision 2"]);
  assert.equal(detail.question, "Merge #212 into main? Checks are green at a1f3c9e.");
  assert.deepEqual(detail.recommendation, { text: "Recommended: Merge.", next: "Then the agent squash-merges #212 and watches the checks." });
  assert.deepEqual(detail.options, [
    { index: 0, label: "Merge", key: "1", recommended: true, notes: "Recommended. Runs a command", armed: false, dimmed: false },
    { index: 1, label: "Wait for a second review", key: "2", recommended: false, notes: "", armed: false, dimmed: false },
    { index: 2, label: "Close the pull request", key: "3", recommended: false, notes: "", armed: false, dimmed: false },
  ]);
  assert.deepEqual(detail.chips, [["why", "Why"], ["history", "History"], ["command", "Command"]]);
  assert.deepEqual(detail.why, {
    text: "I read the diff of #212 and ran `gh pr checks 212`: fmt, clippy and test are green.\n<b>not html</b>",
    run: null,
    tradeoff: "Merging now ships the exporter today. A second review may still ask for changes.",
    whatWaits: "PRI-64 rebases on this merge.",
  });
  assert.deepEqual(detail.history, [{ tile: "ask", text: "Asked revision 2", time: "10:00" }]);
  assert.deepEqual(detail.command, {
    title: "Command an answer runs",
    words: ["gh", "pr", "merge", "212", "--squash", "--match-head-commit", "a1f3c9e", ""],
    note: "In /home/me/wt/pri-61 (worktree). Runs on Merge. Bound to revision 2. Runs at most once.",
  });
  assert.deepEqual(detail.bar, { count: "3 options", option: 0 });
  assert.equal(detail.canSnooze, true);
});

test("a closed ask shows what happened and offers no buttons", () => {
  const answered = askDetail(asks[2], { now, armed: null });
  assert.equal(answered.decided, "Answered: Per revision. 11 h ago.");
  assert.deepEqual([answered.options, answered.bar, answered.canSnooze], [[], null, false]);
  assert.deepEqual(answered.history, [
    { tile: "answered", text: "Answered Per revision", time: "09:41" },
    { tile: "ask", text: "Asked revision 1", time: "09:00" },
  ]);
  const withdrawn = askDetail(asks[3], { now, armed: null });
  assert.equal(withdrawn.decided, "Withdrawn: revived as generation 2.");
  assert.deepEqual(withdrawn.history[0], { tile: "withdrawn", text: "Withdrawn: revived as generation 2", time: "" });
});

test("an answer that ran the command shows how it went", () => {
  // Hand-built from serve.md's `ran` and the #94 tests: the command ask, answered with --run.
  const ran = (status, error = null) => askDetail(
    { ...asks[0], state: "answered", answer: "Merge", answered_at: "2026-11-20T20:40:00Z", ran: { status, revision: 2, error, stdout: "", stderr: "" } },
    { now, armed: null },
  ).ran;
  assert.deepEqual(ran("running"), { text: "The command runs.", warn: false });
  assert.deepEqual(ran("done"), { text: "The command ran: exit 0.", warn: false });
  assert.deepEqual(ran("failed", "exit 1: no such pull request"), { text: "The command failed: exit 1: no such pull request.", warn: false });
  assert.deepEqual(ran("uncertain"), { text: "Priory stopped while this ran. Check the target before you act on it.", warn: true });
  assert.equal(askDetail(asks[0], { now, armed: null }).ran, null);
});

test("a command option arms on the first press and runs on a second press within 4 s", () => {
  const [command, outside] = [asks[0], asks[4]];
  assert.deepEqual(press(command, 1, null, now), { answer: { text: "Wait for a second review", run: false } });
  const first = press(command, 0, null, now);
  assert.deepEqual(first, { arm: { id: 1, revision: 2, option: 0, at: now } });
  assert.deepEqual(press(command, 0, first.arm, now + 3_999), { answer: { text: "Merge", run: true } });
  assert.deepEqual(press(command, 0, first.arm, now + 4_000), { arm: { id: 1, revision: 2, option: 0, at: now + 4_000 } });
  assert.deepEqual(press({ ...command, revision: 3 }, 0, first.arm, now + 10), { arm: { id: 1, revision: 3, option: 0, at: now + 10 } });
  // An outside ask's command is a string that never runs, so its options answer at once.
  assert.deepEqual(press(outside, 0, null, now), { answer: { text: "Ship", run: false } });

  const armed = askDetail(command, { now, armed: first.arm }).options;
  assert.deepEqual(armed.map((option) => [option.armed, option.dimmed]), [[true, false], [false, true], [false, true]]);
});

test("a refused answer says why, from the ask as Priory lists it after the refusal", () => {
  const revised = fixture("asks-revised.json").asks;
  const error = "priory answer: ask 1 is at revision 3, not 2";
  assert.equal(refusal(asks[0], revised[0], error, now), "The question changed. Read revision 3 and answer again.");
  // Hand-built: the same ask after another answer, or after a revive withdrew it.
  assert.equal(refusal(asks[0], { ...asks[0], state: "answered", answer: "Merge", answered_at: "2026-11-20T20:48:00Z" }, error, now),
    "Already answered 2 min ago: Merge.");
  assert.equal(refusal(asks[0], { ...asks[0], state: "withdrawn", withdrawn_why: "dismissed" }, error, now), "Withdrawn: dismissed.");
  assert.equal(refusal(asks[0], asks[0], "priory answer: no priory daemon listens on /run/priory.sock", now),
    "Not sent: priory answer: no priory daemon listens on /run/priory.sock. The draft stays here.");
  assert.equal(refusal(asks[0], undefined, "priory answer: no ask 1", now), "Not sent: priory answer: no ask 1. The draft stays here.");
});

test("snooze until tomorrow names 09:00 local time on the next day", () => {
  assert.equal(tomorrowAt(9, now), "2026-11-21T09:00:00.000Z");
  assert.equal(tomorrowAt(9, Date.parse("2026-11-30T23:59:00Z")), "2026-12-01T09:00:00.000Z");
});

// Captured from this branch's `priory asks --all --json` and `priory task 1 --json` on a scratch
// state seeded by SQL: an ask revised once, a stale answer refused, then answered; the task has a
// groom and a plan result, a pull request with two checks, and three runs.
test("History lists every revision and refused answer of the ask, newest first", () => {
  const [ask] = fixture("asks-history.json").asks;
  assert.deepEqual(askDetail(ask, { now, armed: null }).history, [
    { tile: "answered", text: "Answered Merge", time: "10:07" },
    { tile: "refused", text: "Refused: Wait, sent for revision 1 from cli: ask 1 is at revision 2, not 1", time: "10:06" },
    { tile: "ask", text: "Asked revision 2: Merge #212 into main? Checks are green.", time: "10:05" },
    { tile: "ask", text: "Asked revision 1: Merge #212 into main?", time: "09:50" },
  ]);
});

test("Brief, Plan, PR and Trace come from the task's detail, each left out when the task lacks it", () => {
  const [ask] = fixture("asks-history.json").asks;
  const task = fixture("task.json");
  const detail = askDetail(ask, { now, armed: null, task });
  assert.deepEqual(detail.chips, [["why", "Why"], ["brief", "Brief"], ["plan", "Plan"], ["pr", "PR"], ["trace", "Trace"], ["history", "History"]]);
  assert.deepEqual(detail.brief, {
    request: "Export the board as CSV.\n\nKeep the column order.",
    text: "## Goal\nExport the board as CSV.\n\n## Scope\nOne CSV per board.",
    from: "groom, run 1",
  });
  assert.deepEqual(detail.plan, { text: "- [x] Write the exporter\n- [ ] Merge #212", from: "plan, run 2" });
  assert.deepEqual(detail.pr, {
    title: "#212 Export the board as CSV",
    line: "pri-61 into main, head a1f3c9e, open",
    checks: [{ name: "clippy", ok: false, text: "failure" }, { name: "test", ok: true, text: "success" }],
    review: null,
  });
  assert.deepEqual(detail.trace, [
    { tile: "moved", label: "groom, run 1", span: "08:00 to 08:20, 20 min", outcome: "yellow", tokens: null },
    { tile: "moved", label: "plan, run 2", span: "08:20 to 08:40, 20 min", outcome: "planned", tokens: "6000 tokens" },
    { tile: "asleep", label: "implement, run 3", span: "08:40 to 09:50, 70 min", outcome: null, tokens: null },
  ]);

  const bare = askDetail(ask, { now, armed: null, task: { ...task, brief: { request: null, result: null }, plan: null, progress: null, pull_request: null, runs: [] } });
  assert.deepEqual(bare.chips, [["why", "Why"], ["history", "History"]]);
});

test("Plan falls back to the task's progress when no plan step ran", () => {
  const [ask] = fixture("asks-history.json").asks;
  const progress = { goal: "Export the board", acceptanceCriteria: [{ id: "AC1", description: "a CSV per board", required: true }], latestChange: "wrote the exporter", nextAction: "merge" };
  const detail = askDetail(ask, { now, armed: null, task: { ...fixture("task.json"), plan: null, progress } });
  assert.deepEqual(detail.plan, { text: "Goal: Export the board\n- AC1: a CSV per board\nLatest change: wrote the exporter\nNext: merge", from: "progress" });
});

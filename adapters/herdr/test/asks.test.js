import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { askDetail, asksList, press, refusal, tomorrowAt } from "../mods/priory-asks/view.js";

process.env.TZ = "UTC";

// Captured from `priory asks --all --json` on a scratch factory, its asks seeded by SQL as
// crates/priory/tests/serve.rs seeds them. asks-revised.json is the same state after ask 1 moved
// to revision 3 and ask 2, snoozed, to revision 2. Each ask's `risk` is the one the CLI lists with
// task 2 graded red and task 3 green.
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

test("Open, Snoozed and All list open asks oldest first, another runner's among this runner's, as the web since #318", () => {
  // Priory lists this runner's asks, then each other runner's, so a runner's older ask comes last.
  const mixed = [
    { ...asks[0], id: 1, since: "2026-11-20T11:00:00Z" },
    { ...asks[0], id: "laptop:4", runner: "laptop", submissions: [], since: "2026-11-20T10:00:00+01:00" },
    { ...asks[0], id: "laptop:5", runner: "laptop", submissions: [], since: "2026-11-20T10:30:00Z" },
  ];
  assert.deepEqual(asksList(mixed, { view: "open", now }).sections.map(ids), [["laptop:4", "laptop:5", 1]]);
  assert.deepEqual(asksList(mixed, { view: "all", now }).sections.map(ids), [["laptop:4", "laptop:5", 1]]);
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
  assert.deepEqual([all[2].rows[0].tile, all[2].rows[0].meta], ["withdrawn", ["Withdrawn", "its task was revived"]]);
});

test("facets narrow every view and offer each lane and task with its count", () => {
  const byLane = asksList(asks, { view: "all", now, facets: { lane: "none/misc" } });
  assert.deepEqual(byLane.views.map((view) => view.count), [1, 1, 1, 4]);
  assert.deepEqual(asksList(asks, { view: "open", now, facets: { task: "REL-7" } }).sections.map(ids), [["review-7"]]);
  assert.deepEqual(asksList(asks, { view: "all", now, facets: { command: true } }).sections.map(ids), [[1, "review-7"]]);
  assert.deepEqual(byLane.facets, {
    lane: [["none/misc", 4]],
    band: [["now", 1], ["later", 1], ["blocker", 2], ["none", 1]],
    risk: [["irreversible", 1], ["red", 1], ["green", 2], ["none", 1]],
    task: [["cli:PRI-docs-1", 1], ["cli:PRI-docs-2", 1], ["cli:PRI-docs-3", 2], ["REL-7", 1]],
    runner: [],
  });
});

test("the band facet narrows by the task's band, offers bands in the factory's order, and puts an outside ask under none", () => {
  const { bands } = fixture("asks.json");
  assert.deepEqual(asksList(asks, { view: "all", now, bands }).facets.band, [["blocker", 2], ["now", 1], ["later", 1], ["none", 1]]);
  assert.deepEqual(asksList(asks, { view: "all", now, bands, facets: { band: "now" } }).sections.map(ids), [[1]]);
  assert.deepEqual(asksList(asks, { view: "all", now, bands, facets: { band: "none" } }).sections.map(ids), [["review-7"]]);
  assert.deepEqual(asksList(asks, { view: "all", now, bands, facets: { band: "blocker" } }).views.map((view) => view.count), [0, 0, 1, 2]);
});

test("the risk facet narrows by any of an ask's risks, offers them irreversible, red, yellow, green, none, and puts an ask with none under none", () => {
  assert.deepEqual(asksList(asks, { view: "all", now }).facets.risk, [["irreversible", 1], ["red", 1], ["green", 2], ["none", 1]]);
  assert.deepEqual(asksList(asks, { view: "all", now, facets: { risk: "irreversible" } }).sections.map(ids), [[1]]);
  assert.deepEqual(asksList(asks, { view: "all", now, facets: { risk: "green" } }).sections.map(ids), [[3], [4]]);
  assert.deepEqual(asksList(asks, { view: "all", now, facets: { risk: "none" } }).sections.map(ids), [["review-7"]]);
});

test("an open command ask's detail puts the decision first, then Why, History and the command word by word", () => {
  const detail = askDetail(asks[0], { now, armed: null });
  assert.deepEqual(detail.context, ["cli:PRI-docs-1", "implement", "none/misc", "Command", "10 h", "revision 2"]);
  assert.equal(detail.question, "Merge #212 into main? Checks are green at a1f3c9e.");
  assert.deepEqual(detail.recommendation, { text: "Recommended: Merge.", next: "Then the agent squash-merges #212 and watches the checks." });
  assert.deepEqual(detail.options, [
    { index: 0, label: "Merge", key: "1", recommended: true, notes: "Recommended. Runs a command", runs: true, armed: false, dimmed: false },
    { index: 1, label: "Wait for a second review", key: "2", recommended: false, notes: "", runs: false, armed: false, dimmed: false },
    { index: 2, label: "Close the pull request", key: "3", recommended: false, notes: "", runs: false, armed: false, dimmed: false },
  ]);
  assert.deepEqual(detail.chips, [["why", "Why"], ["history", "History"], ["command", "Command"]]);
  assert.deepEqual(detail.why, {
    question: null,
    text: "I read the diff of #212 and ran `gh pr checks 212`: fmt, clippy and test are green.\n<b>not html</b>",
    run: null,
    tradeoff: "Merging now ships the exporter today. A second review may still ask for changes.",
    whatWaits: "PRI-64 rebases on this merge.",
  });
  assert.deepEqual(detail.history, [{ tile: "ask", text: "Asked revision 2", time: "10:00" }]);
  assert.deepEqual(detail.command, {
    title: "Command an answer runs",
    words: ["gh", "pr", "merge", "212", "--squash", "--match-head-commit", "a1f3c9e", ""],
    note: "In /home/me/wt/pri-61 (worktree). Runs when answered Merge. Bound to revision 2. Runs at most once.",
    line: "gh pr merge 212 --squash --match-head-commit a1f3c9e ''",
  });
  // Copy gives a shell line: a word with a space or a quote stays one word.
  const quoted = askDetail({ ...asks[0], command: { ...asks[0].command, argv: ["echo", "it's done", "a b"] } }, { now, armed: null });
  assert.equal(quoted.command?.line, `echo 'it'\\''s done' 'a b'`);
  // An outside ask's command is one string, copied as it is.
  assert.equal(askDetail(asks[4], { now, armed: null }).command?.line, asks[4].command);
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
  assert.equal(withdrawn.decided, "Withdrawn: its task was revived.");
  assert.deepEqual(withdrawn.history[0], { tile: "withdrawn", text: "Withdrawn: its task was revived", time: "" });
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

test("every option arms on the first press and answers on a second press within 4 s", () => {
  const [command, outside] = [asks[0], asks[4]];
  const plain = press(command, 1, null, now);
  assert.deepEqual(plain, { arm: { id: 1, revision: 2, option: 1, at: now } }, "the first press arms and answers nothing");
  assert.deepEqual(press(command, 1, plain.arm, now + 249), {}, "a second press within SETTLE_MS does nothing");
  assert.deepEqual(press(command, 1, plain.arm, now + 250), { answer: { text: "Wait for a second review", run: false } }, "a second press after it answers at once");
  assert.deepEqual(press(command, 1, plain.arm, now + 3_999), { answer: { text: "Wait for a second review", run: false } }, "a second press answers at once");
  assert.deepEqual(press(command, 1, plain.arm, now + 4_000), { arm: { id: 1, revision: 2, option: 1, at: now + 4_000 } }, "a press after ARM_MS arms again");
  assert.deepEqual(press(command, 2, plain.arm, now + 1_000), { arm: { id: 1, revision: 2, option: 2, at: now + 1_000 } }, "a press on another option moves the arm");
  assert.deepEqual(press({ ...command, revision: 3 }, 1, plain.arm, now + 10), { arm: { id: 1, revision: 3, option: 1, at: now + 10 } }, "a new revision drops the arm");

  // A command option runs its command on the second press.
  const first = press(command, 0, null, now);
  assert.deepEqual(first, { arm: { id: 1, revision: 2, option: 0, at: now } });
  assert.deepEqual(press(command, 0, first.arm, now + 3_999), { answer: { text: "Merge", run: true } });
  // An outside ask's command is a string that never runs.
  const ship = press(outside, 0, null, now);
  assert.deepEqual(press(outside, 0, ship.arm, now + 300), { answer: { text: "Ship", run: false } });

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
  // Under a minute, and past a day, in the web view's words.
  assert.equal(refusal(asks[0], { ...asks[0], state: "answered", answer: "Merge", answered_at: new Date(now - 20_000).toISOString() }, error, now),
    "Already answered just now: Merge.");
  assert.equal(askDetail({ ...asks[2], answered_at: "2026-11-18T08:00:00Z" }, { now, armed: null }).decided, "Answered: Per revision. On Nov 18, 08:00.");
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
// state seeded by SQL: an ask answered, revised, a stale answer refused, then answered; the task has a
// groom and a plan result, a pull request with two checks, and three runs.
test("History lists every revision and refused answer of the ask, newest first", () => {
  const [ask] = fixture("asks-history.json").asks;
  assert.deepEqual(askDetail(ask, { now, armed: null }).history, [
    { tile: "answered", text: "Answered Merge", time: "10:07" },
    { tile: "refused", text: "Refused: Close, sent for revision 1 from cli: ask 1 is at revision 2, not 1", time: "10:06" },
    { tile: "ask", text: "Asked revision 2: Merge #212 into main? Checks are green.", time: "10:05" },
    { tile: "answered", text: "Answered Wait", time: "09:55" },
    { tile: "ask", text: "Asked revision 1: Merge #212 into main?", time: "09:50" },
  ]);
});

test("Brief, Plan, PR and Trace come from the task's detail, each left out when the task lacks it", () => {
  const [ask] = fixture("asks-history.json").asks;
  const task = fixture("task.json");
  const detail = askDetail(ask, { now, armed: null, task });
  assert.deepEqual(detail.chips, [["why", "Why"], ["plan", "Plan"], ["brief", "Brief"], ["pr", "PR"], ["trace", "Trace"], ["history", "History"]]);
  assert.deepEqual(detail.brief, {
    request: "Export the board as CSV.\n\nKeep the column order.",
    text: "## Goal\nExport the board as CSV.\n\n## Scope\nOne CSV per board.",
    from: "groom, run 1",
  });
  assert.deepEqual(detail.plan, [{ text: "- [x] Write the exporter\n- [ ] Merge #212", from: "plan, run 2" }]);
  assert.deepEqual(detail.pr, {
    title: "#212 Export the board as CSV",
    line: "pri-61 into main, head a1f3c9e, open",
    checks: [{ name: "clippy", state: "failed", text: "failure" }, { name: "test", state: "passed", text: "passed" }],
    review: null,
  });
  // As the web's task page: a check with no conclusion is running, a skipped one is muted, and states read in plain words.
  const pending = { ...task.pull_request, review: "changes_requested", checks: [{ name: "e2e", conclusion: null }, { name: "docs", conclusion: "neutral" }] };
  const unfinished = askDetail(ask, { now, armed: null, task: { ...task, pull_request: pending } }).pr;
  assert.deepEqual([unfinished?.checks, unfinished?.review], [[{ name: "e2e", state: "running", text: "running" }, { name: "docs", state: "skipped", text: "neutral" }], "changes requested"]);
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
  assert.deepEqual(detail.plan, [{ text: "Goal: Export the board\n- AC1: a CSV per board\nLatest change: wrote the exporter\nNext: merge", from: "progress" }]);
});

const approval = () => {
  const [answered] = fixture("asks-history.json").asks;
  const step = { ...answered, state: "open", kind: "step", options: ["approve", "revise"], approves_plan: true };
  const plan_file = { ask: step.id, revision: step.revision, step: "groom", run: 2, files: [{ path: "/home/me/worktrees/acme/shop/7/.priory/brief.md", text: "## Decisions\nD1: cursor", truncated: false }] };
  return { answered, step, plan_file, task: { ...fixture("task.json"), plan_file } };
};

test("Plan shows the plan an open approval ask approves, right after Why", () => {
  const { answered, step, plan_file, task } = approval();
  const detail = askDetail(step, { now, armed: null, task });
  assert.deepEqual(detail.plan, [{ text: "## Decisions\nD1: cursor", from: ".priory/brief.md" }]);
  assert.equal(detail.first, "why");
  assert.equal(detail.locked, false);
  assert.deepEqual(detail.chips.map(([id]) => id), ["why", "plan", "brief", "pr", "trace", "history"]);

  const [file] = plan_file.files;
  const two = askDetail(step, { now, armed: null, task: { ...task, plan_file: { ...plan_file, files: [{ ...file, path: "/w/.priory/plan.md", text: "P" }, { ...file, truncated: true }] } } });
  assert.deepEqual(two.plan, [{ text: "P", from: ".priory/plan.md" }, { text: `${file.text}\n\nShowing the first 64 KB. Read the rest at ${file.path}.`, from: ".priory/brief.md" }]);
  assert.equal(two.cut, `The plan is cut at 64 KB. Read the rest at ${file.path} before you answer.`);
  assert.equal(detail.cut, null);
  assert.equal(askDetail({ ...answered, kind: "step", approves_plan: true }, { now, armed: null, task }).chips[0][0], "why");
});

test("an approval ask shows no plan and takes no answer until the plan of its revision is read", () => {
  const { step, task } = approval();
  for (const read of [null, { ...task, plan_file: null }]) assert.equal(askDetail(step, { now, armed: null, task: read }).locked, true);
  const raised = askDetail({ ...step, revision: step.revision + 1 }, { now, armed: null, task });
  assert.equal(raised.locked, true);
  assert.equal(raised.plan, null, "the plan of the earlier revision is not shown");
});

test("next and previous below 720 px open an approval ask on Why, and an option waits for the plan", async () => {
  const { context, draw, herdr, press: tap, settle } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  const { step, task } = approval();
  const before = { ...step, id: 901, since: "2026-11-01T00:00:00Z", approves_plan: false };
  const approving = { ...step, id: 902, since: "2026-11-02T00:00:00Z", task: step.task };
  const after = { ...before, id: 903, since: "2026-11-03T00:00:00Z" };
  herdr.reset();
  herdr.replies = { asks: { revision: 1, bands: [], asks: [before, approving, after] }, task: { ...task, plan_file: { ...task.plan_file, ask: 902 } }, links: { links: [] } };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  panel.select(901, cx);
  panel.reading = "why";
  tap(draw(panel, cx), "next");
  assert.deepEqual([panel.selected, panel.reading], [902, "why"]);
  panel.task = null;
  panel.choose(approving, 0, cx);
  assert.equal(panel.armed, null, "no plan was read");
  await settle();
  panel.choose(approving, 0, cx);
  assert.equal(panel.armed?.option, 0);
  tap(draw(panel, cx), "next");
  assert.deepEqual([panel.selected, panel.reading], [903, "why"]);
  tap(draw(panel, cx), "previous");
  assert.deepEqual([panel.selected, panel.reading], [902, "why"]);
});

test("History keeps entries of the same second newest first", () => {
  // Hand-built from the captured ask: both revisions and the first answer within one second.
  const [ask] = fixture("asks-history.json").asks;
  const at = "2026-11-20T10:00:00Z";
  const same = { ...ask, refused: [], revisions: ask.revisions.map((revision) => ({ ...revision, at, answered_at: revision.answered_at && at })) };
  assert.deepEqual(askDetail(same, { now, armed: null }).history.map((item) => item.text), [
    "Answered Merge", "Asked revision 2: Merge #212 into main? Checks are green.", "Answered Wait", "Asked revision 1: Merge #212 into main?",
  ]);
});

test("an ask's question in its detail turns the factory's keys into link spans", () => {
  const { links } = fixture("links.json");
  const ask = { ...asks[0], repo: "acme/app" };
  const pr = [{ text: "Merge " }, { text: "#212", url: "https://github.com/acme/app/issues/212" }, { text: " into main? Checks are green at a1f3c9e." }];
  assert.deepEqual(askDetail(ask, { now, armed: null, links }).questionSpans, pr);
});

test("Why draws the agent's text as escaped Markdown, with the factory's links and the ask's repository", async () => {
  const { context, draw, find, herdr, settle } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  const { links } = fixture("links.json");
  const ask = { ...asks[0], repo: "acme/app", text: "Fixes ENG-7 and #3.\n\n- one\n- <b>two</b>" };
  herdr.reset();
  herdr.replies = { asks: { revision: 1, bands: [], asks: [ask] }, links: { links } };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  panel.select(ask.id, cx);
  panel.reading = "why";
  assert.equal(find(draw(panel, cx), `why-${ask.id}`).html,
    '<p>Fixes <a href="https://linear.app/acme/issue/ENG-7">ENG-7</a> and <a href="https://github.com/acme/app/issues/3">#3</a>.</p><ul><li>one</li><li>&lt;b&gt;two&lt;/b&gt;</li></ul>');
});

test("a task's stack of pull requests lists bottom first in PR, each with its state, the checks it has and the next to merge", () => {
  const [ask] = fixture("asks-history.json").asks;
  const task = fixture("task.json");
  const pull = (number, branch, base, state) => ({ pull_request: `github:acme/board#${number}`, number, branch, base, state });
  const stack = [pull(210, "pri-61-a", "main", "merged"), pull(211, "pri-61-b", "pri-61-a", "open"), pull(212, "pri-61", "pri-61-b", "open")];
  const detail = askDetail(ask, { now, armed: null, task: { ...task, stack } });
  assert.equal(detail.pr.title, "#212 Export the board as CSV");
  assert.equal(detail.pr.prs, "3 PRs");
  assert.deepEqual(detail.pr.stack, [
    { number: 210, url: "https://github.com/acme/board/pull/210", branch: "pri-61-a", base: "main", state: "merged", next: false, checks: null },
    { number: 211, url: "https://github.com/acme/board/pull/211", branch: "pri-61-b", base: "pri-61-a", state: "open", next: true, checks: null },
    { number: 212, url: "https://github.com/acme/board/pull/212", branch: "pri-61", base: "pri-61-b", state: "open", next: false, checks: "1 of 2 checks failing" },
  ]);

  const single = askDetail(ask, { now, armed: null, task: { ...task, stack: [pull(212, "pri-61", "main", "open")] } });
  assert.deepEqual(Object.keys(single.pr), ["title", "line", "checks", "review"]);
});

test("a stack shows in PR even before the latest pull request has events to summarize", () => {
  const [ask] = fixture("asks-history.json").asks;
  const pull = (number, base) => ({ pull_request: `github:acme/board#${number}`, number, branch: `b${number}`, base, state: "open" });
  const detail = askDetail(ask, { now, armed: null, task: { ...fixture("task.json"), pull_request: null, stack: [pull(211, "main"), pull(212, "b211")] } });
  assert.ok(detail.chips.some(([id]) => id === "pr"));
  assert.equal(detail.pr.title, "2 PRs");
  assert.deepEqual(detail.pr.stack.map((each) => [each.number, each.next]), [[211, true], [212, false]]);
});

const lead = (fields) => ({ ...asks[0], routed_to: "lead", ...fields });

test("an open ask routed to a lead says it is with the lead, and an unrouted ask has no such mark", () => {
  const [routed, plain] = asksList([lead(), asks[4]], { view: "open", now }).sections[0].rows;
  assert.equal(routed.routed, "with lead");
  assert.equal("routed" in plain, false);
});

test("an answered row names who answered, a person or the lead", () => {
  const answered = (by) => ({ ...lead({ state: "answered", answer: "Merge", answered_at: "2026-11-20T20:00:00Z", answered_by: by }) });
  const rows = asksList([answered("agent:lead"), { ...answered("cli"), id: 9 }, { ...answered(null), id: 10 }], { view: "answered", now }).sections.flatMap((section) => section.rows);
  assert.deepEqual(rows.map((row) => [row.by, row.routed]), [["agent:lead", undefined], ["a person", undefined], [undefined, undefined]]);
});

test("a routed ask's detail says the lead is answering and that the person can answer first", () => {
  const detail = askDetail(lead(), { now, armed: null });
  assert.equal(detail.routed, "lead is answering. You can still answer first: the first answer stands.");
  assert.equal("routed" in askDetail(asks[4], { now, armed: null }), false);
});

test("an answered ask's detail and history say who answered", () => {
  const ask = lead({
    state: "answered", answer: "Merge", answered_at: "2026-11-20T20:00:00Z", answered_by: "agent:lead", revision: 2,
    revisions: [
      { revision: 1, question: "q", at: "2026-11-20T10:00:00Z", answer: "Wait", answered_at: "2026-11-20T11:00:00Z" },
      { revision: 2, question: "q", at: "2026-11-20T12:00:00Z", answer: "Merge", answered_at: "2026-11-20T20:00:00Z" },
    ],
  });
  const detail = askDetail(ask, { now, armed: null });
  assert.match(detail.decided, /^Answered: Merge\. 50 min ago\. By agent:lead\.$/);
  assert.deepEqual(detail.history.filter((entry) => entry.tile === "answered").map((entry) => entry.text), ["Answered Merge by agent:lead", "Answered Wait"]);
  assert.equal("routed" in detail, false);
});

test("an ask the Desk opens with priory.asks.open opens here, in a view that shows it, once", async () => {
  const { context, draw, herdr, settle } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  herdr.reset();
  herdr.replies = { asks: fixture("asks.json"), task: fixture("task.json"), links: { links: [] } };
  // A request left by an earlier session opens nothing.
  herdr.shared.set("priory.asks.open", { ask: "review-7", at: Date.now() - 60_000 });
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  draw(panel, cx);
  assert.equal(panel.selected, 1);

  // Ask 2 is snoozed, so the Open view hides it and All shows it.
  herdr.shared.set("priory.asks.open", { ask: 2, at: Date.now() });
  draw(panel, cx);
  await settle();
  assert.deepEqual([panel.selected, panel.view], [2, "all"]);

  panel.select(1, cx);
  draw(panel, cx);
  await settle();
  assert.equal(panel.selected, 1);
});

test("the panel answers an option on its second press or key, with no Undo wait", async () => {
  const { context, draw, find, herdr, press: click, settle, text } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  herdr.reset();
  herdr.replies = { asks: fixture("asks.json"), task: fixture("task.json"), links: { links: [] }, answer: {} };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  draw(panel, cx);
  assert.equal(panel.selected, 1);
  const answers = () => herdr.argv.filter((argv) => argv.includes("answer"));

  click(draw(panel, cx), "option-1-1");
  assert.deepEqual(answers(), [], "the first press sends nothing");
  const armed = draw(panel, cx);
  assert.match(text(find(armed, "option-1-1")), /Press 2 again or click to send Confirm: Wait for a second review Cancels in 4 s/, "the armed option reads Confirm");
  assert.equal(herdr.timers.length > 0, true);

  // A double-click's second press does nothing; the arm stays.
  click(armed, "option-1-1");
  assert.deepEqual(answers(), []);
  assert.notEqual(panel.armed, null);
  /** @type {any} */ (panel.armed).at -= 300;
  click(draw(panel, cx), "option-1-1");
  await settle();
  assert.equal(answers().length, 1, "the second press sends at once, with no timer to wait out");
  assert.deepEqual(answers()[0].slice(-4), ["2", "--", "1", "Wait for a second review"]);

  // The answered ask leaves, and the next opens. Its keys work the same way: 1 arms, 1 again sends.
  assert.equal(panel.selected, "review-7");
  herdr.argv = [];
  panel.key({ key: "1" }, cx);
  assert.deepEqual(answers(), []);
  panel.key({ key: "1", is_held: true }, cx);
  assert.deepEqual(answers(), [], "a held key does not repeat into a send");
  /** @type {any} */ (panel.armed).at -= 300;
  panel.key({ key: "1" }, cx);
  await settle();
  assert.deepEqual(answers()[0].slice(-2), ["review-7", "Ship"]);
});

test("every open ask takes a written answer; a step's ask offers no dismissal, and an agent's ask does", () => {
  const step = askDetail({ ...asks[0], dismissable: false }, { now, armed: null });
  assert.equal(step.takesText, true);
  assert.equal(step.dismissable, false);
  assert.equal(askDetail({ ...asks[0], dismissable: true }, { now, armed: null }).dismissable, true);
});

test("the Attach menu's Open task opens the ask's task in the Task panel", async () => {
  const { context, draw, find, herdr, press, settle } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  herdr.reset();
  herdr.replies = { asks: fixture("asks.json"), task: fixture("task.json"), links: { links: [] } };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  draw(panel, cx);
  const ask = fixture("asks.json").asks.find((/** @type {any} */ item) => item.id === panel.selected);
  assert.notEqual(ask.task, null);

  panel.popover = "more";
  const tree = draw(panel, cx);
  // The menu's buttons are `menu-<index>-<label>`, and the snooze items before them vary.
  const open = [0, 1, 2, 3].map((index) => `menu-${index}-Open task`).find((id) => find(tree, id));
  assert.ok(open, "the menu offers Open task");
  press(tree, /** @type {string} */ (open));
  assert.equal(herdr.shared.get("priory.task.open").task, ask.task);
  assert.equal(herdr.shared.get("priory.attach"), undefined);
});

// Another runner's ask as `priory asks --json` lists it: `<runner>:<ask>`, its runner, last_seen and submissions.
const remote = (fields) => ({
  ...asks[0], id: "desktop:12", task: 4, runner: "desktop", last_seen: "2026-11-20T20:10:00Z", submissions: [], command: null, ...fields,
});

test("another runner's row names its runner, says when it was last seen once away, and what became of an answer sent to it", () => {
  const list = asksList([asks[0], remote({ submissions: [{ verb: "answer", text: "approved", outcome: null }] })], { view: "open", now });
  const [here, there] = list.sections[0].rows;
  assert.equal(here.runner, undefined);
  assert.equal(there.runner, "desktop");
  assert.equal(there.away, "last seen 40 min ago");
  assert.deepEqual(there.sent, { text: "Sent to desktop: approved", refused: false });
  assert.deepEqual(asksList([remote({ last_seen: "2026-11-20T20:49:00Z" })], { view: "open", now }).sections[0].rows[0].away, null);
});

test("the runner facet narrows to one runner's asks and offers each runner with its count", () => {
  const all = [{ ...asks[0], runner: "workstation" }, remote(), remote({ id: "laptop:3", runner: "laptop" })];
  const list = asksList(all, { view: "open", now, facets: { runner: "desktop" } });
  assert.deepEqual(list.sections[0].rows.map((row) => row.id), ["desktop:12"]);
  assert.deepEqual(list.facets.runner, [["workstation", 1], ["desktop", 1], ["laptop", 1]]);
});

test("another runner's command ask names its runner, says the answer goes there, runs its command there and offers no snooze", () => {
  const command = { argv: ["./deploy", "staging"], dir: "/srv/api", target: "shell", runs: ["approve"] };
  const detail = askDetail(remote({ id: "laptop:3", runner: "laptop", options: ["approve", "reject"], command, submissions: [{ verb: "answer", revision: 2, text: "reject", sent: "2026-11-20T20:30:00Z", outcome: "refused", why: "ask 3 is at revision 2" }] }), { now, armed: null });
  assert.ok(detail.context.includes("laptop, last seen 40 min ago"));
  assert.equal(detail.canSnooze, false);
  assert.equal(detail.command.note, "In /srv/api (shell). Runs on laptop when answered approve. Bound to revision 2. Runs at most once.");
  assert.deepEqual(detail.sent, { text: "laptop refused reject: ask 3 is at revision 2", refused: true });
  assert.equal(detail.elsewhere, "This ask belongs to laptop. Your answer goes there through the log, and laptop applies it when it reads it. Snooze stays on laptop.");
});

test("a closed ask of another runner still says why it refused the answer sent to it", () => {
  const refused = { verb: "answer", revision: 2, text: "approved", sent: "2026-11-20T20:46:00Z", outcome: "refused", why: "ask 12 is answered" };
  const detail = askDetail(remote({ state: "answered", answer: "rejected", answered_at: "2026-11-20T20:47:00Z", submissions: [refused] }), { now, armed: null });
  assert.deepEqual(detail.sent, { text: "desktop refused approved: ask 12 is answered", refused: true });
});

test("an ask whose answer lets an irreversible step run shows the server's History line, and others none", () => {
  const line = "History: 4 of 12 tasks through this step were judged wrong within 30 days, 1 after the merge. Too few to estimate.";
  const gated = { ...asks[0], risk_estimate: { step: "open-pr", trials: 12, line } };
  assert.equal(askDetail(gated, { now, armed: null }).riskLine, line);
  assert.equal(askDetail(asks[0], { now, armed: null }).riskLine, null);
});

test("another runner's ask lists each answer sent to it in its history, newest first, with its runner, outcome and time", () => {
  const submissions = [
    { submission: "s1", verb: "answer", revision: 2, text: "reject", sent: "2026-11-20T20:30:00Z", outcome: "refused", why: "ask 12 is at revision 3", applied: "2026-11-20T20:31:00Z" },
    { submission: "s2", verb: "answer", revision: 2, text: "approve", sent: "2026-11-20T20:40:00Z", outcome: null, why: null, applied: null },
  ];
  assert.deepEqual(askDetail(remote({ submissions }), { now, armed: null }).history, [
    { tile: "queued", text: "Sent approve to desktop for revision 2. Not applied yet", time: "20:40" },
    { tile: "refused", text: "Sent reject to desktop for revision 2. desktop refused it: ask 12 is at revision 3", time: "20:30" },
    { tile: "ask", text: "Asked revision 2", time: "10:00" },
  ]);
});

test("Copy puts an ask's command on the clipboard as one shell line, and its note is selectable", async () => {
  const { context, draw, find, herdr, press, settle } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  herdr.reset();
  herdr.replies = { asks: fixture("asks.json"), task: fixture("task.json"), links: { links: [] } };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  panel.select(1, cx);
  panel.reading = "command";
  await settle();
  const tree = draw(panel, cx);
  assert.equal(find(tree, "command-note-section")?.calls.selectable?.[0], true);
  press(tree, "copy-command-section");
  assert.equal(herdr.clipboard, "gh pr merge 212 --squash --match-head-commit a1f3c9e ''");
  assert.equal(find(draw(panel, cx), "copy-command-section")?.kids.join(""), "Copied");
});

// An approval step's ask as Max's desk showed it on 2026-11-23: no title, and a 12-line question.
const LONG = [
  "approve-implementation: Plan: /home/max/factory/plans/FND-2434-plan.md. Only apps/admin-api/src/mcp/routes.ts changes.",
  "The route table gains one entry for the export tool, behind the existing admin guard.",
  "No migration: the export reads the tables the list tools already read.",
  "Tests: apps/admin-api/test/mcp/routes.test.ts adds a case for the guard and one for the export.",
  "The plan leaves the client untouched; the next step regenerates its types.",
  "Risk: the guard is shared with the list tools, so a mistake there would expose them too.",
  "I checked https://github.com/acme/app/pull/812, which added the guard, for its tests.",
  "They cover an admin and a viewer; the export adds a third case for a revoked token.",
  "The change is about 40 lines with tests.",
  "Approve to start the implementation run, or revise to send the plan back.",
  "Reject drops the task.",
  "Nothing else waits on this answer.",
].join("\n");

test("an untitled ask with a 12-line question heads its row and detail with a short title, and Why keeps the whole question", () => {
  assert.equal(LONG.split("\n").length, 12);
  const long = { ...asks[1], key: "linear:FND-2434", step: "approve-implementation", title: null, question: LONG, text: null };
  const links = [{ pattern: "\\bpull/(\\d+)\\b", url: "https://github.com/acme/app/pull/$1" }];
  const detail = askDetail(long, { now, armed: null, links });
  assert.equal(detail.question, "FND-2434 asks at approve-implementation");
  assert.deepEqual(detail.questionSpans, [{ text: "FND-2434 asks at approve-implementation" }]);
  assert.equal(detail.why.question, LONG);
  assert.equal(detail.why.text, null);
  const row = asksList([long], { view: "all", now }).sections.flatMap((section) => section.rows)[0];
  assert.equal(row.text, "FND-2434 asks at approve-implementation");
});

test("askHeading, askBody and cutAt are the web view's, word for word", () => {
  const helpers = (/** @type {string} */ source) => source.slice(source.indexOf("/** The longest heading of an untitled ask"), source.indexOf("\n", source.indexOf("export const askBody")));
  const web = readFileSync(new URL("../../../crates/priory-core/web/asks.js", import.meta.url), "utf8");
  const herdr = readFileSync(new URL("../lib/priory.js", import.meta.url), "utf8");
  assert.ok(helpers(herdr).includes("export function askHeading"));
  assert.equal(helpers(herdr), helpers(web), "copy askHeading, askBody and cutAt from adapters/herdr/lib/priory.js to crates/priory-core/web/asks.js");
});

test("an approval ask whose task cannot be read says why above its options and on its bar", async () => {
  const { context, draw, herdr, settle, text } = await import("./kit.js");
  const { default: Asks } = await import("../mods/priory-asks/main.js");
  const { palette } = await import("../mods/priory-asks/tokens.js");
  const { step, task } = approval();
  herdr.reset();
  herdr.replies = { asks: { revision: 1, bands: [], asks: [step] }, task, links: { links: [] } };
  const panel = new Asks();
  const cx = context();
  panel.init({}, cx);
  await settle();
  assert.equal(panel.taskError, null);
  const client = /** @type {any} */ (panel.priory).client;
  client.task = async () => { throw new Error("timed out"); };
  panel.select(step.id, cx);
  await settle();
  const why = "Could not read the plan: timed out. It must load before you answer.";
  const detail = askDetail(step, { now, armed: null, task: null, taskError: panel.taskError });
  assert.equal(detail.wait, why);
  assert.ok(text(draw(panel, cx)).includes(why), "above the options");
  assert.ok(text(panel.bar(step, detail, palette({}), now)).includes(why), "on the bar");
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { all, context, draw, find, herdr, press, settle, text } from "./kit.js";
import { taskView } from "../mods/priory-task/view.js";

process.env.TZ = "UTC";

const { default: TaskPanel } = await import("../mods/priory-task/main.js");
const { default: Tabs } = await import("../mods/priory-tabs/main.js");

// Captured from `priory task 1 --json`, as the Asks panel's tests use it.
const task = JSON.parse(readFileSync(new URL("./fixtures/task.json", import.meta.url), "utf8"));
const now = Date.parse("2026-11-20T12:00:00Z");

test("a task shows its state, step, asks, pull request and its runs newest first", () => {
  const view = taskView(task, { now });
  assert.deepEqual([view.key, view.state, view.context, view.title], ["cli:PRI-61", "asleep", ["implement", "merge", "none/misc"], "Export the board as CSV."]);
  assert.deepEqual(view.asks, [{ id: 1, tile: "answered", text: "Merge #212 into main? Checks are green.", note: "Answered Merge", age: "1 h", confidence: null, bars: 0, rationale: null, auto: false }]);
  assert.deepEqual(view.pr, {
    title: "#212 Export the board as CSV",
    url: "https://github.com/acme/board/pull/212",
    line: "pri-61 into main, open",
    checks: "1 of 2 checks failing",
  });
  assert.deepEqual(view.history.map((run) => [run.id, run.tile, run.text, run.when]), [
    [3, "asleep", "implement, run 3", "3 h"],
    [2, "moved", "plan, run 2, planned", "3 h"],
    [1, "moved", "groom, run 1, yellow", "4 h"],
  ]);
});

test("a check with no conclusion yet counts as running, not failing, as the web's prStack does", () => {
  const checks = (/** @type {any[]} */ list) => taskView({ ...task, pull_request: { ...task.pull_request, checks: list } }, { now }).pr?.checks;
  assert.equal(checks([{ name: "test", conclusion: "success" }, { name: "e2e", conclusion: null }]), "1 of 2 checks running");
  assert.equal(checks([{ name: "lint", conclusion: "failure" }, { name: "e2e", conclusion: null }]), "1 of 2 checks failing");
});

test("each run shows its latest score by each scorer, as the web's task page does", () => {
  // As `priory task ID --json` lists them: the latest per run and scorer, the run as text.
  const scores = [
    { run: "2", step: "plan", scorer: "gate", kind: "outcome", verdict: "pass", reason: "Plan names the files.", time: "2026-11-20T08:30:00Z" },
    { run: "3", step: "implement", scorer: "gate", kind: "outcome", verdict: "error", reason: "Scorer timed out.", time: "2026-11-20T09:00:00Z" },
    { run: "3", step: "implement", scorer: "tests", kind: "outcome", verdict: "not applicable", reason: null, time: "2026-11-20T09:00:00Z" },
  ];
  const view = taskView({ ...task, scores }, { now });
  assert.deepEqual(view.history.map((run) => [run.id, run.scores]), [
    [3, [
      { scorer: "gate", verdict: "error", reason: "Scorer timed out." },
      { scorer: "tests", verdict: "not applicable", reason: null },
    ]],
    [2, [{ scorer: "gate", verdict: "pass", reason: "Plan names the files." }]],
    [1, []],
  ]);
});

test("the Inbox lists the events routed to the task, newest first, with what each did", async () => {
  const inbox = [
    { seq: 9, type: "github.pull_request.comment", subject: "acme/board#212", time: "2026-11-20T11:50:00Z", task: 1, outcome: "kept", prompt: "queued", wait: null },
    { seq: 7, type: "github.check_run.completed", subject: "acme/board#212", time: "2026-11-20T11:00:00Z", task: 1, outcome: "woke", prompt: "confirmed", wait: 2 },
    { seq: 5, type: "github.pull_request.comment", subject: "acme/board#212", time: "2026-11-19T10:00:00Z", task: 3, outcome: "kept", prompt: null, wait: 4 },
  ];
  const view = taskView({ ...task, id: 1, inbox }, { now });
  assert.deepEqual(view.inbox.map((event) => [event.id, event.tile, event.text, event.note]), [
    [9, "queued", "GitHub pull request comment acme/board#212", "kept, still queued · 10 min"],
    [7, "done", "GitHub check run completed acme/board#212", "woke, delivered as prompt, fired wait 2 · 1 h"],
    [5, "ready", "GitHub pull request comment acme/board#212", "kept, fired wait 4 (routed to task 3) · Nov 19, 10:00"],
  ]);
  const { handled: web } = await import("../../../crates/priory-core/web/inbox.js");
  for (const event of inbox) assert.equal(view.inbox.find((row) => row.id === event.seq).note.split(" · ")[0], web(event, 1));
});

test("open asks come first, and an open one says how long ago it was asked", () => {
  const asks = [
    { id: 1, state: "answered", question: "Merge?", answer: "Merge" },
    { id: 2, state: "open", question: "Ship?", title: "Ship it", since: "2026-11-20T11:30:00Z" },
    { id: 3, state: "withdrawn", question: "Wait?" },
  ];
  assert.deepEqual(taskView({ ...task, asks }, { now }).asks.map((ask) => [ask.id, ask.tile, ask.text, ask.note]), [
    [2, "ask", "Ship it", "Asked 30 min ago"],
    [1, "answered", "Merge?", "Answered Merge"],
    [3, "withdrawn", "Wait?", "Withdrawn"],
  ]);
});

test("the Task panel shows the task another panel opened, and Attach asks the Priory bar to attach", async () => {
  herdr.reset();
  herdr.replies = { task, links: { links: [] } };
  const panel = new TaskPanel();
  const cx = context();
  panel.init({}, cx);
  assert.match(text(draw(panel, cx)), /Open a task from the Desk/);

  herdr.shared.set("priory.task.open", { task: 1, at: Date.now() });
  draw(panel, cx);
  await settle();
  const tree = draw(panel, cx);
  assert.deepEqual(herdr.argv.filter((args) => args[0] === "task"), [["task", "1", "--json"]]);
  assert.match(text(find(tree, "task-scroll")), /Export the board as CSV\..*Merge #212 into main\?.*#212 Export the board as CSV.*groom, run 1, yellow/s);

  press(tree, "attach");
  assert.deepEqual([herdr.shared.get("priory.attach").task, herdr.shared.get("priory.attach").readOnly], [1, false]);
  press(tree, "watch");
  assert.deepEqual([herdr.shared.get("priory.attach").task, herdr.shared.get("priory.attach").readOnly], [1, true]);
});

test("the Task panel draws each score under its run, with its verdict tile and reason", async () => {
  herdr.reset();
  const scores = [
    { run: "3", step: "implement", scorer: "gate", kind: "outcome", verdict: "error", reason: "Scorer timed out.", time: "2026-11-20T09:00:00Z" },
    { run: "3", step: "implement", scorer: "tests", kind: "outcome", verdict: "fail", reason: "2 tests fail.", time: "2026-11-20T09:00:00Z" },
  ];
  herdr.replies = { task: { ...task, scores }, links: { links: [] } };
  const panel = new TaskPanel();
  const cx = context();
  panel.init({}, cx);
  herdr.shared.set("priory.task.open", { task: 1, at: Date.now() });
  draw(panel, cx);
  await settle();
  const tree = draw(panel, cx);
  assert.match(text(find(tree, "run-3")), /implement, run 3.*gate.*error.*Scorer timed out\..*tests.*fail.*2 tests fail\./s);
  assert.doesNotMatch(text(find(tree, "run-2")), /gate/);
  const icons = all(find(tree, "run-3"), "svg").map((icon) => icon.path);
  assert.ok(icons.includes("mods/priory-task/icons/circle-alert.svg"), icons.join());
  for (const icon of icons) assert.ok(existsSync(new URL(`../${icon}`, import.meta.url)), `${icon} is missing`);
});

test("an open ask's Answer opens it in the Asks aside, as the web task page links to it", async () => {
  const { layout } = await import("../lib/layout.js");
  herdr.reset();
  const asks = [{ id: 2, state: "open", question: "Ship?", title: "Ship it", since: "2026-11-20T11:30:00Z" }, ...task.asks];
  herdr.replies = { task: { ...task, asks }, links: { links: [] } };
  const frame = JSON.parse(readFileSync(new URL("../layouts/frame.json", import.meta.url), "utf8"));
  herdr.layout = layout("default", { layout: { tabs: ["desk", "asks", "tasks"].map((panel) => ({ panel })) } }, frame);
  const panel = new TaskPanel();
  const cx = context();
  panel.init({}, cx);
  herdr.shared.set("priory.task.open", { task: 1, at: Date.now() });
  draw(panel, cx);
  await settle();
  const tree = draw(panel, cx);
  assert.equal(find(tree, "answer-ask:1"), null, "a closed ask has no Answer");

  press(tree, "answer-ask:2");

  assert.equal(herdr.shared.get("priory.asks.open").ask, 2);
  assert.deepEqual(herdr.runs, [["layout.show", { id: "aside-asks" }]]);
});

test("the Priory bar opens a read-only terminal for Watch and a typing one for Attach", async () => {
  herdr.reset();
  herdr.replies = { attach: "tmux attach" };
  const bar = new Tabs();
  const cx = context();
  bar.init({}, cx);
  for (const readOnly of [true, false]) {
    herdr.shared.set("priory.attach", { task: 2, at: Date.now() + (readOnly ? 1 : 2), readOnly });
    draw(bar, cx);
    await settle();
  }
  assert.deepEqual(herdr.argv.filter((args) => args[0] === "attach"), [["attach", "2", "--read-only", "--print"], ["attach", "2", "--print"]]);
  assert.deepEqual(herdr.runs.map(([command, args]) => [command, args.name]), [["tab.new", "Watch task 2"], ["tab.new", "Task 2"]]);
});

test("the Priory bar opens the Task tab for a new request, and not for one left from before it started", async () => {
  herdr.reset();
  herdr.shared.set("priory.task.open", { task: 1, at: Date.now() - 60_000 });
  const bar = new Tabs();
  const cx = context();
  bar.init({}, cx);
  draw(bar, cx);
  await settle();
  assert.deepEqual(herdr.runs, []);

  herdr.shared.set("priory.task.open", { task: 2, at: Date.now() });
  draw(bar, cx);
  draw(bar, cx);
  await settle();
  assert.deepEqual(herdr.runs, [["tab.new", { space: "space-1", name: "Task", mod: "priory-task" }]]);
});

// Three answered asks as `priory task 4 --json` lists them, oldest first, and an open one.
test("an untitled ask with a 12-line question lists under its task with a short title", () => {
  const question = Array.from({ length: 12 }, (_, line) => `Line ${line + 1} of the plan, which changes apps/admin-api/src/mcp/routes.ts.`).join("\n");
  const asks = [{ id: 4, state: "open", key: "linear:FND-2434", step: "approve-implementation", title: null, question, since: "2026-11-20T11:30:00Z" }];
  assert.equal(taskView({ ...task, asks }, { now }).asks[0].text, "FND-2434 asks at approve-implementation");
});

const answered = [
  { id: 41, state: "answered", title: "Keep the full response", question: "Every order or the first page?", answer: "every order, as today", recommendation: "every order, as today",
    answered_by: "auto", answered_at: "2026-11-20T11:22:00Z", confidence: "high", rationale: "The brief asks for it in so many words.", since: "2026-11-20T11:20:00Z" },
  { id: 42, state: "answered", title: "Name the cursor parameter", answer: "after", answered_by: "session:7f3c2a", answered_at: "2026-11-20T11:00:00Z",
    confidence: "medium", rationale: "The invoices endpoint already uses after.", since: "2026-11-20T10:50:00Z" },
  { id: 43, state: "answered", title: "Pick the default page size", answer: "40", answered_by: "serve", answered_at: "2026-11-20T10:00:00Z", confidence: "medium", since: "2026-11-20T09:50:00Z" },
  { id: 44, state: "open", title: "Choose the pagination cursor", since: "2026-11-20T11:20:00Z" },
];

test("each answered ask says who answered, newest answer first, with its confidence, and an auto answer its rationale", async () => {
  const view = taskView({ ...task, asks: answered }, { now });
  assert.deepEqual(view.asks.map((ask) => [ask.id, ask.tile, ask.note]), [
    [44, "ask", "Asked 40 min ago"],
    [41, "auto", "Answered every order, as today by auto"],
    [42, "session", "Answered after by session 7f3c2a, medium"],
    [43, "person", "Answered 40 by a person, medium"],
  ]);
  const auto = view.asks[1];
  assert.deepEqual([auto.confidence, auto.bars, auto.rationale, auto.age], ["high", 3, "The brief asks for it in so many words.", "38 min"]);
  assert.equal(view.asks[2].rationale, null, "only an auto answer opens its rationale");
  const { answeredBy } = await import("../../../crates/priory-core/web/asks.js");
  for (const ask of answered.slice(0, 3)) assert.ok(view.asks.find((row) => row.id === ask.id).note.includes(`by ${answeredBy(ask.answered_by)}`));
});

test("the Task panel draws an auto answer with its answerer's icon, confidence, rationale and how to change course", async () => {
  herdr.reset();
  herdr.replies = { task: { ...task, asks: answered }, links: { links: [] } };
  const panel = new TaskPanel();
  const cx = context();
  panel.init({}, cx);
  herdr.shared.set("priory.task.open", { task: 1, at: Date.now() });
  draw(panel, cx);
  await settle();
  const tree = draw(panel, cx);
  assert.match(text(find(tree, "ask-41")), /Keep the full response.*Answered every order, as today by auto.*high confidence.*The brief asks for it in so many words\..*The first answer stands\. Say to the agent to change course\./s);
  assert.doesNotMatch(text(find(tree, "ask-42")), /first answer stands/);
  const icons = ["ask-41", "ask-42", "ask-43"].flatMap((id) => all(find(tree, id), "svg").map((icon) => icon.path));
  assert.deepEqual(icons, ["mods/priory-task/icons/cog.svg", "mods/priory-task/icons/terminal.svg", "mods/priory-task/icons/user.svg"]);
  for (const icon of icons) assert.ok(existsSync(new URL(`../${icon}`, import.meta.url)), `${icon} is missing`);
  assert.equal(find(tree, "answer-ask:41"), null, "an answered ask has no Answer");
});

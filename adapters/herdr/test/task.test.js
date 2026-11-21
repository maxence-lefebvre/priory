import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, find, herdr, press, settle, text } from "./kit.js";
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
  assert.deepEqual(view.asks, [{ id: 1, tile: "answered", text: "Merge #212 into main? Checks are green.", note: "Answered Merge" }]);
  assert.deepEqual(view.pr, {
    title: "#212 Export the board as CSV",
    url: "https://github.com/acme/board/pull/212",
    line: "pri-61 into main, open",
    checks: "1 of 2 checks failing",
  });
  assert.deepEqual(view.history.map((run) => [run.id, run.tile, run.text, run.when]), [
    [3, "asleep", "implement, run 3", "08:40"],
    [2, "moved", "plan, run 2, planned", "08:20"],
    [1, "moved", "groom, run 1, yellow", "08:00"],
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
    [9, "queued", "GitHub pull request comment acme/board#212", "kept, still queued · 11:50"],
    [7, "done", "GitHub check run completed acme/board#212", "woke, delivered as prompt, fired wait 2 · 11:00"],
    [5, "ready", "GitHub pull request comment acme/board#212", "kept, fired wait 4 (routed to task 3) · Nov 19, 10:00"],
  ]);
  const { handled: web } = await import("../../../crates/priory-core/web/inbox.js");
  for (const event of inbox) assert.equal(view.inbox.find((row) => row.id === event.seq).note.split(" · ")[0], web(event, 1));
});

test("open asks come first, and an open one says since when", () => {
  const asks = [
    { id: 1, state: "answered", question: "Merge?", answer: "Merge" },
    { id: 2, state: "open", question: "Ship?", title: "Ship it", since: "2026-11-20T11:30:00Z" },
    { id: 3, state: "withdrawn", question: "Wait?" },
  ];
  assert.deepEqual(taskView({ ...task, asks }, { now }).asks.map((ask) => [ask.id, ask.tile, ask.text, ask.note]), [
    [2, "ask", "Ship it", "Open since 11:30"],
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

test("the Priory bar opens a read-only terminal for Watch and a typing one for Attach", async () => {
  herdr.reset();
  herdr.replies = { attach: "tmux attach" };
  const bar = new Tabs();
  bar.init();
  const cx = context();
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
  bar.init();
  const cx = context();
  draw(bar, cx);
  await settle();
  assert.deepEqual(herdr.runs, []);

  herdr.shared.set("priory.task.open", { task: 2, at: Date.now() });
  draw(bar, cx);
  draw(bar, cx);
  await settle();
  assert.deepEqual(herdr.runs, [["tab.new", { space: "space-1", name: "Task", mod: "priory-task" }]]);
});

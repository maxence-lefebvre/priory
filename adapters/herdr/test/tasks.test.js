import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
mock.timers.enable({ apis: ["Date"], now: Date.parse("2026-11-17T12:00:00Z") });
const { default: Tasks } = await import("../mods/priory-tasks/main.js");

// `priory tasks --json` with title and batch, as #224 adds them: one task in each kind of state.
const fixture = JSON.parse(readFileSync(new URL("./fixtures/tasks-list.json", import.meta.url), "utf8"));

/** The Tasks panel after its first read. */
async function tasks() {
  herdr.reset();
  herdr.replies.tasks = fixture;
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();
  return { view, cx, drawn: () => draw(view, cx) };
}

test("Active lists the tasks not ended, each with its key, title, step and lane, or its reason", async () => {
  const { drawn } = await tasks();

  assert.deepEqual(herdr.argv[0], ["tasks", "--json"]);
  const shown = text(drawn());
  assert.match(shown, /Active 3 Blocked 1 Ended 2/);
  assert.match(shown, /ENG-244 implement review running 18 min Rate-limit the checkout webhook payments · now/);
  assert.match(shown, /ENG-239 implement implement held 4 h Export invoices as CSV waits for ENG-201 to merge · batch 1/);
  assert.doesNotMatch(shown, /Add invoice numbers|ENG-199/);
});

test("Ended lists the ended tasks newest first, with a failed task's reason", async () => {
  const { drawn } = await tasks();

  press(drawn(), "view-ended");

  const shown = text(drawn());
  assert.match(shown, /ENG-199 implement implement failed 5 h Retry invoice emails tests failed twice ENG-201 implement merge done Nov 16, 09:00 Add invoice numbers payments · now/);
  assert.doesNotMatch(shown, /Rate-limit/);
});

test("a facet value keeps the tasks that have it, and pressing the chip again clears it", async () => {
  const { drawn } = await tasks();

  assert.match(text(drawn()), /Lane none 1 payments 4 /, "values A to Z, counted over every task");

  press(drawn(), "pick-lane-none");
  assert.match(text(drawn()), /Active 1 Blocked 1 Ended 0/, "the views count the filtered tasks");
  assert.match(text(drawn()), /ENG-250/);
  assert.doesNotMatch(text(drawn()), /ENG-244/);

  press(drawn(), "facet-lane");
  assert.match(text(drawn()), /ENG-244/);
});

test("pressing a row opens its task in the Task panel", async () => {
  const { drawn } = await tasks();

  press(drawn(), "task-2");

  assert.equal(/** @type {any} */ (herdr.shared.get("priory.task.open")).task, 2);
});

test("the view model is the web view's tasks.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  assert.equal(source("../mods/priory-tasks/view.js"), source("../../../crates/priory-core/web/tasks.js"),
    "cp crates/priory-core/web/tasks.js adapters/herdr/mods/priory-tasks/view.js");
});

test("another runner's task, which has no page here, stays out of the panel", async () => {
  herdr.reset();
  herdr.replies.tasks = { ...fixture, tasks: [...fixture.tasks, { id: null, key: "ENG-300", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-17T11:00:00Z", runner: "a", hold: "kept", last_seen: "2026-11-17T11:50:00Z", title: "Move the queue" }] };
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();

  const shown = text(draw(view, cx));
  assert.match(shown, /Active 3 Blocked 1 Ended 2/);
  assert.doesNotMatch(shown, /ENG-300|elsewhere/);
});

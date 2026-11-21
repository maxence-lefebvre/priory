import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, find, herdr, press, settle, text } from "./kit.js";

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

test("another runner's task shows in Active with its hold and runner, and opens nothing", async () => {
  herdr.reset();
  herdr.replies.tasks = { ...fixture, tasks: [...fixture.tasks, { id: null, key: "ENG-300", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-17T11:00:00Z", runner: "a", hold: "kept", last_seen: "2026-11-17T11:48:00Z", title: "Move the queue" }] };
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();

  const drawn = draw(view, cx);
  const shown = text(drawn);
  assert.match(shown, /Active 4 Blocked 1 Ended 2/);
  assert.match(shown, /ENG-300 implement kept 1 h Move the queue On runner a, last seen 12 min ago$/);
  assert.equal(find(drawn, "task-null"), null, "its page stays on its runner");
  assert.match(text(drawn), /State blocked 1 done 1 elsewhere 1/);
  assert.match(text(drawn), /Runner a 1 none 5 /, "this runner's tasks name no runner in this fixture");
});

test("in a factory of several runners a Runner facet marks this runner (here)", async () => {
  herdr.reset();
  const here = fixture.tasks.map((/** @type {any} */ task) => ({ ...task, runner: "b", hold: "kept" }));
  herdr.replies.tasks = { ...fixture, tasks: [...here, { id: null, key: "ENG-300", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-17T11:00:00Z", runner: "a", hold: "kept", last_seen: null }] };
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();

  assert.match(text(draw(view, cx)), /Runner a 1 b \(here\) 5 /);
  press(draw(view, cx), "pick-runner-a");
  assert.match(text(draw(view, cx)), /Active 1 Blocked 0 Ended 0/);
});

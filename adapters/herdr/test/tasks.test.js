import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
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
  assert.match(shown, /ENG-244 implement review running .* Rate-limit the checkout webhook payments · now/);
  assert.match(shown, /ENG-239 .* Export invoices as CSV waits for ENG-201 to merge/);
  assert.doesNotMatch(shown, /Add invoice numbers|ENG-199/);
});

test("Ended lists the ended tasks newest first, with a failed task's reason", async () => {
  const { drawn } = await tasks();

  press(drawn(), "view-ended");

  const shown = text(drawn());
  assert.match(shown, /ENG-199 .* Retry invoice emails tests failed twice ENG-201 .* Add invoice numbers payments · now/);
  assert.doesNotMatch(shown, /Rate-limit/);
});

test("a facet value keeps the tasks that have it, and pressing the chip again clears it", async () => {
  const { drawn } = await tasks();

  press(drawn(), "pick-lane-none");
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

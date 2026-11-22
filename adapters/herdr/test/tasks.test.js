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

test("another runner's task shows in Active with its hold and runner, and opens no Task panel", async () => {
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

test("another runner's task with no workflow, last seen days ago, still draws, under the workflow none", async () => {
  herdr.reset();
  herdr.replies.tasks = { ...fixture, tasks: [...fixture.tasks, { id: null, key: "ENG-301", workflow: null, step: null, state: null, lane: null, band: null, since: "2026-11-15T12:00:00Z", runner: "a", hold: "waiting", last_seen: "2026-11-15T12:00:00Z", title: null }] };
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();

  const shown = text(draw(view, cx));
  assert.match(shown, /Workflow groom 1 implement 4 none 1 /);
  assert.match(shown, /ENG-301 waiting Nov 15, 12:00 ENG-301 On runner a, last seen on Nov 15, 12:00$/);
});

/** The Tasks panel with one more task, which runner `a` holds. @param {object} remote */
async function withRemote(remote) {
  herdr.reset();
  herdr.replies.tasks = { ...fixture, tasks: [...fixture.tasks, { id: null, key: "ENG-300", workflow: "implement", step: null, state: null, lane: null, band: "now", since: "2026-11-17T11:00:00Z", runner: "a", hold: "kept", last_seen: "2026-11-17T11:58:00Z", title: "Move the queue", generation: 2, submissions: [], ...remote }] };
  const view = new Tasks();
  const cx = context();
  view.init({}, cx);
  await settle();
  return () => draw(view, cx);
}

test("pressing another runner's kept task opens its actions: Resume sends at once, Cancel on a second press", async () => {
  const drawn = await withRemote({});
  herdr.replies.resume = "sent";
  herdr.replies.cancel = "sent";

  assert.equal(find(drawn(), "remote-a:ENG-300-cancel"), null, "closed until pressed");
  press(drawn(), "remote-a:ENG-300");
  assert.match(text(drawn()), /Resume Cancel Release This task runs on a\. An action goes there through the log, and a applies it when it reads it\./);

  press(drawn(), "remote-a:ENG-300-resume");
  await settle();
  assert.deepEqual(herdr.argv.at(-1), ["resume", "--generation", "2", "a:ENG-300"]);

  press(drawn(), "remote-a:ENG-300-cancel");
  assert.notEqual(herdr.argv.at(-1)[0], "cancel", "the first press only arms");
  assert.match(text(drawn()), /Press again to cancel/);
  press(drawn(), "remote-a:ENG-300-cancel");
  await settle();
  assert.deepEqual(herdr.argv.at(-1), ["cancel", "--generation", "2", "a:ENG-300"]);
});

test("a waiting task offers only Cancel, and a send that fails says why under the actions", async () => {
  const drawn = await withRemote({ hold: "waiting" });

  press(drawn(), "remote-a:ENG-300");
  assert.equal(find(drawn(), "remote-a:ENG-300-resume"), null);
  press(drawn(), "remote-a:ENG-300-cancel");
  press(drawn(), "remote-a:ENG-300-cancel");
  await settle();

  assert.match(text(drawn()), /Could not cancel: priory cancel: no such thing/);
});

test("another runner's task shows the newest action sent to it on its last line", async () => {
  const sent = { submission: "s1", act: "resume", generation: 2, text: null, step: null, sent: "2026-11-17T11:59:00Z", outcome: null, why: null, applied: null };

  assert.match(text((await withRemote({ submissions: [sent] }))()), /Move the queue Sent to a: resume On runner a, last seen 2 min ago/);
  assert.match(text((await withRemote({ submissions: [{ ...sent, outcome: "refused", why: "task 2 is running, not blocked" }] }))()), /Move the queue a refused resume: task 2 is running, not blocked On runner a/);
});

test("an armed Cancel lapses when the task moves to another generation, so the next press only arms again", async () => {
  const drawn = await withRemote({});
  press(drawn(), "remote-a:ENG-300");
  press(drawn(), "remote-a:ENG-300-cancel");

  const moved = /** @type {any} */ (herdr.replies.tasks);
  herdr.replies.tasks = { ...moved, tasks: moved.tasks.map((/** @type {any} */ task) => (task.key === "ENG-300" ? { ...task, generation: 3 } : task)) };
  herdr.wake();
  await settle();
  press(drawn(), "remote-a:ENG-300-cancel");
  await settle();

  assert.equal(herdr.argv.some((args) => args[0] === "cancel"), false, "nothing sent for the generation the person did not see");
  assert.match(text(drawn()), /Press again to cancel/);
});

test("a send that fails shows under its own task, not under the one opened meanwhile", async () => {
  const drawn = await withRemote({});
  const other = /** @type {any} */ (herdr.replies.tasks);
  herdr.replies.tasks = { ...other, tasks: [...other.tasks, { ...other.tasks.at(-1), key: "ENG-301", title: "Split the queue" }] };
  herdr.wake();
  await settle();

  press(drawn(), "remote-a:ENG-300");
  press(drawn(), "remote-a:ENG-300-resume");
  press(drawn(), "remote-a:ENG-301");
  await settle();

  assert.doesNotMatch(text(drawn()), /Could not resume/);
});

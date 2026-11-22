import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
mock.timers.enable({ apis: ["Date"], now: Date.parse("2026-11-17T12:00:00Z") });
const { default: Runs } = await import("../mods/priory-runs/main.js");

// `priory runs --json`: a live run, a moved one with model and tokens, a failed one with no agent,
// and one from over a week ago.
const fixture = JSON.parse(readFileSync(new URL("./fixtures/runs.json", import.meta.url), "utf8"));

/** The Runs panel after its first read. */
async function runs() {
  herdr.reset();
  herdr.replies.runs = fixture;
  const view = new Runs();
  const cx = context();
  view.init({}, cx);
  await settle();
  return { view, cx, drawn: () => draw(view, cx) };
}

test("Recent lists the last week's runs newest first, each with its task, agent, model, outcome, duration and tokens", async () => {
  const { drawn } = await runs();

  assert.deepEqual(herdr.argv[0], ["runs", "--json"]);
  const shown = text(drawn());
  assert.match(shown, /Recent 4 Live 1/);
  assert.match(shown, /ENG-244 review running .* reviewer run 93 · 10 min/);
  assert.match(shown, /ENG-212 implement moved .* coder, claude-sonnet-5-5 run 92 · passed · 42 min · 312k tokens/);
  assert.match(shown, /ENG-219 implement failed Nov 16, 08:00 no agent run 91 · 1 h 5 min/);
  assert.doesNotMatch(shown, /ENG-150/);
  assert.match(shown, /Show 1 older run/);
});

test("Live lists only the runs still going", async () => {
  const { drawn } = await runs();

  press(drawn(), "view-live");

  const shown = text(drawn());
  assert.match(shown, /ENG-244/);
  assert.doesNotMatch(shown, /ENG-212|ENG-219/);
});

test("the Outcome facet finds a run by how it ended or by its result's outcome", async () => {
  const { drawn } = await runs();

  assert.match(text(drawn()), /Outcome failed 1 moved 2 passed 2 running 1 /, "values A to Z, counted over every run");

  press(drawn(), "pick-outcome-passed");

  const shown = text(drawn());
  assert.match(shown, /Recent 2 Live 0/, "the views count the filtered runs");
  assert.match(shown, /ENG-212/);
  assert.doesNotMatch(shown, /ENG-244|ENG-219/);
});

test("pressing a run opens its task in the Task panel", async () => {
  const { drawn } = await runs();

  press(drawn(), "run-91");

  assert.equal(/** @type {any} */ (herdr.shared.get("priory.task.open")).task, 3);
});

test("the view models are the web view's runs.js, tasks.js and costs.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  for (const [copy, web] of [["view.js", "runs.js"], ["tasks.js", "tasks.js"], ["costs.js", "costs.js"]]) {
    assert.equal(source(`../mods/priory-runs/${copy}`), source(`../../../crates/priory-core/web/${web}`),
      `cp crates/priory-core/web/${web} adapters/herdr/mods/priory-runs/${copy}`);
  }
});

test("Recent counts the older runs and shows them on request, after a facet too", async () => {
  const { drawn } = await runs();
  assert.match(text(drawn()), /Recent 4 Live 1/);

  press(drawn(), "pick-agent-coder");
  press(drawn(), "runs-older");

  const shown = text(drawn());
  assert.match(shown, /Recent 2 Live 0/);
  assert.match(shown, /ENG-212 .* ENG-150/);
});

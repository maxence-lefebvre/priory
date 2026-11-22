import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { all, context, draw, find, herdr, settle, text } from "./kit.js";
import { attentionView, costTiles, dayBars, splitTable } from "../mods/priory-costs/view.js";

process.env.TZ = "UTC";

// Captured from `priory costs FACTORY --json`, `--by model` and `priory report attention --json` on
// a scratch factory, its records and asks stored by SQL as the daemon would, on a clock fixed at
// Tuesday 2026-11-17 12:00 in Paris: three priced records last week, then one priced record, one
// unpriced model and one record without tokens this week, one merge, two tasks and three asks.
const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));

const { default: CostsPanel } = await import("../mods/priory-costs/main.js");

test("the view model is the web view's costs.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  assert.equal(source("../mods/priory-costs/view.js"), source("../../../crates/priory-core/web/costs.js"),
    "cp crates/priory-core/web/costs.js adapters/herdr/mods/priory-costs/view.js");
});

test("this week's tiles compare with last week's, and a missing week shows no change", () => {
  assert.deepEqual(costTiles(fixture("costs")).map(({ label, value, delta }) => [label, value, delta]), [
    ["Cost this week", "$1.60", { text: "-60% vs last week", direction: "down" }],
    ["Merges", "1", null],
    ["Cost per merge", "$1.60", null],
  ]);
});

test("each day of both weeks gets a bar against the busiest day, this week's marked", () => {
  assert.deepEqual(dayBars(fixture("costs")).map(({ weekday, date, fraction, current, text }) => [`${weekday} ${date}`, Math.round(fraction * 100), current, text]), [
    ["Tue 10", 100, false, "Tue, Nov 10: $3.00, no merge"],
    ["Thu 12", 33, false, "Thu, Nov 12: $1.00, no merge"],
    ["Mon 16", 53, true, "Mon, Nov 16: $1.60, 1 merge"],
    ["Tue 17", 0, true, "Tue, Nov 17: no priced tokens, no merge"],
  ]);
});

test("the split gives each value its cost and tokens, and says what is not in the cost", () => {
  const table = splitTable(fixture("costs"));
  assert.equal(table.dimension, "Workflow");
  assert.deepEqual(table.rows, [
    { name: "implement", cost: "$1.60", tokens: "2.3M", note: "tokens unknown for 1 record" },
    { name: "review", cost: "-", tokens: "1.5k", note: "price missing" },
  ]);
  assert.deepEqual(table.unpriced, [{ model: "m2", tokens: "1.5k" }]);
  assert.equal(table.unknown, "1 record has no tokens");
  assert.deepEqual(splitTable(fixture("costs-model")).rows.map((row) => [row.name, row.cost]), [["m1", "$1.60"], ["m2", "-"]]);
});

test("attention gives minutes per merge, the ask waits and where each task's time went", () => {
  const view = attentionView(fixture("attention"));
  assert.deepEqual(view.headline, { label: "minutes of your attention per merge", value: "10", note: "10 min over 1 merge, 2 acts" });
  assert.deepEqual(view.waits, [
    { label: "Ask wait, median", value: "31 min", note: "31 min while you were present" },
    { label: "Ask wait, p90", value: "31 min", note: "31 min while you were present" },
    { label: "Asks acted on", value: "2", note: "1 still open" },
  ]);
  assert.deepEqual(view.tasks.map(({ key, workflow, wall, parts }) => [key, workflow, wall, parts.map((part) => part.value)]), [
    ["PRI-41", "implement", "4 h", ["1 h 50 min", "30 min", "1 h 30 min", "10 min"]],
    ["PRI-42", "review", "1 d 3 h", ["0 min", "1 h 1 min", "0 min", "1 d 1 h"]],
  ]);
});

test("the Costs panel shows each scorer's pass rate under the split, stacked when narrow, and with no cost", async () => {
  const scores = [
    { workflow: "implement", scorer: "gate", pass: 11, fail: 3, error: 1, not_applicable: 0 },
    { workflow: "plan-batch", scorer: "gate", pass: 0, fail: 0, error: 2, not_applicable: 0 },
  ];
  /** @param {any} costs */
  const drawn = async (costs, width) => {
    herdr.reset();
    herdr.replies = { costs, report: fixture("attention") };
    const panel = new CostsPanel();
    const cx = context();
    panel.init({}, cx);
    await settle();
    panel.width = width;
    return find(draw(panel, cx), "costs-scores");
  };

  const wide = text(await drawn({ ...fixture("costs"), scores }, 700));
  assert.match(wide, /^Scores by workflow The pass rate leaves out errors and not applicable\. Workflow Scorer Pass rate pass fail error not applicable /);
  assert.match(wide, /implement gate 79% 11 3 1 0 plan-batch gate - 0 0 2 0$/);
  const narrow = text(await drawn({ ...fixture("costs"), scores }, 420));
  assert.match(narrow, /implement gate pass rate 79%, 11 pass, 3 fail, 1 error plan-batch gate no pass rate, 2 error$/);
  const empty = text(await drawn({ ...fixture("costs"), days: [], split: [], scores }, 420));
  assert.match(empty, /implement gate pass rate 79%/);
  assert.equal(await drawn(fixture("costs"), 420), null);
  // Scorers that share a long prefix stay told apart: their names wrap, never cut.
  const long = [{ workflow: "implement", scorer: "integration-tests-linux", pass: 1, fail: 0, error: 0, not_applicable: 0 }];
  const names = all(await drawn({ ...fixture("costs"), scores: long }, 700), "div").filter((cell) => text(cell) === "integration-tests-linux");
  assert.equal(names.length, 1);
  assert.ok(names[0].calls.whitespace_normal && !names[0].calls.truncate);
  // Each selectable cell has its own id, even when two rows' names join to the same words.
  const alike = [{ workflow: "a-b", scorer: "c", pass: 1, fail: 0, error: 0, not_applicable: 0 }, { workflow: "a", scorer: "b-c", pass: 1, fail: 0, error: 0, not_applicable: 0 }];
  for (const width of [420, 700]) {
    const ids = all(await drawn({ ...fixture("costs"), scores: alike }, width), "text").map((cell) => cell.id);
    assert.equal(new Set(ids).size, ids.length, `ids at ${width} px: ${ids}`);
  }
});

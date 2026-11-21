import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, find, herdr, press, text } from "./kit.js";
import { layout } from "../lib/layout.js";

const { default: Tabs } = await import("../mods/priory-tabs/main.js");
const FRAME = JSON.parse(readFileSync(new URL("../layouts/frame.json", import.meta.url), "utf8"));

/** The Priory bar over the layout generated from `view`, and the ids and labels of its buttons. @param {any} tree */
function bar(tree) {
  herdr.reset();
  herdr.layout = layout("default", { layout: tree }, FRAME);
  const panel = new Tabs();
  const cx = context();
  panel.init();
  const drawn = () => draw(panel, cx);
  /** @param {string[]} ids @returns {any[]} each button's id, label and whether it is selected, after the bar's whole text */
  const labels = (ids) => [text(drawn()), ...ids.map((id) => [id, text(find(drawn(), id)), find(drawn(), id).calls.selected[0]])];
  return { drawn, labels };
}

const DEFAULT = { tabs: ["desk", "asks", "tasks", "runs", "inbox", "reports", "workflow", "costs"].map((panel) => ({ panel })) };

test("the bar shows a button for each aside, then each tab in the center, and selects what is shown", () => {
  const { labels } = bar(DEFAULT);

  assert.deepEqual(labels(["priory-aside-desk", "priory-aside-asks", "priory-tasks", "priory-costs"]), [
    "Desk Asks Tasks Runs Inbox Bonjour Flows Costs",
    ["priory-aside-desk", "Desk", true],
    ["priory-aside-asks", "Asks", false],
    ["priory-tasks", "Tasks", false],
    ["priory-costs", "Costs", false],
  ]);
});

test("an aside's button toggles that aside, whatever else is open", () => {
  const { drawn } = bar(DEFAULT);

  press(drawn(), "priory-aside-asks");
  press(drawn(), "priory-aside-desk");
  assert.deepEqual(herdr.runs, [["layout.toggle", { id: "aside-asks" }], ["layout.toggle", { id: "aside-desk" }]]);
});

test("a tab's button selects it in the center, and the shown one goes back to main", () => {
  const { drawn, labels } = bar(DEFAULT);

  press(drawn(), "priory-costs");
  assert.deepEqual(herdr.runs, [["layout.select", { stack: "center", child: "costs" }]]);
  herdr.runs = [];
  herdr.layout.children[1].children[1].children[0].active = "costs";
  assert.equal(labels(["priory-costs"])[1][2], true);
  press(drawn(), "priory-costs");
  assert.deepEqual(herdr.runs, [["layout.select", { stack: "center", child: "main" }]]);
});

test("opening a task shows main first when a stack hides it", async () => {
  const { settle } = await import("./kit.js");
  const { drawn } = bar({ tabs: [{ panel: "desk" }, { panel: "costs" }] });
  herdr.layout.children[1].children[1].children[0].active = "costs";

  herdr.shared.set("priory.task.open", { task: 7, at: Date.now() + 1 });
  drawn();
  await settle();

  assert.deepEqual(herdr.runs, [
    ["layout.select", { stack: "center", child: "main" }],
    ["tab.new", { space: "space-1", name: "Task", mod: "priory-task" }],
  ]);
});

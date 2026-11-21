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

test("the bar shows one button per child of the default view's stack, and selects the shown one", () => {
  const { labels } = bar(DEFAULT);

  assert.deepEqual(labels([0, 1, 2, 3, 4, 5, 6, 7].map((at) => `priory-view-${at}`)), [
    "Desk Asks Tasks Runs Inbox Bonjour Flows Costs",
    ["priory-view-0", "Desk", true],
    ["priory-view-1", "Asks", false],
    ["priory-view-2", "Tasks", false],
    ["priory-view-3", "Runs", false],
    ["priory-view-4", "Inbox", false],
    ["priory-view-5", "Bonjour", false],
    ["priory-view-6", "Flows", false],
    ["priory-view-7", "Costs", false],
  ]);
});

test("a button selects its block in the stack, and the shown one hides the view beside main", () => {
  const { drawn } = bar(DEFAULT);

  press(drawn(), "priory-view-1");
  assert.deepEqual(herdr.runs, [["layout.select", { stack: "view", child: "view-1" }]]);
  herdr.runs = [];
  press(drawn(), "priory-view-0");
  assert.deepEqual(herdr.runs, [["layout.hide", { id: "view" }]]);
  herdr.runs = [];
  herdr.layout.children[1].children[1].children[1].hidden = true;
  press(drawn(), "priory-view-0");
  assert.deepEqual(herdr.runs, [["layout.show", { id: "view" }]]);
});

test("nested stacks each get their buttons, and a button selects its branch in every stack above it", () => {
  const { drawn, labels } = bar({ tabs: [{ row: [{ panel: "desk" }, { tabs: [{ panel: "queue" }, { panel: "runs" }] }] }, { panel: "costs" }] });
  herdr.layout.children[1].children[1].children[1].active = "view-1";

  assert.deepEqual(labels(["priory-view-0", "priory-view-1", "priory-view-0-1-0", "priory-view-0-1-1"]), [
    "Desk · Queue · Runs Costs Queue Runs",
    ["priory-view-0", "Desk · Queue · Runs", false],
    ["priory-view-1", "Costs", true],
    ["priory-view-0-1-0", "Queue", false],
    ["priory-view-0-1-1", "Runs", false],
  ]);
  press(drawn(), "priory-view-0-1-1");
  assert.deepEqual(herdr.runs, [
    ["layout.select", { stack: "view", child: "view-0" }],
    ["layout.select", { stack: "view-0-1", child: "view-0-1-1" }],
  ]);
});

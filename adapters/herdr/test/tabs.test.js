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
  panel.init({}, cx);
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

test("a layout from before the asides gives a bar that says to run install again", () => {
  herdr.reset();
  herdr.layout = JSON.parse(JSON.stringify(FRAME));
  herdr.layout.children[1].children[1].children.push({ id: "view", type: "stack", active: "view-0", children: [{ id: "view-0", type: "block", content: { mod: "priory-desk", props: { panel: "desk" } } }] });
  const panel = new Tabs();
  const cx = context();
  panel.init({}, cx);

  assert.match(text(draw(panel, cx)), /older than this bar.*run install again/);
});

const { createHash } = await import("node:crypto");
const { layouts } = await import("../lib/layout.js");
const { settle } = await import("./kit.js");
const hash = (/** @type {string} */ text) => createHash("sha256").update(text.trim()).digest("hex").slice(0, 12);
const OLD = { factory: "demo", views: { default: { layout: { tabs: [{ panel: "desk" }] } }, gone: { layout: { tabs: [{ panel: "queue" }] } } } };
const NEW = { factory: "demo", views: { default: { layout: { tabs: [{ panel: "desk" }, { panel: "costs" }] } }, wide: { layout: { row: [{ panel: "terminal" }, { panel: "costs" }] } } } };

/** The bar started in the default layout installed from `installed`, with `priory views --json` printing `current`. @param {any} installed @param {any} current */
async function started(installed, current) {
  herdr.reset();
  const files = layouts(installed.views, FRAME, hash(JSON.stringify(installed)));
  for (const [file, built] of Object.entries(files)) herdr.files[`/herdr/layouts/${file}`] = JSON.stringify(built, null, 2);
  herdr.files["/herdr/layouts/main.json"] = JSON.stringify({ id: "main" });
  herdr.layout = files["priory.json"];
  herdr.replies.views = current;
  const panel = new Tabs();
  const cx = context();
  panel.init({}, cx);
  draw(panel, cx);
  await settle();
  return panel;
}

test("on start, the bar offers to regenerate the layouts when the view files changed, and only then", async () => {
  await started(OLD, OLD);
  assert.deepEqual(herdr.notices, []);

  await started(OLD, NEW);
  assert.deepEqual(herdr.notices, [{
    title: "The view files changed",
    body: "Click to regenerate the Priory layouts from them.",
    level: "info",
    action: { name: "priory-tabs.regenerate" },
  }]);
});

test("regenerating writes every view's layout and removes its own layout of a view that is gone", async () => {
  await started(OLD, NEW);
  herdr.notices = [];

  herdr.command("regenerate");
  await settle();

  assert.deepEqual(Object.keys(herdr.files).sort(), ["/herdr/layouts/main.json", "/herdr/layouts/priory-wide.json", "/herdr/layouts/priory.json"]);
  const written = JSON.parse(herdr.files["/herdr/layouts/priory.json"]);
  assert.equal(written.id, `priory-generated-${hash(JSON.stringify(NEW))}`);
  assert.deepEqual(written, layouts(NEW.views, FRAME, hash(JSON.stringify(NEW)))["priory.json"]);
  assert.deepEqual(herdr.notices, [{ title: "Regenerated the Priory layouts", level: "success" }]);
});

test("regenerating refuses a layout file it did not write, and writes nothing", async () => {
  await started(OLD, NEW);
  herdr.notices = [];
  herdr.files["/herdr/layouts/priory-wide.json"] = JSON.stringify({ id: "mine" });
  const before = { ...herdr.files };

  herdr.command("regenerate");
  await settle();

  assert.deepEqual(herdr.files, before);
  assert.deepEqual(herdr.notices, [{
    title: "Can't regenerate the Priory layouts",
    body: "layouts/priory-wide.json is not a layout this adapter wrote. Move it away, then regenerate again.",
    level: "error",
  }]);
});

test("regenerating owns only a string root id, so it keeps a layout whose id only looks like its own", async () => {
  await started(OLD, NEW);
  herdr.notices = [];
  herdr.files["/herdr/layouts/priory-gone.json"] = JSON.stringify({ id: ["priory-generated"] });

  herdr.command("regenerate");
  await settle();

  assert.equal(herdr.files["/herdr/layouts/priory-gone.json"], JSON.stringify({ id: ["priory-generated"] }));
});

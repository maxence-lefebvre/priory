import { test } from "node:test";
import assert from "node:assert/strict";
import { START, entryOf, needs, shellOf, step, where } from "./frame.js";

const tabs = (...panels) => ({ tabs: panels.map((panel) => ({ panel })) });
const layout = tabs("desk", "asks", "tasks", "runs", "inbox");
const shell = (href, size = "wide", list = "desk", node = layout) => shellOf(where(href), { size, list, layout: node });
const read = (href, size = "wide", list = "desk", node = layout) => needs(where(href), shell(href, size, list, node), href.split("/")[2]);

test("an address leads to its page, and to the ask, task or batch it opens", () => {
  assert.deepEqual(where("/"), { here: "desk", ask: null, task: null, batch: null });
  assert.deepEqual(where("/index.html?x=1").here, "desk");
  assert.equal(where("/asks/desk-1%3A42").ask, "desk-1:42");
  assert.equal(where("/tasks/7#terminal").task, 7);
  assert.equal(where("/tasks/abc").task, null);
  assert.equal(where("/batches/2").batch, "2");
  assert.equal(where("/batches/x").batch, null);
});

test("each page reads the listings it shows and no other", () => {
  assert.deepEqual(read("/"), ["asks", "queue", "tasks"]);
  assert.deepEqual(read("/asks", "wide", "asks"), ["asks"]);
  assert.deepEqual(read("/tasks"), ["tasks"]);
  assert.deepEqual(read("/runs"), ["runs"]);
  assert.deepEqual(read("/queue"), ["queue", "tasks"]);
  assert.deepEqual(read("/inbox"), ["tasks"]);
  assert.deepEqual(read("/terminal"), ["tasks"]);
  assert.deepEqual(read("/reports"), ["report", "reports", "tasks"]);
  assert.deepEqual(read("/tasks/7"), ["asks", "tasks"]);
  assert.deepEqual(read("/batches/2"), ["tasks"]);
  for (const href of ["/costs", "/workflow", "/more", "/memories", "/settings/notifications"]) assert.deepEqual(read(href), [], href);
  assert.deepEqual(read("/settings/appearance"), ["asks", "queue", "tasks"], "the theme previews draw the Desk");
});

test("an ask reads the list drawn beside it, and only on a screen that draws one", () => {
  assert.deepEqual(read("/asks/4", "wide", "desk"), ["asks", "queue", "tasks"]);
  assert.deepEqual(read("/asks/4", "wide", "asks"), ["asks", "tasks"]);
  assert.deepEqual(read("/asks/4", "compact", "desk"), ["asks", "tasks"]);
});

test("a view of several panels reads each of them on a wide screen, and one below it", () => {
  const node = { columns: [{ panel: "desk" }, { tabs: [{ panel: "runs" }, { panel: "inbox" }] }] };
  assert.deepEqual(read("/", "wide", "desk", node), ["asks", "queue", "runs", "tasks"]);
  assert.deepEqual(read("/", "medium", "desk", node), ["asks", "queue", "tasks"]);
  assert.deepEqual(read("/", "compact", "desk", node), ["asks", "queue", "tasks"]);
});

test("the shell fits the page and the screen", () => {
  assert.deepEqual(shell("/"), { grid: "wide", nav: "desk", panels: ["desk"] });
  assert.deepEqual(shell("/", "medium"), { grid: "medium", nav: "desk", panels: ["desk"] });
  assert.deepEqual(shell("/tasks"), { grid: "wide no-drawer", nav: "tasks", panels: ["tasks"] });
  assert.deepEqual(shell("/inbox"), { grid: "wide no-drawer", nav: "inbox", panels: ["inbox"] });
  assert.equal(shell("/inbox", "wide", "desk", { columns: [{ panel: "desk" }, { panel: "inbox" }] }).grid, "wide");
  assert.deepEqual(shell("/tasks/7"), { grid: "wide no-drawer", nav: "tasks", panels: [] });
  assert.deepEqual(shell("/asks/4", "wide", "asks"), { grid: "wide", nav: "asks", panels: ["asks"] });
  assert.equal(shell("/nothing", "medium").grid, "medium no-drawer");
});

test("the navigation marks the page's entry", () => {
  const nav = (href, size) => shell(href, size).nav;
  assert.deepEqual(["wide", "medium", "compact"].map((size) => nav("/settings/appearance", size)), ["settings", "more", "more"]);
  assert.equal(nav("/batches/2"), "tasks");
  assert.equal(nav("/memories", "compact"), "desk");
  assert.equal(nav("/asks/4", "compact"), null, "an open ask takes the phone's whole screen");
});

test("a list becomes the one a detail goes back to, and a detail leaves it", () => {
  const list = step(START, "/asks?view=all", {});
  assert.deepEqual(list, { list: "asks", listUrl: "/asks?view=all", from: "/asks?view=all" });
  assert.equal(step(list, "/tasks/3", {}), list);
  assert.deepEqual(step(list, "/queue", {}), { ...list, from: "/queue" });
  assert.deepEqual(entryOf(list, "/asks/4"), { list: "asks", listUrl: "/asks?view=all" });
  assert.deepEqual(entryOf(list, "/tasks/3"), { from: "/asks?view=all" });
  assert.deepEqual(entryOf(list, "/runs"), {});
});

test("an address that does not decode leads to its page as it is written", () => {
  assert.equal(where("/asks/%E0%A4%A").ask, "%E0%A4%A");
  assert.equal(where("/asks/100%").ask, "100%");
});

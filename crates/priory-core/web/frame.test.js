import { test } from "node:test";
import assert from "node:assert/strict";
import { START, entryOf, partsIn, partsOf, shellOf, step, where } from "./frame.js";

const tabs = (...panels) => ({ tabs: panels.map((panel) => ({ panel })) });
const layout = tabs("desk", "asks", "tasks", "runs", "inbox");
const shell = (href, size = "wide", list = "desk", node = layout) => shellOf(where(href), { size, list, layout: node });
const parts = (href, size = "wide", list = "desk", node = layout) => partsOf(where(href), shell(href, size, list, node), { size, layout: node });

test("an address leads to its page, and to the ask, task or batch it opens", () => {
  assert.deepEqual(where("/"), { here: "desk", ask: null, task: null, batch: null });
  assert.deepEqual(where("/index.html?x=1").here, "desk");
  assert.equal(where("/asks/desk-1%3A42").ask, "desk-1:42");
  assert.equal(where("/tasks/7#terminal").task, 7);
  assert.equal(where("/tasks/abc").task, null);
  assert.equal(where("/batches/2").batch, "2");
  assert.equal(where("/batches/x").batch, null);
});

test("each page fills the shell with its own part, beside a drawer where the screen has one", () => {
  const list = (name) => ({ kind: "list", name });
  assert.deepEqual(parts("/"), { main: list("desk"), drawer: true });
  assert.deepEqual(parts("/", "medium"), { main: list("desk"), drawer: true });
  assert.deepEqual(parts("/", "compact"), { main: { kind: "page", name: "desk" }, drawer: false });
  assert.deepEqual(parts("/tasks"), { main: list("tasks"), drawer: false });
  assert.deepEqual(parts("/inbox"), { main: list("inbox"), drawer: false });
  assert.deepEqual(parts("/costs"), { main: { kind: "page", name: "costs" }, drawer: false });
  assert.deepEqual(parts("/tasks/7"), { main: { kind: "detail", name: "tasks", id: 7 }, drawer: false });
  assert.deepEqual(parts("/batches/2", "compact"), { main: { kind: "detail", name: "batches", id: "2" }, drawer: false });
  assert.deepEqual(parts("/asks/4", "wide", "asks"), { main: list("asks"), drawer: true });
  assert.deepEqual(parts("/asks/4", "compact"), { main: { kind: "detail", name: "asks", id: "4" }, drawer: false });
});

test("a view of several panels fills a wide screen, and one of them a smaller one", () => {
  const node = { columns: [{ panel: "desk" }, { tabs: [{ panel: "runs" }, { panel: "inbox" }] }] };
  assert.deepEqual(parts("/", "wide", "desk", node), { main: node, drawer: true });
  assert.deepEqual(partsIn(node, "inbox").map((part) => part.name), ["desk", "inbox"]);
  assert.deepEqual(parts("/", "medium", "desk", node).main, { kind: "list", name: "desk" });
  assert.deepEqual(parts("/tasks", "wide", "desk", node).main, { kind: "list", name: "tasks" }, "a list whose rows open a page takes the whole width");
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

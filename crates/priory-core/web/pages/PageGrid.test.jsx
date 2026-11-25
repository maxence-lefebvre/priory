import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";
import { where, shellOf } from "../frame.js";

// The legacy files touch the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  document: { getElementById: () => sheet, addEventListener() {}, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} }, hidden: false },
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { PageGrid, needsOf } = await import("./PageGrid.jsx");
const { cache } = await import("../data.js");
const { PANELS } = await import("../pages.js");
const { entries, entryOf, register } = await import("../registry.js");
const { Place } = await import("./place.js");
const { partsIn, partsOf } = await import("../frame.js");
const { landing } = await import("../legacy/data.js");
await import("../entries/index.js");
const all = [...entries];
landing.set(false);

const tabs = (...panels) => ({ tabs: panels.map((panel) => ({ panel })) });
const TABS = tabs("desk", "asks", "tasks", "runs", "inbox");
const trail = (list) => ({ list, listUrl: "/", from: null });

function place(href, { size = "wide", list = "desk", layout = TABS } = {}) {
  const at = where(href);
  return { href, entry: {}, trail: trail(list), at, size, section: href.split("/")[2], layout, shell: shellOf(at, { size, list, layout }) };
}
const draw = (href, options) => renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={place(href, options)}><PageGrid /></Place.Provider></QueryClientProvider>);
function reads(href, options) {
  const { at, size, section, layout, shell, trail: { list } } = place(href, options);
  const { main, drawer } = partsOf(at, shell, { size, layout });
  return needsOf([...partsIn(main, at.ask ? list : at.here), ...(drawer ? [{ kind: "drawer", name: "asks", id: at.ask }] : [])], section);
}

test("every panel of a view has an entry with a list and a page", () => {
  for (const panel of Object.keys(PANELS)) {
    const entry = entryOf(panel);
    assert.ok(entry?.list ?? entry?.List, panel);
    assert.ok(entry?.page ?? entry?.Page, panel);
  }
  assert.equal(new Set(all.map((entry) => entry.name)).size, all.length);
});

test("a countdown is a memory's, else a task's, else the ask's", () => {
  const order = all.filter((entry) => entry.drain).map((entry) => entry.name);
  assert.deepEqual(order, ["memories", "taskacts", "asks"]);
});

test("each page's legacy parts read the listings they show and no other", () => {
  assert.deepEqual(reads("/"), ["asks", "queue", "tasks"]);
  assert.deepEqual(reads("/asks"), ["asks"]);
  assert.deepEqual(reads("/tasks"), ["tasks"]);
  assert.deepEqual(reads("/runs"), ["runs"]);
  assert.deepEqual(reads("/queue"), ["queue", "tasks"]);
  assert.deepEqual(reads("/inbox"), ["tasks"]);
  assert.deepEqual(reads("/terminal", { size: "compact" }), ["tasks"]);
  assert.deepEqual(reads("/reports"), ["report", "reports", "tasks"]);
  assert.deepEqual(reads("/batches/2"), ["tasks"]);
  for (const href of ["/costs", "/workflow", "/more", "/memories", "/settings/notifications"]) assert.deepEqual(reads(href), [], href);
  assert.deepEqual(reads("/settings/appearance"), ["asks", "queue", "tasks"], "the theme previews draw the Desk");
  assert.deepEqual(reads("/tasks/7"), [], "a React part reads its own");
  assert.deepEqual(reads("/asks/4"), ["asks", "queue", "tasks"]);
  assert.deepEqual(reads("/asks/4", { list: "asks" }), ["asks", "tasks"]);
  assert.deepEqual(reads("/asks/4", { size: "compact" }), ["asks", "tasks"]);
});

const TREE = { row: [{ panel: "desk" }, { column: [{ panel: "queue" }, { tabs: [{ panel: "runs" }, { panel: "inbox" }] }] }] };
const SLOT = '<div class="legacy"></div>';

test("a list and its drawer each draw in their own element", () => {
  assert.equal(draw("/"), SLOT + SLOT);
  assert.equal(draw("/tasks"), SLOT);
  assert.equal(draw("/costs"), SLOT);
  assert.equal(draw("/asks/4", { size: "compact" }), SLOT);
});

test("a tree draws its wrappers, its tab strip and one element per panel", () => {
  const html = draw("/", { layout: TREE }).replace(/<svg.*?<\/svg>/g, "");
  assert.equal(html, `<div class="tree tree-row">${SLOT}<div class="tree tree-column">${SLOT}<div class="tree-tabs"><nav class="tabbar" aria-label="Tabs"><a href="/runs" aria-current="page">Runs</a><a href="/inbox" aria-current="false">Inbox</a></nav>${SLOT}</div></div></div>${SLOT}`);
});

test("a React list sits beside legacy panels, and the legacy ones read only their own", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "queue" ? { name: "queue", List: () => <p>Queue in React</p>, Page: () => <p>Queue page</p> } : entry)));
  assert.match(draw("/", { layout: TREE }), /tree-column"><p>Queue in React<\/p><div class="tree-tabs">/);
  assert.equal(draw("/queue"), "<p>Queue in React</p>", "no legacy element, so nothing lands for one");
  assert.deepEqual(reads("/queue"), []);
  assert.deepEqual(reads("/", { layout: { row: [{ panel: "queue" }, { panel: "runs" }] } }), ["runs"]);
});

test("a React drawer takes the ask's id, beside a legacy list", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "asks" ? { ...entry, Drawer: ({ id, panels }) => <aside>{`${id} ${panels}`}</aside> } : entry)));
  assert.equal(draw("/asks/4", { list: "desk" }), `${SLOT}<aside>4 desk</aside>`);
  assert.deepEqual(reads("/asks/4", { list: "asks" }), ["asks"]);
});

test("while the listings load, a tree of legacy panels draws one element, and one with a React panel keeps it", (t) => {
  t.after(() => (register(all), landing.set(false)));
  landing.set(true);
  assert.equal(draw("/", { layout: TREE }), SLOT);
  register(all.map((entry) => (entry.name === "queue" ? { name: "queue", List: () => <p>Queue in React</p> } : entry)));
  assert.match(draw("/", { layout: TREE }), /Queue in React/);
});

test("an address without a page says so", () => assert.match(draw("/batches/x"), /Page not found/));

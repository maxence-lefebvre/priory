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
const { PageGrid } = await import("./PageGrid.jsx");
const { needsOf } = await import("../registry.js");
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

test("a legacy countdown is a task's", () => {
  assert.deepEqual(all.filter((entry) => entry.drain).map((entry) => entry.name), ["taskacts"]);
});

test("each page's legacy parts read the listings they show and no other", () => {
  for (const href of ["/", "/asks", "/tasks", "/runs", "/queue", "/inbox"]) assert.deepEqual(reads(href), [], `${href}: a React part reads its own`);
  assert.deepEqual(reads("/terminal", { size: "compact" }), []);
  assert.deepEqual(reads("/batches/2"), []);
  for (const href of ["/costs", "/workflow", "/reports", "/more", "/memories", "/settings/notifications", "/settings/appearance"]) assert.deepEqual(reads(href), [], href);
  assert.deepEqual(reads("/tasks/7"), [], "a React part reads its own");
  assert.deepEqual(reads("/asks/4"), []);
  assert.deepEqual(reads("/asks/4", { list: "asks" }), []);
  assert.deepEqual(reads("/asks/4", { size: "compact" }), []);
});

const TREE = { row: [{ panel: "desk" }, { column: [{ panel: "queue" }, { tabs: [{ panel: "runs" }, { panel: "inbox" }] }] }] };
const SLOT = '<div class="legacy"></div>';
// A React part not read yet, and a drawer with no ask, which needs no read.
const LOADING = '<p class="empty" role="status">Loading…</p>';
const DRAWER = '<aside class="drawer" aria-label="Ask"><p class="empty">Select an ask to read it here.</p></aside>';
const LEGACY_TREE = { row: [{ panel: "costs" }, { panel: "reports" }] };

test("a legacy list draws in its own element, and a React one says it loads beside its drawer", () => {
  assert.equal(draw("/", { layout: LEGACY_TREE }), `<div class="tree tree-row">${SLOT}${SLOT}</div>${DRAWER}`);
  assert.equal(draw("/tasks"), LOADING, "a React list says it loads in its own place");
  assert.equal(draw("/queue"), LOADING);
  assert.equal(draw("/"), LOADING + DRAWER);
  assert.equal(draw("/asks/4", { size: "compact" }), `<div class="flat">${LOADING}</div>`);
});

test("a tree draws its wrappers, its tab strip and one element per panel", () => {
  const html = draw("/", { layout: TREE }).replace(/<svg.*?<\/svg>/g, "");
  assert.equal(html, `<div class="tree tree-row">${LOADING}<div class="tree tree-column">${LOADING}<div class="tree-tabs"><nav class="tabbar" aria-label="Tabs"><a href="/runs" aria-current="page">Runs</a><a href="/inbox" aria-current="false">Inbox</a></nav>${LOADING}</div></div></div>${DRAWER}`);
});

test("a React list sits beside legacy panels, and the legacy ones read only their own", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "queue" ? { name: "queue", List: () => <p>Queue in React</p>, Page: () => <p>Queue page</p> } : entry)));
  assert.match(draw("/", { layout: TREE }), /tree-column"><p>Queue in React<\/p><div class="tree-tabs">/);
  assert.equal(draw("/queue"), "<p>Queue in React</p>", "no legacy element, so nothing lands for one");
  assert.deepEqual(reads("/queue"), []);
  assert.deepEqual(reads("/", { layout: { row: [{ panel: "queue" }, { panel: "costs" }] } }), []);
});

test("a drawer takes the ask's id and the panels beside it", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "asks" ? { ...entry, Drawer: ({ id, panels }) => <aside>{`${id} ${panels}`}</aside> } : entry)));
  assert.equal(draw("/asks/4", { list: "desk" }), `${LOADING}<aside>4 desk</aside>`);
});

test("while the listings load, a tree of legacy panels draws one element, and one with a React panel keeps it", (t) => {
  t.after(() => (register(all), landing.set(false)));
  landing.set(true);
  assert.equal(draw("/", { layout: LEGACY_TREE }), SLOT);
  assert.match(draw("/", { layout: TREE }), /tree-row"><p class="empty" role="status">Loading…<\/p>/);
  register(all.map((entry) => (entry.name === "queue" ? { name: "queue", List: () => <p>Queue in React</p> } : entry)));
  assert.match(draw("/", { layout: { row: [{ panel: "queue" }, { panel: "costs" }] } }), /Queue in React/);
});

test("Settings draws the Desk in each theme's previews, empty until its listings are read", () => {
  const html = draw("/settings/appearance");
  assert.equal(html.match(/<div class="preview" data-theme="\w+" data-mode="(light|dark)"><div><\/div><\/div>/g).length, 10);
});

test("an entry that draws nothing holds events alone", () => {
  const drawing = ["list", "page", "detail", "drawer", "List", "Page", "Detail", "Drawer"];
  assert.deepEqual(all.filter((entry) => !drawing.some((part) => entry[part])).map((entry) => entry.name), ["add", "taskacts"]);
});

test("an address without a page says so", () => assert.match(draw("/batches/x"), /Page not found/));

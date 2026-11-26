import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";
import { where, shellOf } from "../frame.js";

// The data modules read the device's storage as they load.
Object.assign(globalThis, {
  document: { getElementById: () => null, addEventListener() {}, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} }, hidden: false },
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { PageGrid } = await import("./PageGrid.jsx");
const { cache } = await import("../data.js");
const { PANELS } = await import("../pages.js");
const { entries, entryOf, register } = await import("../registry.js");
const { Place } = await import("./place.js");
await import("../entries/index.js");
const all = [...entries];

const tabs = (...panels) => ({ tabs: panels.map((panel) => ({ panel })) });
const TABS = tabs("desk", "asks", "tasks", "runs", "inbox");
const trail = (list) => ({ list, listUrl: "/", from: null });

function place(href, { size = "wide", list = "desk", layout = TABS } = {}) {
  const at = where(href);
  return { href, entry: {}, trail: trail(list), at, size, section: href.split("/")[2], layout, shell: shellOf(at, { size, list, layout }) };
}
const draw = (href, options) => renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={place(href, options)}><PageGrid /></Place.Provider></QueryClientProvider>);

test("every panel of a view has an entry with a list and a page", () => {
  for (const panel of Object.keys(PANELS)) {
    const entry = entryOf(panel);
    assert.ok(entry?.List, panel);
    assert.ok(entry?.Page, panel);
  }
  assert.equal(new Set(all.map((entry) => entry.name)).size, all.length);
});

const TREE = { row: [{ panel: "desk" }, { column: [{ panel: "queue" }, { tabs: [{ panel: "runs" }, { panel: "inbox" }] }] }] };
// A part not read yet, and a drawer with no ask, which needs no read.
const LOADING = '<p class="empty" role="status">Loading…</p>';
const DRAWER = '<aside class="drawer" aria-label="Ask"><p class="empty">Select an ask to read it here.</p></aside>';
const later = (label) => `<div class="list" role="region" aria-label="${label}"><div class="lhead"><h1>${label}</h1></div><p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code></code>.</p></div>`;

test("a panel with only a page says it comes later, and a list says it loads beside its drawer", () => {
  assert.equal(draw("/", { layout: { row: [{ panel: "costs" }, { panel: "reports" }] } }), `<div class="tree tree-row">${later("Costs")}${later("Reports")}</div>${DRAWER}`);
  assert.equal(draw("/tasks"), LOADING, "a list says it loads in its own place");
  assert.equal(draw("/queue"), LOADING);
  assert.equal(draw("/"), LOADING + DRAWER);
  assert.equal(draw("/asks/4", { size: "compact" }), `<div class="flat">${LOADING}</div>`);
});

test("a tree draws its wrappers, its tab strip and one element per panel", () => {
  const html = draw("/", { layout: TREE }).replace(/<svg.*?<\/svg>/g, "");
  assert.equal(html, `<div class="tree tree-row">${LOADING}<div class="tree tree-column">${LOADING}<div class="tree-tabs"><nav class="tabbar" aria-label="Tabs"><a href="/runs" aria-current="page">Runs</a><a href="/inbox" aria-current="false">Inbox</a></nav>${LOADING}</div></div></div>${DRAWER}`);
});

test("a list sits in its place in the tree", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "queue" ? { name: "queue", List: () => <p>Queue list</p>, Page: () => <p>Queue page</p> } : entry)));
  assert.match(draw("/", { layout: TREE }), /tree-column"><p>Queue list<\/p><div class="tree-tabs">/);
  assert.equal(draw("/queue"), "<p>Queue list</p>");
});

test("a drawer takes the ask's id and the panels beside it", (t) => {
  t.after(() => register(all));
  register(all.map((entry) => (entry.name === "asks" ? { ...entry, Drawer: ({ id, panels }) => <aside>{`${id} ${panels}`}</aside> } : entry)));
  assert.equal(draw("/asks/4", { list: "desk" }), `${LOADING}<aside>4 desk</aside>`);
});

test("Settings draws the Desk in each theme's previews, empty until its listings are read", () => {
  const html = draw("/settings/appearance");
  assert.equal(html.match(/<div class="preview" data-theme="\w+" data-mode="(light|dark)"><div><\/div><\/div>/g).length, 10);
});

test("every entry draws a part", () => {
  assert.deepEqual(all.filter((entry) => !["List", "Page", "Detail", "Drawer"].some((part) => entry[part])).map((entry) => entry.name), []);
});

test("an address without a page says so", () => assert.match(draw("/batches/x"), /Page not found/));

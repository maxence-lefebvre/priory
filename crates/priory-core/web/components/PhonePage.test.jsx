import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";

// The legacy files touch the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  document: { getElementById: () => sheet, addEventListener() {}, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} }, hidden: false },
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
await import("../entries/index.js");
const { rowsPage } = await import("../legacy/frame.js");
const { savedViews, html } = await import("../legacy/shared.js");
const { PageTop, Panel } = await import("./PhonePage.jsx");
const { SavedViews } = await import("./SavedViews.jsx");

const flat = (text) => text.replace(/>\s+</g, "><");

test("a list's page on a phone draws what its legacy one draws", () => {
  const legacy = rowsPage("Runs & more", { views: savedViews([["recent", "Recent", 3, true], ["live", "Live", null, false]], "/runs", "&band=now"), rows: html`<p>row</p>` }).text;
  const views = [{ key: "recent", label: "Recent", count: 3, on: true }, { key: "live", label: "Live", count: null, on: false }];
  const react = renderToStaticMarkup(<><PageTop title="Runs & more" /><Panel label="Runs & more"><div className="scroll"><SavedViews views={views} base="/runs" facets="&band=now" /><p>row</p></div></Panel></>);
  assert.equal(react.replaceAll("&amp;", "&#38;"), flat(legacy).replace('href="/runs?view=recent&band', 'href="/runs?view=recent&#38;band'));
});

test("a panel keeps a class of its own", () => {
  assert.equal(renderToStaticMarkup(<Panel label="Reports" kind="report" />), '<div class="panel report" role="region" aria-label="Reports"></div>');
});

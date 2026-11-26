import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup as markup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The data modules read the device's storage and the page as they load.
const kept = new Map();
const root = { dataset: { mode: "dark", theme: "ink" } };
Object.assign(globalThis, {
  document: { getElementById: () => ({ addEventListener() {} }), addEventListener() {}, querySelector: () => null, querySelectorAll: () => [], documentElement: root, hidden: false },
  localStorage: { getItem: (key) => kept.get(key) ?? null, setItem: (key, value) => kept.set(key, value), removeItem: (key) => kept.delete(key) },
  addEventListener() {},
  location: { origin: "http://localhost" },
});
const { Appearance, KindBoxes, Settings, pick, push } = await import("./Settings.jsx");
const { PUSH_KINDS } = await import("../push.js");
const { Place } = await import("./place.js");
const { client } = await import("../queries.js");
// The theme previews read the Desk's listings.
const renderToStaticMarkup = (part) => markup(<QueryClientProvider client={client({ api: async () => ({}) })}>{part}</QueryClientProvider>);

const DEFAULTS = { ask: true, stopped: true, intake: true, uncertain: true, cap: true, landed: false, stall: false };
const checked = (html) => [...html.matchAll(/id="push-kind-(\w+)"[^>]*checked=""/g)].map(([, kind]) => kind);
const at = (section, size) => renderToStaticMarkup(<Place.Provider value={{ section, size }}><Settings /></Place.Provider>);

test("each kind the daemon has draws a named checkbox, checked as it is stored", () => {
  const html = renderToStaticMarkup(<KindBoxes kinds={DEFAULTS} busy={false} />);
  assert.equal(html.match(/type="checkbox"/g).length, PUSH_KINDS.length);
  for (const [, name] of PUSH_KINDS) assert.ok(html.includes(`<b>${name}</b>`), name);
  assert.deepEqual(checked(html), ["ask", "stopped", "intake", "uncertain", "cap"]);
  assert.ok(!html.includes("Nothing is chosen"));
  assert.ok(renderToStaticMarkup(<KindBoxes kinds={DEFAULTS} busy />).includes('aria-disabled="true"'));
});

test("no kind chosen says so, and a daemon without kinds draws nothing", () => {
  const none = Object.fromEntries(Object.keys(DEFAULTS).map((kind) => [kind, false]));
  assert.ok(renderToStaticMarkup(<KindBoxes kinds={none} busy={false} />).includes("Nothing is chosen, so this device gets no notification."));
  assert.equal(renderToStaticMarkup(<KindBoxes kinds={null} busy={false} />), "");
});

test("Appearance checks the document's mode and theme, and draws two previews of each theme", () => {
  const html = renderToStaticMarkup(<Appearance />);
  assert.deepEqual([...html.matchAll(/aria-checked="true"[^>]*>(\w+)|aria-checked="true" aria-label="(\w+)"/g)].map(([, mode, theme]) => mode ?? theme), ["Dark", "Ink"]);
  assert.equal(html.match(/class="preview" data-theme="\w+" data-mode="(light|dark)"/g).length, 10);
  assert.match(html, /<div class="previews" aria-hidden="true" inert="">/);
});

test("a pick sets the document's mode or theme and keeps both", (t) => {
  t.after(() => Object.assign(root.dataset, { mode: "dark", theme: "ink" }));
  pick("theme", "abbey");
  pick("mode", "light");
  assert.deepEqual({ ...root.dataset }, { mode: "light", theme: "abbey" });
  assert.equal(kept.get("priory.mode"), "light");
  assert.equal(kept.get("priory.theme"), "abbey");
});

test("a phone's Settings goes back to More, a wide screen's sits in a list", () => {
  assert.match(at("appearance", "compact"), /^<div class="flat"><div class="top"><a class="back" href="\/more">/);
  assert.match(at("appearance", "wide"), /^<div class="list "><div class="scroll"><div class="screen"><h1>Appearance<\/h1>/);
});

test("Notifications says why this device cannot get them, and offers the kinds once on", (t) => {
  t.after(() => push.set({ ...push.get(), status: null, kinds: null, error: null }));
  push.set({ ...push.get(), status: "insecure" });
  assert.match(at("notifications", "wide"), /<p class="note" role="status">.*<span>Notifications need HTTPS\./);
  push.set({ ...push.get(), status: "on", kinds: DEFAULTS, error: "Could not save <this>" });
  const html = at("notifications", "wide");
  assert.match(html, /<span>Could not save &lt;this&gt;<\/span>/);
  assert.match(html, /<fieldset class="push-kinds">/);
  assert.match(html, /<button type="button" class="btn secondary" aria-disabled="false">Turn off notifications<\/button>/);
});

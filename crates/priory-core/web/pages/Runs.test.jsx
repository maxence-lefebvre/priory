import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The data modules these pages import read the device's storage and the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, documentElement: { dataset: {} } },
  addEventListener() {},
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client, keyOf } = await import("../queries.js");
const { RunsList, RunsPage } = await import("./Runs.jsx");
const { Place } = await import("./place.js");

const SINCE = "2026-11-25T10:00:00Z";

function draw(t, Part, { href = "/tasks", size = "wide", data = {}, errors = {} } = {}) {
  const cache = client({ api: async () => ({}) });
  for (const [key, value] of Object.entries(data)) cache.setQueryData(JSON.parse(key), value);
  for (const [key, message] of Object.entries(errors)) cache.getQueryCache().build(cache, { queryKey: JSON.parse(key) }).setState({ status: "error", error: new Error(message) });
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ href, size }}><Part /></Place.Provider></QueryClientProvider>);
}
const K = (name, all) => JSON.stringify(keyOf(name, { all: { asks: false, tasks: Boolean(all), runs: Boolean(all) } }));

const RUNS = {
  older: 2,
  runs: [
    { run: 11, task: 1, key: "linear:ENG-1", step: "implement", agent: "claude", model: "opus", started: SINCE, tokens: { input: 12000 } },
    { run: 10, task: 2, key: "linear:ENG-2", step: "plan", started: SINCE, ended: "2026-11-25T11:05:00Z", end: "moved", outcome: "planned" },
  ],
};

test("Runs draws its table and list rows as before React", (t) => {
  const html = draw(t, RunsList, { href: "/runs", data: { [K("runs")]: RUNS } });
  assert.match(html, /<a href="\/runs\?view=recent" aria-current="true">Recent<span class="num">4<\/span><\/a><a href="\/runs\?view=live" aria-current="false">Live<span class="num">1<\/span><\/a>/);
  assert.match(html, /<table class="tbl runs"><thead><tr><th>State<\/th><th class="r">Run<\/th><th>Task<\/th><th>Step<\/th><th>Agent and model<\/th><th>Outcome<\/th><th class="r">Duration<\/th><th class="r">Tokens<\/th><th class="r">Started<\/th><\/tr><\/thead>/);
  assert.match(html, /<tr data-href="\/tasks\/2"><td><span class="sti t-[a-z]+"><svg[^>]*>.*?<\/svg>moved<\/span><\/td><td class="r">10<\/td><td class="mono"><a href="\/tasks\/2" title="task 2">ENG-2<\/a><\/td><td>plan<\/td><td> <span class="t3"><\/span><\/td><td>planned<\/td><td class="r">1 h 5 min<\/td><td class="r"><\/td><td class="r"><time/);
  assert.match(html, /<tr class="grp" data-older="runs"><td colSpan="9">Loading older runs…<\/td><\/tr><\/tbody><\/table><\/div><\/div>$/);
  const phone = draw(t, RunsPage, { href: "/runs?view=live", size: "compact", data: { [K("runs")]: RUNS } });
  assert.match(phone, /<span class="l2 r">claude, opus<\/span><span class="l3"><span>run 11<\/span><span>\d+ (min|h \d+ min)<\/span><span>12k tokens<\/span><\/span>/);
  assert.ok(!phone.includes("data-older"));
  assert.match(draw(t, RunsList, { href: "/runs?view=live", data: { [K("runs")]: { runs: [] } } }), /<p class="empty">No run is live. A run shows here while an agent works on a step.<\/p>/);
});

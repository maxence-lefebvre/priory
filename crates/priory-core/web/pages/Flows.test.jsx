import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

Object.assign(globalThis, { localStorage: { getItem: () => null, setItem() {}, removeItem() {} } });
const { client } = await import("../queries.js");
const { flows: read } = await import("../reads/flows.js");
const { Place } = await import("./place.js");
const { Flows } = await import("./Flows.jsx");

const edge = (outcome, to, by) => ({ outcome, to, by });
const ship = {
  name: "ship it",
  start: "plan",
  steps: [
    { id: "plan", decides: "model", kind: "reversible", edges: [edge("green", "merge", "model")] },
    { id: "merge", decides: "code", kind: "irreversible", edges: [edge("merged", "done", "code")] },
  ],
};
const FLOWS = {
  workflows: [ship],
  paths: [{ workflow: "ship it", path: ["plan", "merge"] }],
  irreversible: [{ workflow: "ship it", step: "merge", risk_estimate: { trials: 30, shown: true, caught: 2, judged_wrong: { k: 3, p95: 0.2 }, landed_wrong: { k: 1, p95: 0.1 } } }],
};

function draw(t, { size = "wide", flows = FLOWS, error = null } = {}) {
  const cache = client({ api: async () => ({}) });
  t.after(() => cache.clear());
  const query = cache.getQueryCache().build(cache, cache.defaultQueryOptions(read()));
  if (flows) query.setData(flows);
  if (error) query.setState({ status: "error", error });
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ size, href: "/workflow" }}><Flows /></Place.Provider></QueryClientProvider>);
}

test("each workflow draws its diagram, its history, its unattended paths and its steps as text", (t) => {
  const html = draw(t);
  assert.match(html, /^<div class="list"><div class="lhead"><h1>Flows<\/h1><\/div><div class="panel" role="region" aria-label="Flows"><div class="scroll"><section class="sect flegend" aria-labelledby="legend-title">/);
  assert.match(html, /<section class="sect flow" aria-labelledby="flow-ship_it"><h2 class="sh" id="flow-ship_it">ship it<span class="meta num">2 steps<\/span><\/h2><svg class="diagram"/);
  assert.match(html, /<g transform="translate\([\d.]+ [\d.]+\)" class="fnode k-irreversible"><title>merge: irreversible write, no approval<\/title>.*?<text x="75" y="39" text-anchor="middle" class="fl2">no approval<\/text>/);
  assert.match(html, /<svg class="ico flock" x="[\d.]+" y="[\d.]+" width="[\d.]+" height="[\d.]+" viewBox="0 0 24 24" aria-hidden="true">/);
  assert.match(html, /<path d="M0 4H48" stroke-width="[\d.]+" stroke-dasharray="[\d ]+" class="fline"><\/path>/);
  assert.match(html, /<tbody><tr><th scope="row" class="mono">merge<\/th><td class="r" data-label="Caught">2<\/td><td data-label="Judged wrong">3 of 30<span class="bound">/);
  assert.match(html, /<div class="unattended" role="note"><p class="t-danger"><svg class="ico s15"[^>]*>.*?<\/svg>Reaches an irreversible step with no person&#x27;s decision:<\/p><ul><li class="mono">plan &gt; merge<\/li><\/ul><\/div>/);
  assert.match(html, /<li><b class="mono">plan<\/b>, reversible write, decided by model\. green leads to merge \(a model decides\)\. <\/li>/);
});

test("a phone draws the page's top, and a factory with no workflow says where to add one", (t) => {
  const html = draw(t, { size: "compact", flows: { workflows: [], paths: [], irreversible: [] } });
  assert.match(html, /^<div class="flat"><div class="top"><h1>Flows<\/h1><span class="sp"><\/span><span class="conn ">live<\/span><\/div>/);
  assert.match(html, /<p class="empty">This factory has no workflows\. Add one under <code>workflows\/<\/code>\.<\/p>/);
});

test("flows not read yet say so, and a failed first read says why with a Retry", (t) => {
  assert.match(draw(t, { flows: null }), /<p class="empty" role="status">Loading the flows…<\/p><\/div>$/);
  const html = draw(t, { flows: null, error: new Error("/api/flows answered 500") });
  assert.match(html, /<p class="empty" role="alert">The flows did not load: \/api\/flows answered 500\. Run <code>priory check --explain<\/code> meanwhile\.<\/p><p class="empty"><button type="button" class="btn secondary">Retry<\/button><\/p>/);
});

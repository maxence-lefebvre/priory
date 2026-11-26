import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

Object.assign(globalThis, { localStorage: { getItem: () => null, setItem() {}, removeItem() {} } });
const { client, hold } = await import("../queries.js");
const { attention: attentionRead } = await import("../reads/attention.js");
const { costs: costsRead } = await import("../reads/costs.js");
const { Place } = await import("./place.js");
const { Costs } = await import("./Costs.jsx");

const tokens = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
const total = (cost_usd, merges = 0) => ({ cost_usd, tokens, unpriced: [], unknown: 0, merges });
const day = (date, cost_usd, merges = 0) => ({ day: date, ...total(cost_usd, merges) });
const COSTS = {
  by: "workflow",
  current: { since: "2026-11-15T23:00:00.000Z", since_day: "2026-11-16", until: "2026-11-17T11:00:00.000Z", ...total(12.5, 2), unpriced: [{ model: "local-7b", tokens: { ...tokens, input: 1200 } }] },
  previous: { since: "2026-11-08T23:00:00.000Z", since_day: "2026-11-09", until: "2026-11-15T23:00:00.000Z", ...total(10, 1) },
  days: [day("2026-11-14", 10, 1), day("2026-11-16", 12.5, 2)],
  split: [{ workflow: "implement", ...total(12.5, 2), unpriced: [{ model: "local-7b" }] }],
  scores: [],
};
const ATTENTION = {
  since: "2026-11-16T00:00:00.000Z", until: "2026-11-17T11:00:00.000Z", by: "workflow",
  acts: 4, person_minutes: 16, merges: 2, person_minutes_per_merge: 8,
  waits: { acted: 4, open: 0, median_minutes: 45.5, p90_minutes: 61, median_present_minutes: 45.5, p90_present_minutes: 61 },
  split: [],
  tasks: [{ id: 3, key: "ENG-3", workflow: "implement", lane: "web", state: "done", since: "2026-11-16T08:00:00Z", until: "2026-11-16T12:00:00Z", wall_minutes: 240, working_minutes: 110, person_wait_minutes: 30, world_wait_minutes: 90, queued_minutes: 10 }],
};

function draw(t, { size = "wide", href = "/costs", costs = COSTS, attention = ATTENTION, at = Date.now() } = {}) {
  const cache = client({ api: async () => ({}) });
  t.after(() => cache.clear());
  if (costs) hold(cache, costsRead("workflow"), costs, { updatedAt: at });
  if (attention) hold(cache, attentionRead(), attention, { updatedAt: at });
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ size, href }}><Costs /></Place.Provider></QueryClientProvider>);
}

test("a wide screen shows the spend beside the attention, with no saved views", (t) => {
  const html = draw(t);
  assert.match(html, /^<div class="list"><div class="lhead"><h1>Costs<\/h1><\/div><div class="panel" role="region" aria-label="Costs"><div class="scroll"><div class="spend"><div><section class="sect" aria-labelledby="costs-week">/);
  assert.ok(!html.includes('class="views"'));
  assert.match(html, /<figure class="days"><svg width="100%" height="152" role="img" aria-label="Cost per day, last week then this week\./);
  assert.match(html, /<h2 class="sh" id="costs-split">This week by workflow<label class="by"><span>Split by<\/span><select aria-disabled="false"><option value="workflow" selected="">workflow<\/option>/);
  assert.match(html, /<p><b>Not in the cost\.<\/b> Tokens with no price: <span class="mono">local-7b<\/span> [^<]+ tokens\. Add each model to <code>prices\.toml<\/code> to price them\. <\/p>/);
  assert.match(html, /<tbody><tr><th scope="row" class="mono">implement<\/th><td class="r">\$12\.50<\/td><td class="r">[^<]+<\/td><td class="n">price missing<\/td><\/tr><\/tbody>/);
  assert.match(html, /<\/div><div><section class="sect" aria-labelledby="attention-head">/);
  assert.match(html, /<ol class="tsplit"><li><p><a class="mono" href="\/tasks\/3" title="task 3">ENG-3<\/a><span>implement<\/span><span class="sp"><\/span><span class="num">4 h<\/span><\/p><svg class="tbar" width="100%" height="12" role="img"/);
  assert.equal(html.match(/<line class="gap"/g).length, 3);
  assert.ok(!html.includes("asof"), "a fresh read names no time");
});

test("a phone shows one of the two by its saved view", (t) => {
  const html = draw(t, { size: "compact", href: "/costs?view=attention" });
  assert.match(html, /^<div class="flat"><div class="top"><h1>Costs<\/h1><span class="sp"><\/span><span class="conn ">live<\/span><\/div><div class="panel" role="region" aria-label="Costs"><nav class="views" aria-label="Saved views"><a href="\/costs\?view=costs" aria-current="false">Costs<\/a><a href="\/costs\?view=attention" aria-current="true">Attention<\/a><\/nav><div class="scroll"><section class="sect" aria-labelledby="attention-head">/);
  assert.ok(!html.includes("costs-week"));
});

test("a read older than a minute says when it was read", (t) => {
  assert.match(draw(t, { at: Date.now() - 120000 }), /<div class="scroll"><p class="note asof"><span>Costs as read <time dateTime="[^"]+" title="[^"]+">[^<]+<\/time>\.<\/span><\/p><div class="spend">/);
});

test("a week with no cost keeps its scores and their Split by", (t) => {
  const scores = [{ name: "implement", scorer: "tests", pass: 3, fail: 1, error: 0, not_applicable: 0 }];
  const html = draw(t, { costs: { ...COSTS, days: [], split: [], scores } });
  assert.match(html, /<p class="empty">No cost in the last two weeks\./);
  assert.match(html, /<section class="sect" aria-labelledby="costs-scores"><h3 class="sh" id="costs-scores">Scores by workflow<label class="by">/);
});

test("nothing read yet draws the panel empty", (t) => {
  assert.match(draw(t, { costs: null, attention: null }), /<div class="panel" role="region" aria-label="Costs"><div class="scroll"><\/div><\/div><\/div>$/);
});

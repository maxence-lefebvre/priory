import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The legacy files touch the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} } },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client, keyOf } = await import("../queries.js");
const { DeskList, DeskPage, DeskPreview } = await import("./Desk.jsx");
const { Place } = await import("./place.js");

const LINKS = [{ pattern: String.raw`\B#(\d+)\b`, url: "https://github.com/{repo}/issues/$1" }];
const DESK = {
  revision: 4, counts: { asleep: 1, unrouted: 2 }, since_you_looked: [], last_visit: null,
  items: [
    { kind: "ask", id: 3, task: 3, key: "ENG-3", what: "Which cluster?", since: "2026-11-25T10:00:00Z" },
    { kind: "blocked", id: 7, task: 7, key: "ENG-7", step: "implement", what: "Fix #12 first", since: "2026-11-25T09:00:00Z" },
    { kind: "batch", id: 2, key: "batch:2026-11-25-1", what: "Invoice exports: 2 of 4 merged", since: "2026-11-25T08:00:00Z" },
    { kind: "todo", id: 5, what: "Call <the> bank", since: "2026-11-25T08:00:00Z" },
  ],
  next: { kind: "ask", id: 3, task: 3, key: "ENG-3", why: "oldest ask", since: "2026-11-25T10:00:00Z" },
};
const ASKS = [{ id: 3, task: 3, key: "ENG-3", step: "implement", state: "open", revision: 2, since: "2026-11-25T10:00:00Z", question: "Which cluster?", options: ["a", "b"], recommendation: "a" }];
const QUEUE = { running: [], waiting: [], asleep: [], up_next: [{ kind: "queued", task: 8, key: "ENG-8", why: "first in the queue", at: "2026-11-25T10:00:00Z" }] };
const TASKS = [{ id: 7, key: "ENG-7", state: "blocked", repo: "acme/shop" }];

function draw(t, Part, { size = "wide", search = "" } = {}) {
  const cache = client({ api: async () => ({}) });
  cache.setQueryData(keyOf("links"), LINKS);
  cache.setQueryData(keyOf("views"), { factory: "shop", views: { default: { layout: { tabs: [] } } } });
  cache.setQueryData(keyOf("desk"), DESK);
  cache.setQueryData(keyOf("asks"), { asks: ASKS, bands: [], older: { answered: 0, withdrawn: 0 } });
  cache.setQueryData(keyOf("queue"), QUEUE);
  cache.setQueryData(keyOf("tasks"), { tasks: TASKS, older: 0 });
  t.after(() => cache.clear());
  const place = { size, href: `/${search}`, at: { here: "desk", ask: null }, trail: { list: "desk", listUrl: `/${search}` }, entry: {} };
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={place}><Part /></Place.Provider></QueryClientProvider>);
}

test("a wide Desk draws a table whose keys link their task or batch, and each action again in its fold", (t) => {
  const html = draw(t, DeskList);
  assert.match(html, /^<div class="list" role="region" aria-label="Desk"><div class="lhead"><h1>Desk<\/h1><span class="cnt"><span class="n num">1<\/span><span class="w">ask needs you<span>/);
  assert.match(html, /<tr data-href="\/asks\/3" aria-current="false">/);
  assert.match(html, /<td class="mono"><a href="\/tasks\/7" title="task 7">ENG-7<\/a><\/td>/);
  assert.match(html, /<td class="mono"><a href="\/batches\/2">batch:2026-11-25-1<\/a><\/td>/);
  assert.match(html, /<span>Fix <a class="autolink" href="https:\/\/github.com\/acme\/shop\/issues\/12" rel="noopener noreferrer" target="_blank">#12<\/a> first<\/span>/, "a row's text links in its task's repository");
  assert.match(html, /<div class="fifth"><span class="task-acts"><button type="button" id="fold-act-7-resume"/);
  assert.match(html, /<td class="recw"><span class="task-acts"><button type="button" id="act-7-resume"/);
  assert.match(html, /id="fold-todo-row-5"/);
  assert.match(html, /Call &lt;the&gt; bank/, "a to-do's text stays escaped");
  assert.match(html, /<a href="\/tasks\/8" title="task 8"><span class="mono">ENG-8<\/span> first in the queue<\/a>/);
  assert.match(html, /<a class="row one calm" href="\/inbox\?view=unrouted">/);
});

test("a phone's Desk lists its rows and ends with the Answer capsule beside Add", (t) => {
  const html = draw(t, DeskPage, { size: "compact" });
  assert.match(html, /^<div class="flat"><div class="top"><span class="fac">shop<\/span><span class="conn ">live<\/span><\/div><div class="panel" role="region" aria-label="Desk"><h1 class="sr">Desk<\/h1><div class="scroll desk-end">/);
  assert.match(html, /<div class="sec">Asks<span>oldest first<\/span><\/div><a class="row " href="\/asks\/3" aria-current="false">/);
  assert.match(html, /<a class="mono" href="\/tasks\/7" title="task 7">ENG-7<\/a>/);
  assert.match(html, /<a class="capsule " href="\/asks\/3"><svg[^>]*>.*<\/svg>Answer 1 ask<\/a><button type="button" class="fab" data-action="add-open"/);
});

test("a saved view draws its own rows, and a preview has no capsule", (t) => {
  assert.match(draw(t, DeskList, { search: "?view=running" }), /<p class="empty">No run is live\. Runs show here while an agent works\.<\/p>/);
  const preview = draw(t, DeskPreview);
  assert.match(preview, /^<div><div class="top">/);
  assert.doesNotMatch(preview, /capsule/);
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The data modules read the device's storage and the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} } },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client, keyOf } = await import("../queries.js");
const { AskDrawer, AskPage, AsksList, answers } = await import("./Asks.jsx");
const { Place } = await import("./place.js");

const LINKS = [{ pattern: String.raw`\b([A-Z]{2,5}-\d+)\b`, url: "https://linear.app/acme/issue/$1" }];
const ask = (id, more = {}) => ({
  id, task: id, key: `ENG-${id}`, step: "implement", state: "open", revision: 2, since: "2026-11-25T10:00:00Z", question: `Which cluster for ENG-${id}?`,
  options: ["staging-b", "skip"], recommendation: "staging-b", dismissable: true, ...more,
});
const ASKS = [ask(3, { tradeoff: "<b>older</b> index" }), ask(5, { since: "2026-11-25T11:00:00Z" }), ask(7, { state: "answered", answer: "skip", answered_at: "2026-11-25T09:00:00Z", answered_by: "serve" })];
const TASK = { id: 3, key: "ENG-3", repo: "acme/shop", brief: { request: "Run the search tests." }, runs: [{ id: 9, step: "implement", started: "2026-11-25T09:00:00Z" }] };

function draw(t, Part, { size = "wide", ask: at = null, list = "asks", props = {} } = {}) {
  const cache = client({ api: async () => ({}) });
  cache.setQueryData(keyOf("links"), LINKS);
  cache.setQueryData(keyOf("desk"), { revision: 4, items: ASKS.filter((each) => each.state === "open").map((each) => ({ kind: "ask", id: each.id, what: each.question })), counts: {} });
  cache.setQueryData(keyOf("asks"), { asks: ASKS, bands: [], older: { answered: 0, withdrawn: 0 } });
  cache.setQueryData(keyOf("tasks"), { tasks: [], older: 0 });
  cache.setQueryData(["task", 3], TASK);
  t.after(() => cache.clear());
  const place = { size, href: at ? `/asks/${at}` : "/asks", at: { here: "asks", ask: at }, trail: { list, listUrl: "/asks" }, entry: {} };
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={place}><Part {...props} /></Place.Provider></QueryClientProvider>);
}

test("the asks list draws its views, a table of groups and the selected row", (t) => {
  const html = draw(t, AsksList, { ask: "5" });
  assert.match(html, /^<div class="list" role="region" aria-label="Asks"><div class="lhead"><h1>Asks<\/h1><\/div><nav class="views" aria-label="Saved views"><a href="\/asks\?view=open" aria-current="true">Open<span class="num">2<\/span><\/a>/);
  assert.match(html, /<tr class="grp"><td colSpan="6">Open<span>2, oldest first<\/span><\/td><\/tr><tr data-href="\/asks\/3" aria-current="false">.*<tr data-href="\/asks\/5" aria-current="true">/);
  assert.doesNotMatch(html, /\/asks\/7/, "an answered ask stays in its own view");
  assert.match(html, /<div class="keys">/);
});

test("an ask reads top to bottom: title, recommendation, options, the action row with Snooze, chips, then sections", (t) => {
  const html = draw(t, AskPage, { size: "compact", ask: "3", props: { id: "3" } });
  const at = (part) => html.indexOf(part);
  const order = ['<h1 class="question">', '<div class="rec"><span><b>Recommended: staging-b.</b>', '<div class="opts" role="group" aria-label="Options">', "Snooze 1 h", '<nav class="jump" aria-label="Sections">', '<section class="sect" id="why">', '<section class="sect" id="brief">', '<section class="sect" id="trace">', '<section class="sect" id="history">'];
  assert.ok(order.every((part) => at(part) >= 0), order.filter((part) => at(part) < 0).join(", "));
  assert.deepEqual(order.map(at), [...order.map(at)].sort((a, b) => a - b));
  assert.match(html, /^<div class="flat"><h1 class="sr">Ask #3<\/h1><div class="ahead">/);
  assert.match(html, /<a class="autolink" href="https:\/\/linear.app\/acme\/issue\/ENG-3"[^>]*>ENG-3<\/a>/, "the question links its keys");
  assert.match(html, /<b>Trade-off.<\/b> &lt;b&gt;older&lt;\/b&gt; index/, "agent text stays escaped");
  assert.match(html, /<a href="#why" aria-current="true">Why<\/a><a href="#brief" aria-current="false">Brief<\/a><a href="#trace" aria-current="false">Trace<\/a><a href="#history" aria-current="false">History<\/a>/);
  assert.match(html, /<div class="abar" hidden="">/, "the bar waits for the options to scroll away");
  assert.match(html, /<button type="button" id="bar-answer" class="btn primary " data-answer="0" aria-disabled="false"><span>staging-b<\/span><small>Recommended<\/small><\/button>/);
});

test("an armed option says what its second press does, and only for the revision it armed", (t) => {
  t.after(() => answers.set({ ...answers.get(), armed: null }));
  answers.set({ ...answers.get(), armed: { id: 3, revision: 2, text: "staging-b", at: 0 } });
  const html = draw(t, AskDrawer, { ask: "3", props: { id: "3", panels: ["asks"] } });
  assert.match(html, /<button type="button" class="opt rec-opt  armed" data-option="0" data-answer="0" aria-disabled="false"><span><small>Tap again to send<\/small>Confirm: staging-b<small>Cancels in 4 s<\/small><\/span><span class="drain"/);
  assert.match(html, /<kbd>2<\/kbd>/, "a wide drawer names each option's key");
  answers.set({ ...answers.get(), armed: { id: 3, revision: 1, text: "staging-b", at: 0 } });
  assert.doesNotMatch(draw(t, AskDrawer, { ask: "3", props: { id: "3", panels: ["asks"] } }), /Confirm:/);
});

test("a pressed option shows its answer sending on itself, keeps its label, and the others take no press", (t) => {
  t.after(() => answers.set({ ...answers.get(), pending: null }));
  answers.set({ ...answers.get(), pending: { id: 3, text: "skip" } });
  const html = draw(t, AskDrawer, { ask: "3", props: { id: "3", panels: ["asks"] } });
  assert.match(html, /<button type="button" class="opt  chosen " data-option="1" data-answer="1" aria-disabled="true" aria-busy="true"><span class="busy" aria-hidden="true"><\/span><span>skip<small>Sending…<\/small><\/span>/);
  assert.match(html, /<button type="button" class="opt rec-opt  " data-option="0" data-answer="0" aria-disabled="true">/);
  assert.doesNotMatch(html, /class="sending"/, "nothing else on the page says it is sending");
  const phone = draw(t, AskPage, { size: "compact", ask: "3", props: { id: "3" } });
  assert.doesNotMatch(phone, /id="bar-answer"/, "the bar hides when another option is sending");
  answers.set({ ...answers.get(), pending: { id: 3, text: "staging-b" } });
  const bar = draw(t, AskPage, { size: "compact", ask: "3", props: { id: "3" } });
  assert.match(bar, /id="bar-answer" class="btn primary " data-answer="0" aria-disabled="true" aria-busy="true"><span class="busy" aria-hidden="true"><\/span><span>staging-b<\/span><small>Sending…<\/small>/);
});

test("a drawer with no ask says what to select, and an unknown ask says where to look", (t) => {
  assert.equal(draw(t, AskDrawer, { props: { id: null, panels: ["asks"] } }), '<aside class="drawer" aria-label="Ask"><p class="empty">Select an ask to read it here.</p></aside>');
  assert.match(draw(t, AskDrawer, { ask: "99", props: { id: "99", panels: ["asks"] } }), /No ask 99 here\. It may belong to another factory\. Check <code>priory asks --all<\/code>\./);
});

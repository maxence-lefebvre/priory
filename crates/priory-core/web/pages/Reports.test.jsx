import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";
import fixture from "../../../../adapters/herdr/test/fixtures/report.json";

// The data modules, whose store of what to read this page shares, read these as they load.
const element = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => element, documentElement: { dataset: {} } },
  addEventListener() {},
});
const { client, keyOf } = await import("../queries.js");
const { setFailed } = await import("../session.js");
const { askFor } = await import("../data.js");
const { Place } = await import("./place.js");
const { Reports } = await import("./Reports.jsx");

const thread = { id: 5, author: "lead", created: "2026-11-20T08:00:00Z", text: "why later?", resolved: false, replies: [{ id: 6, author: "agent", created: "2026-11-20T08:10:00Z", text: "needs a <b>repro</b>" }] };
const done = { id: 7, author: "lead", created: "2026-11-20T07:00:00Z", text: "seen", resolved: true, replies: [] };
// The shared fixture's rows carry tracker-like keys; this page's tests read them as neutral ones.
const neutral = (read) => JSON.parse(JSON.stringify(read).replaceAll("FND-", "ROW-").replaceAll("fnd-", "row-"));
const withThreads = (read) => structuredClone({
  ...read,
  report: { ...read.report, sections: read.report.sections.map((section) => ({ ...section, rows: section.rows.map((row) => (row.id === "row-2413" ? { ...row, comments: [thread, done] } : row)) })) },
});
const READ = withThreads(neutral(fixture));

function draw(t, { size = "wide", read = READ, reports = [{ task: 1 }, { task: 0 }], shown = null } = {}) {
  const cache = client({ api: async () => ({}) });
  t.after(() => cache.clear());
  askFor({ reportShown: shown });
  t.after(() => askFor({ reportShown: null }));
  cache.setQueryData(keyOf("report", { all: {}, reportShown: shown }), read);
  cache.setQueryData(["reports"], reports);
  cache.setQueryData(keyOf("tasks"), { tasks: [{ id: 1, key: "cli:bonjour-1" }] });
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ size, href: "/reports" }}><Reports /></Place.Provider></QueryClientProvider>);
}

test("a report draws its bar, its head, its section chips and its rows", (t) => {
  const html = draw(t);
  assert.match(html, /^<div class="list"><div class="lhead"><h1>Reports<\/h1><\/div><div class="panel report" role="region" aria-label="Reports"><div class="rbar"><button type="button" class="iconbtn" aria-label="Older report" aria-disabled="false">/);
  assert.match(html, /<span class="rdate">Nov 20<\/span><button type="button" class="iconbtn" aria-label="Newer report" aria-disabled="true">/);
  assert.match(html, /<span class="sp"><\/span><button type="button" class="btn secondary">Next pick<\/button><button type="button" class="iconbtn" aria-label="Search this report" aria-pressed="false">/);
  assert.match(html, /<div class="scroll" data-sections=""><header class="rhead"><p class="greet">Bonjour, Max<\/p><p class="meta">Nov 20, <a class="mono" href="\/tasks\/1" title="task 1">[^<]+<\/a>\. Since /);
  assert.match(html, /<p class="prog"><progress max="4" value="2" aria-label="Picks agreed"><\/progress><span class="num">2 of 4 agreed<\/span><\/p><\/header>/);
  assert.match(html, /<nav class="jump" aria-label="Sections"><a href="#report-0" aria-current="true">Reading<span class="num">2<\/span><\/a><a href="#report-1" aria-current="false">/);
  assert.match(html, /<section class="sect" id="report-2" aria-labelledby="report-2-title"><h2 class="sh" id="report-2-title">Sentry<span class="meta num">3<\/span><\/h2><div class="rfilter"><div role="group" aria-label="Show in Sentry"><button type="button" aria-pressed="true">All<span class="num">2<\/span><\/button>/);
  assert.match(html, /<div role="group" aria-label="Recommends in Sentry"><span class="cap">Recommends<\/span><button type="button" aria-pressed="false">Take<span class="num">1<\/span><\/button><button type="button" aria-pressed="false">Later<span class="num">1<\/span><\/button><\/div><button type="button" class="btn primary" aria-disabled="false">Pick all recommended<span class="num">1<\/span><\/button>/);
  assert.match(html, /<h2 class="sh" id="report-4-title">Blocked<span class="meta num">2<\/span><\/h2><ol class="rrows">/, "a section without a pick has no chips");
  assert.match(html, /<p class="rl1"><span class="rtext">Yesterday: merged the MCP v2 plan review[^<]*&lt;b&gt;not html&lt;\/b&gt;<\/span><\/p>/, "agent text stays text");
});

test("a pick is open, agreed with a Change, or applied", (t) => {
  const html = draw(t);
  const id = (...parts) => encodeURIComponent(JSON.stringify(parts)).replace(/[%()]/g, (c) => `\\${c}`);
  assert.match(html, /<p class="note rpicked"><svg class="ico s15"[^>]*>.*?<\/svg><span><b>Picked Take\.<\/b> requested task cli:ROW-2409<\/span><\/p>/);
  assert.match(html, new RegExp(`<div class="rpick" role="group" aria-label="Pick for ROW-2413"><button type="button" id="${id("pick", "2026-11-20_sentry_row-2413", "Take")}" class="" aria-pressed="false" aria-disabled="false">Take</button>`));
  assert.match(html, /<\/div><p class="note">Recommends Later<\/p>/);
  assert.match(html, /<span><b>Picked Take\.<\/b> Waiting for the workflow to apply it\.<\/span><button type="button" id="[^"]+" class="ghost">Change<\/button><\/p>/);
});

test("a row's threads take a reply, a resolve or a reopen, and the newest report takes comments", (t) => {
  const html = draw(t);
  assert.match(html, /<div class="rthread" aria-label="Thread 5"><ol><li><p class="rwho"><b>lead<\/b> <span>[^<]+<\/span><\/p><p class="rsay">why later\?<\/p><\/li><li><p class="rwho"><b>agent<\/b> <span>[^<]+<\/span><\/p><p class="rsay">needs a &lt;b&gt;repro&lt;\/b&gt;<\/p><\/li><\/ol>/);
  assert.match(html, /<p class="racts"><button type="button" id="reply-5" class="ghost" aria-disabled="false">Reply<\/button><button type="button" id="resolve-5" class="ghost" aria-disabled="false">Resolve<\/button><\/p><\/div>/);
  assert.match(html, /<p class="note rresolved"><svg[^>]*>.*?<\/svg><span>Resolved: seen<\/span><button type="button" id="reopen-7" class="ghost" aria-disabled="false">Reopen<\/button><\/p>/);
  assert.equal(html.match(/>Comment<\/button>/g).length, 2, "each row with its own id");
  assert.ok(!draw(t, { shown: 0 }).includes(">Comment</button>"), "an older report takes replies only");
});

test("offline, nothing can be picked, changed or sent", (t) => {
  t.after(() => setFailed(null));
  setFailed("the connection dropped");
  const html = draw(t);
  assert.match(html, /<p class="note"><svg[^>]*>.*?<\/svg><span>Reconnect to pick\.<\/span><\/p>/);
  assert.ok(!html.includes(">Change</button>"));
  assert.ok(!html.includes('aria-disabled="false">Reply'));
});

test("a phone draws the page's top, and no report says how one is written", (t) => {
  assert.match(draw(t, { size: "compact" }), /^<div class="flat"><div class="top"><h1>Reports<\/h1><span class="sp"><\/span><span class="conn ">live<\/span><\/div><div class="panel report"/);
  assert.match(draw(t, { read: null }), /<div class="panel" role="region" aria-label="Reports"><p class="empty">No report yet\. A workflow writes one with the <code>report<\/code> tool or <code>priory report put<\/code>\.<\/p><\/div><\/div>$/);
});

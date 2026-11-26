import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The legacy files touch the page as they load.
Object.assign(globalThis, {
  document: { getElementById: () => ({ addEventListener() {} }), addEventListener() {}, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} }, hidden: false },
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  addEventListener() {},
  location: { origin: "http://localhost" },
});
const { Memories, disarmMemory, memoryActs, pressMemory } = await import("./Memories.jsx");
const { cache } = await import("../data.js");
const { keyOf } = await import("../queries.js");
const { Place } = await import("./place.js");

const CANDIDATE = { id: "c1", name: "release-day", description: "When <b>releases</b> go out.", text: "Tuesdays.", current_text: "Mondays.", replaces: true, task: "ENG-7", runner: "desktop", written: "2026-11-25T10:00:00Z" };
const EMPTY = () => ({ armed: null, busy: null, notices: new Map(), sent: new Set(), open: new Set() });

function draw(t, candidates, size = "wide") {
  cache.setQueryData(keyOf("desk"), { revision: 1, items: candidates ? [{ kind: "memories", candidates }] : [] });
  t.after(() => {
    cache.clear();
    disarmMemory();
    memoryActs.set(EMPTY());
  });
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ size }}><Memories /></Place.Provider></QueryClientProvider>);
}

test("each candidate shows its text escaped, its tag, its writer and its actions", (t) => {
  const html = draw(t, [CANDIDATE]);
  assert.match(html, /^<div class="list page"><div class="top"><a class="back" href="\/">/);
  assert.match(html, /<h1 class="question">1 memory candidate<\/h1>/);
  assert.match(html, /<section class="sect mem" aria-label="release-day"><div class="sh"><span class="tile wait sm" role="img" aria-label="memories">/);
  assert.match(html, /<span class="tag t-warn">replaces a memory runs get<\/span>/);
  assert.match(html, /<p class="desc">When &lt;b&gt;releases&lt;\/b&gt; go out\.<\/p>/);
  assert.match(html, /<details class="current"><summary>Current text<\/summary>/);
  assert.match(html, /Written by <span class="mono">ENG-7<\/span> on desktop, candidate <span class="mono">c1<\/span>/);
  assert.match(html, /<button type="button" id="memory-accept-c1" class="btn primary act " aria-disabled="false">Accept<\/button><button type="button" id="memory-drop-c1" class="btn secondary act " aria-disabled="false">Drop<\/button>/);
});

test("no candidate says how agents write them, and a phone's page has no box", (t) => {
  const html = draw(t, null, "compact");
  assert.match(html, /^<div class="flat">/);
  assert.match(html, /<h1 class="question">0 memory candidates<\/h1>.*<p class="empty">No memory candidate waits\./);
});

test("a first Accept arms with its countdown, an open Current text stays open", (t) => {
  pressMemory("c1", "accept");
  memoryActs.set({ ...memoryActs.get(), open: new Set(["c1"]) });
  const html = draw(t, [CANDIDATE]);
  assert.match(html, /class="btn primary act armed" aria-disabled="false"><span><small>Tap again to accept<\/small>Accept<\/span><span class="drain" aria-hidden="true" style="animation-delay:-\d+ms"><\/span>/);
  assert.match(html, /<details class="current" open="">/);
});

test("a second Accept sends, keeps the note and leaves the buttons off", async (t) => {
  const sent = [];
  globalThis.fetch = async (path) => {
    sent.push(path);
    return { status: 200, json: async () => ({ ok: "accepted" }) };
  };
  t.after(() => delete globalThis.fetch);
  pressMemory("c1", "accept");
  await pressMemory("c1", "accept");
  assert.deepEqual(sent, ["/api/memories/c1/accept"]);
  const html = draw(t, [CANDIDATE]);
  assert.equal(html.match(/aria-disabled="true"/g).length, 2);
  assert.match(html, /<p class="note acts-note" role="status">.*<span>Accepted, then it leaves this list\.<\/span><\/p>/);
});

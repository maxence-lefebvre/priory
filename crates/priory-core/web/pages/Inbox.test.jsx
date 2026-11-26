import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The legacy Add sheet touches the page as it loads.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, documentElement: { dataset: {} } },
  addEventListener() {},
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client } = await import("../queries.js");
const { InboxList, InboxPage, subscribing } = await import("./Inbox.jsx");
const { TerminalList, TerminalPage } = await import("./Terminal.jsx");
const { Place } = await import("./place.js");
const { keyOf } = await import("../queries.js");

const event = (subject) => ({ type: "github.pr_review", subject, time: "2026-11-25T10:00:00Z", line: "<b>said</b>" });
const task = (id, state, workflow = "build") => ({ id, key: `ENG-${id}`, state, workflow });
const INBOX = {
  revision: 41,
  unrouted: {
    owned: [
      { resource: "acme/shop#1", task: task(1, "failed"), events: [event("one")] },
      { resource: "acme/shop#2", task: task(2, "done"), events: [1, 2, 3, 4, 5, 6].map((n) => event(`e${n}`)), request_text: "Fix #2" },
      { resource: "acme/shop#3", task: task(3, "asleep"), events: [event("<script>")] },
    ],
    other: [{ source: "slack", events: 2 }],
    unheard: [{ task: task(4, "blocked"), events: [event("ping")], say_text: "Read the new events" }],
    dropped: [{ key: "acme/shop#9", trigger: "pr", script: "gate.sh", reason: "draft", event: event("drafted") }],
  },
};

function draw(t, Part, { inbox, tasks, size = "wide", error } = {}) {
  const cache = client({ api: async () => ({}) });
  if (inbox) cache.setQueryData(["inbox"], inbox);
  if (tasks) cache.setQueryData(keyOf("tasks"), { tasks });
  if (error) cache.getQueryCache().build(cache, { queryKey: ["inbox"] }).setState({ status: "error", error: new Error(error) });
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ size }}><Part /></Place.Provider></QueryClientProvider>);
}

test("the Inbox draws each kind of group, agent text escaped", (t) => {
  const html = draw(t, InboxList, { inbox: INBOX });
  assert.match(html, /^<div class="list inbox" role="region" aria-label="Inbox"><div class="lhead"><h1>Inbox<\/h1><\/div><nav class="views" aria-label="Saved views"><a href="\/inbox\?view=unrouted" aria-current="true">Unrouted<span class="num">8<\/span><\/a><\/nav><div class="scroll">/);
  assert.deepEqual([...html.matchAll(/<div class="sec"( id="\w+")?>(\w+)/g)].map((found) => found[2]), ["Unheard", "Dropped", "Unrouted"]);
  assert.match(html, /<b title="github.pr_review">GitHub pr review<\/b> &lt;script&gt;<span class="meta">&lt;b&gt;said&lt;\/b&gt;<\/span>/);
  assert.match(html, /<p class="meta">and 1 older<\/p>/);
  assert.match(html, /<p class="meta unowned">No task ever owned: 2 slack events.<\/p>/);
  assert.match(html, /<div class="keys">/);
});

test("each group routes its next event its own way", (t) => {
  const html = draw(t, InboxList, { inbox: INBOX });
  assert.match(html, /id="act-1-revive" class="btn secondary act " aria-disabled="false">Revive</);
  assert.match(html, /<button type="button" class="btn secondary">Request a task<\/button>/);
  assert.match(html, /<button type="button" class="btn secondary">Subscribe ENG-3<\/button>/);
  // A blocked task's unheard group: Say, then Resume alone, never Cancel.
  assert.match(html, /<button type="button" class="btn secondary" aria-disabled="false">Say to ENG-4<\/button><span class="task-acts"><button type="button" id="act-4-resume"[^>]*>Resume<\/button><\/span><p class="meta">Say sends “Read the new events” It reads them when it resumes.<\/p>/);
  assert.ok(!html.includes("act-4-cancel"));
  assert.match(html, /<p class="meta">Dropped: draft. A new event of it that a trigger wants runs the script again.<\/p>/);
});

test("a Subscribe in flight is off, and its note shows under its group", (t) => {
  t.after(() => subscribing.set({ busy: null, notices: new Map() }));
  subscribing.set({ busy: "acme/shop#3", notices: new Map([["acme/shop#3", "The daemon may have applied this but did not confirm it. Check the task before you send again."]]) });
  assert.match(draw(t, InboxList, { inbox: INBOX }), /aria-disabled="true">Subscribe ENG-3<\/button><\/div><p class="note" role="status">The daemon may have applied this/);
});

test("the Inbox loads, fails, and keeps its groups through a failed re-read", (t) => {
  assert.match(draw(t, InboxList), /Unrouted<\/a><\/nav><div class="scroll"><p class="empty">Loading the unrouted events…<\/p>/);
  assert.match(draw(t, InboxList, { error: "the connection dropped" }), /<p class="empty">Could not read the unrouted events: the connection dropped. <button type="button" class="btn secondary">Try again<\/button><\/p>/);
  const kept = draw(t, InboxList, { inbox: INBOX, error: "the connection dropped" });
  assert.match(kept, /Could not read the unrouted events/);
  assert.match(kept, /Subscribe ENG-3/);
});

test("an Inbox with nothing unrouted says so, and a phone draws its page", (t) => {
  const empty = { revision: 1, unrouted: { owned: [], other: [] } };
  assert.match(draw(t, InboxList, { inbox: empty, size: "medium" }), /<div class="scroll"><p class="empty">No event of a task&#x27;s resource went unrouted.[^<]*<\/p><\/div><\/div>$/);
  assert.match(draw(t, InboxPage, { inbox: empty }), /^<div class="top"><h1>Inbox<\/h1><span class="sp"><\/span>.*<div class="panel" role="region" aria-label="Inbox"><div class="scroll"><nav class="views"/);
});

test("the Terminal panel lists the running tasks", (t) => {
  const tasks = [{ id: 8, key: "ENG-8", state: "running", title: "Ship <it>" }, { id: 9, key: "ENG-9", state: "done" }];
  const html = draw(t, TerminalList, { tasks });
  assert.match(html, /^<div class="list" role="region" aria-label="Terminal"><div class="lhead"><h1>Terminal<\/h1><\/div><ol class="hist" aria-label="Running tasks"><li><span class="tile [a-z]+ sm" role="img" aria-label="running">/);
  assert.match(html, /<\/span><a href="\/tasks\/8#terminal" title="task 8"><span class="mono">ENG-8<\/span> Ship &lt;it&gt;<\/a><\/li><\/ol><\/div>$/);
  assert.ok(!html.includes("ENG-9"));
  assert.match(draw(t, TerminalPage, { tasks: [] }), /^<div class="top"><h1>Terminal<\/h1><\/div><p class="empty">No agent runs now. A running task&#x27;s terminal shows here.<\/p>$/);
  assert.match(draw(t, TerminalPage), /<p class="empty" role="status">Loading…<\/p>/);
});

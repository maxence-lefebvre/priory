import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The legacy pages, which the task route falls back to, read these as they load.
const element = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => element, documentElement: { dataset: {} } },
  addEventListener() {},
});
const { acts, draft, pressTask, disarmTask } = await import("../acts.js");
const { client } = await import("../queries.js");
const { TaskBody } = await import("./TaskPage.jsx");

const TASK = { id: 7, key: "ENG-7", state: "blocked", lane: "web", band: "now", tldr: "Fix <b>it</b>", reason: "waits on #3", repo: "acme/shop", asks: [], runs: [], inbox: [] };

function draw(t, task, { asks = [], tasks = [] } = {}) {
  const cache = client({ api: async () => ({}) });
  cache.setQueryData(["links"], [{ pattern: "#(\\d+)", url: "https://github.com/{repo}/issues/$1" }]);
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}><TaskBody task={{ ...TASK, ...task }} asks={asks} tasks={tasks} /></QueryClientProvider>);
}

test("a task's head, reason and actions", (t) => {
  const html = draw(t, {});
  assert.match(html, /<p class="ctx"><span class="tile warn sm" role="img" aria-label="blocked">/);
  assert.match(html, /<h1 class="question">Fix &lt;b&gt;it&lt;\/b&gt;<\/h1><p class="text t-warn">waits on <a class="autolink" href="https:\/\/github.com\/acme\/shop\/issues\/3"/);
  assert.deepEqual([...html.matchAll(/id="act-7-(\w+)"/g)].map(([, act]) => act), ["resume", "cancel"]);
});

test("the Say box shows the task's draft, and an ended task has none", (t) => {
  t.after(() => draft(7, ""));
  draft(7, "a </textarea> draft");
  assert.match(draw(t, {}), /<textarea id="say" name="text" rows="2" placeholder="Say to the agent">a &lt;\/textarea&gt; draft<\/textarea>/);
  assert.ok(!draw(t, { state: "done" }).includes("say"));
});

test("an armed Cancel asks for a second tap, and shows its countdown", (t) => {
  t.after(disarmTask);
  pressTask(7, "cancel", {});
  assert.match(draw(t, {}), /<button type="button" id="act-7-cancel" class="btn secondary act armed" aria-disabled="false"><span><small>Tap again to cancel<\/small>Cancel<\/span><span class="drain" aria-hidden="true" style="animation-delay:-\d+ms"><\/span><\/button>/);
});

test("an action in flight disables them all", (t) => {
  t.after(() => acts.set({ ...acts.get(), busy: null }));
  acts.set({ ...acts.get(), busy: 7 });
  assert.equal(draw(t, {}).match(/aria-disabled="true"/g).length, 3);
});

test("each section shows when the task holds it, agent text escaped", (t) => {
  const html = draw(t, {
    brief: { request: "Do **this** <script>", result: { step: "plan", run: 3, summary: "Planned" } },
    progress: { goal: "Ship", criteria: [], latestChange: "None", nextAction: "Go" },
    plan: { step: "plan", run: 3, summary: "The plan" },
    triage: [{ item: "https://github.com/acme/shop/pull/1", class: "trivial" }],
    inbox: [{ id: 1, type: "github.pr", subject: "opened", time: "2026-11-25T10:00:00Z", outcome: "kept", task: 7 }],
    runs: [{ id: 3, step: "plan", started: "2026-11-25T10:00:00Z", ended: "2026-11-25T10:05:00Z", end: "moved", outcome: "planned" }],
    scores: [{ run: "3", scorer: "brief", verdict: "pass", reason: "clear" }],
  });
  assert.deepEqual([...html.matchAll(/<section class="sect" id="(\w+)"/g)].map(([, id]) => id), ["brief", "progress", "plan", "pr", "inbox", "trace"]);
  assert.match(html, /Do <strong>this<\/strong> &#60;script&#62;/);
  assert.match(html, /<b>plan<\/b>, run <span class="mono">3<\/span>, 5 min, planned/);
  assert.match(html, /<ol class="scores" aria-label="Scores of run 3">/);
  assert.match(html, />acme\/shop\/pull\/1<\/a>/);
});

test("a bare task draws no section", (t) => {
  assert.ok(!draw(t, {}).includes("<section"));
});

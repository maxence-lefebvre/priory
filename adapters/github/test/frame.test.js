import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import "./idb.js";
import { DOMParser, Node, parse } from "./dom.js";
import { clear, put } from "../lib/store.js";

// frame.js as the strip or the docked panel on octo/demo#1, with the daemon behind a stubbed fetch.
class Channel extends BroadcastChannel {
  constructor(name) {
    super(name);
    this.unref();
  }
}
Object.defineProperty(navigator, "locks", { configurable: true, value: { request: () => new Promise(() => {}) } });

const PR = "github:octo/demo#1";
const TASK = { id: 7, key: PR, workflow: "build", step: "implement", state: "running", lane: "cli", runs: [{ step: "implement" }], asks: [] };
const HOLDER = { id: 7, key: PR, workflow: "build", step: "implement", state: "running", holds: ["key"] };
const FLOWS = { workflows: [{ name: "build", steps: [{ id: "implement", decides: "model" }] }] };
const reply = (status, body) => Promise.resolve({ status, json: async () => body });
const ok = (holders) => (url) =>
  url.includes("/api/tasks?pr=") ? reply(200, { tasks: holders }) : url.endsWith("/api/flows") ? reply(200, FLOWS) : reply(200, TASK);

const settle = async (check, what) => {
  for (let i = 0; i < 200; i += 1) {
    if (check()) return;
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.fail(`timed out waiting for ${what}`);
};

let run = 0;
/** Boots frame.js as `view` and returns the calls it made. `daemon(url, init)` answers each one. */
async function boot(view, daemon, cached) {
  await clear();
  await put("config", { origin: "http://127.0.0.1:7340", token: "t" });
  if (cached) await put(`last:${PR}`, cached);
  const calls = [];
  Object.assign(globalThis, {
    Node,
    DOMParser,
    BroadcastChannel: Channel,
    document: parse(readFileSync(new URL("../frame.html", import.meta.url), "utf8")),
    location: new URL(`chrome-extension://x/frame.html?view=${view}&mode=light`),
    chrome: {
      tabs: {
        getCurrent: async () => ({ id: 1, url: "https://github.com/octo/demo/pull/1" }),
        sendMessage: async () => ({ json: null, head: null, failed: false }),
      },
    },
    fetch: (url, init) => {
      calls.push([url, init.method, init.body && JSON.parse(init.body)]);
      return daemon(url, init);
    },
  });
  await import(`../frame.js?run=${(run += 1)}`);
  return calls;
}
const text = () => document.body.textContent;

test("the strip shows a cached empty list as a read in progress, not as no task, until the daemon answers", async () => {
  let answer;
  await boot("strip", (url) => (url.includes("/api/tasks?pr=") ? new Promise((resolve) => (answer = resolve)) : ok([HOLDER])(url)), { holders: [], task: null, readAt: Date.now() - 60000 });
  await settle(() => answer, "the read");
  assert.match(text(), /Reading Priory/);
  assert.doesNotMatch(text(), /No Priory task/);
  answer({ status: 200, json: async () => ({ tasks: [HOLDER] }) });
  await settle(() => /task 7/.test(text()), "the task");
});

test("a 503, a timeout or a network failure reads as retrying, never as no task, and the read is tried again", async (t) => {
  for (const failure of [() => reply(503, { error: "busy" }), () => Promise.reject(new DOMException("timed out", "TimeoutError")), () => Promise.reject(new TypeError("fetch failed"))]) {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    let failing = true;
    const calls = await boot("panel", (url) => (failing && url.includes("/api/tasks?pr=") ? failure() : ok([HOLDER])(url)), { holders: [], task: null, readAt: Date.now() - 60000 });
    await settle(() => /Retrying/.test(text()), "the retrying state");
    assert.doesNotMatch(text(), /No Priory task/);
    assert.doesNotMatch(text(), /Add a task/);
    failing = false;
    t.mock.timers.tick(2000);
    await settle(() => /task 7/.test(text()), "the task after the retry");
    assert.equal(calls.filter(([url]) => url.includes("/api/tasks?pr=")).length, 2);
    t.mock.timers.reset();
  }
});

test("an empty list the daemon confirms says no task holds the pull request, with Add a task", async () => {
  await boot("strip", ok([]));
  await settle(() => /No Priory task owns this pull request/.test(text()), "no task");
  assert.match(text(), /Add a task/);
});

test("with a task and no review guide, the Guide tab offers Generate guide, which says the fixed text to the task", async () => {
  const calls = await boot("panel", (url, init) => (init.method === "POST" ? reply(200, {}) : ok([HOLDER])(url)));
  await settle(() => /task 7/.test(text()), "the task");
  const tab = [...document.body.querySelectorAll('[role="tab"]')].find((button) => button.textContent === "Guide");
  assert.ok(tab, "the Guide tab shows");
  tab.click();
  await settle(() => /Generate guide/.test(text()), "Generate guide");
  assert.match(text(), /This description has no review guide/);
  const generate = [...document.body.querySelectorAll("button")].find((button) => button.textContent.includes("Generate guide"));
  assert.equal(generate.disabled, false);
  generate.click();
  await settle(() => calls.some(([, method]) => method === "POST"), "the say");
  const [url, , body] = calls.find(([, method]) => method === "POST");
  assert.equal(url, "http://127.0.0.1:7340/api/tasks/7/say");
  assert.equal(body.pull_request, PR);
  assert.match(body.text, /review guide/);
  assert.match(body.text, /https:\/\/github\.com\/octo\/demo\/pull\/1/);
});

test("with no task, the Guide tab stays hidden when the description has no guide", async () => {
  await boot("panel", ok([]));
  await settle(() => /No Priory task owns this pull request/.test(text()), "no task");
  assert.equal([...document.body.querySelectorAll('[role="tab"]')].some((button) => button.textContent === "Guide"), false);
});

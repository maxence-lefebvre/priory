import { test } from "node:test";
import assert from "node:assert/strict";
import { screenWatch, rowParts } from "./terminal.js";

// A clock the test moves by hand, and a server whose replies the test releases one by one.
function rig() {
  const timers = [];
  const asked = [];
  let now = 0;
  const watch = screenWatch({
    read: (run, have) => new Promise((resolve, reject) => asked.push({ run, have, resolve, reject })),
    timer: (fn, ms) => { const timer = { fn, at: now + ms }; timers.push(timer); return timer; },
    clear: (timer) => { const i = timers.indexOf(timer); if (i >= 0) timers.splice(i, 1); },
    now: () => now,
  });
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    watch, asked, timers,
    async tick(ms) {
      now += ms;
      for (const timer of timers.filter((timer) => timer.at <= now)) { timers.splice(timers.indexOf(timer), 1); timer.fn(); }
      await settle();
    },
    async answer(reply) { asked.at(-1).resolve(reply); await settle(); },
    async refuse(status) { asked.at(-1).reject(Object.assign(new Error(`answered ${status}`), { status })); await settle(); },
  };
}

const screen = (run, hash, lines = ["$ cargo test"]) => ({ run, task: 1, width: 200, height: 50, cursor: null, lines, redacted: 0, hash });

test("it polls a second after each reply while shown, and once at most in flight", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  assert.deepEqual(asked.map(({ run, have }) => [run, have]), [[4, null]]);
  watch.show(4);
  await tick(5000);
  assert.equal(asked.length, 1);
  await answer(screen(4, "a"));
  assert.equal(watch.state().screen.hash, "a");
  await tick(999);
  assert.equal(asked.length, 1);
  await tick(1);
  assert.deepEqual(asked.map(({ run, have }) => [run, have]), [[4, null], [4, "a"]]);
});

test("it backs off to 3 seconds after 10 unchanged replies, and a new revision polls at once", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  for (let i = 0; i < 10; i++) {
    await tick(1000);
    await answer({ hash: "a", same: true });
  }
  await tick(2999);
  assert.equal(asked.length, 11);
  await tick(1);
  assert.equal(asked.length, 12);
  await answer(screen(4, "b"));
  await tick(1000);
  assert.equal(asked.length, 13);
  await answer({ hash: "b", same: true });
  watch.revision();
  assert.equal(asked.length, 14);
});

test("it stops while hidden and starts again when shown", async () => {
  const { watch, asked, tick, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  watch.hide();
  await tick(10000);
  watch.revision();
  assert.equal(asked.length, 1);
  watch.show(4);
  assert.equal(asked.length, 2);
  assert.equal(watch.state().screen.hash, "a");
});

test("a reply for the run shown before a switch is dropped, and the switch clears the screen", async () => {
  const { watch, asked, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await new Promise((resolve) => setImmediate(resolve));
  watch.revision();
  watch.show(5);
  assert.equal(watch.state().screen, null);
  asked[1].resolve(screen(4, "late"));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(watch.state().screen, null);
  assert.deepEqual(asked.map(({ run }) => run), [4, 4, 5]);
  await answer(screen(5, "c"));
  assert.equal(watch.state().screen.hash, "c");
});

test("a 401 and a sign-out clear the screen and stop polling", async () => {
  const { watch, asked, tick, answer, refuse } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await tick(1000);
  await refuse(401);
  assert.equal(watch.state().screen, null);
  await tick(10000);
  assert.equal(asked.length, 2);

  watch.show(5);
  await answer(screen(5, "b"));
  watch.stop();
  assert.equal(watch.state().screen, null);
  await tick(10000);
  assert.equal(asked.length, 3);
});

test("a run that ended keeps its last screen and stops polling; a failed read keeps it and retries", async () => {
  const { watch, asked, tick, answer, refuse } = rig();
  watch.show(4);
  await answer(screen(4, "a"));
  await tick(1500);
  await refuse(503);
  assert.deepEqual([watch.state().screen.hash, watch.state().stale], ["a", true]);
  assert.equal(watch.state().at, 0);
  await tick(1000);
  await refuse(404);
  assert.deepEqual([watch.state().screen.hash, watch.state().gone], ["a", true]);
  await tick(10000);
  assert.equal(asked.length, 3);
});

test("the watch keeps no screen in local storage", async () => {
  const stored = [];
  globalThis.localStorage = { setItem: (...args) => stored.push(args), getItem: () => null, removeItem: () => {} };
  const { watch, answer } = rig();
  watch.show(4);
  await answer(screen(4, "a", ["token [redacted]"]));
  assert.deepEqual(stored, []);
  delete globalThis.localStorage;
});

test("a row splits into text and hidden parts, and markup stays text", () => {
  assert.deepEqual(rowParts("GH_TOKEN=[redacted] and <script>alert(1)</script>"), [
    { text: "GH_TOKEN=", hidden: false },
    { text: "[redacted]", hidden: true },
    { text: " and <script>alert(1)</script>", hidden: false },
  ]);
  assert.deepEqual(rowParts(""), []);
});

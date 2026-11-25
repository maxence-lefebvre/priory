import { test } from "node:test";
import assert from "node:assert/strict";
import { follow } from "./events.js";

// Runs follow() over scripted answers to the long poll, and records what it did.
async function run({ ready = () => 4, held = ready, answers, refreshed = true }) {
  const did = [];
  let turns = answers.length;
  await follow({
    live: () => turns-- > 0,
    ready,
    held,
    wait: async (since) => {
      did.push(`wait ${since}`);
      const answer = answers.shift();
      if (answer instanceof Error) throw answer;
      return { revision: answer };
    },
    moved: () => did.push("moved"),
    refresh: async () => (did.push("refresh"), refreshed),
    sleep: async (ms) => did.push(`sleep ${ms}`),
  });
  return did;
}

test("a moved revision reads the listings again", async () => {
  assert.deepEqual(await run({ answers: [5] }), ["wait 4", "moved", "refresh"]);
});

test("an unchanged revision waits again without a read", async () => {
  assert.deepEqual(await run({ answers: [4, 4] }), ["wait 4", "wait 4"]);
});

test("a failed wait reads again and pauses 5 s", async () => {
  assert.deepEqual(await run({ answers: [new TypeError("offline")] }), ["wait 4", "refresh", "sleep 5000"]);
});

test("a failed read after a move pauses 5 s", async () => {
  assert.deepEqual(await run({ answers: [5], refreshed: false }), ["wait 4", "moved", "refresh", "sleep 5000"]);
});

test("no wait while signed out, hidden or before the first read", async () => {
  assert.deepEqual(await run({ ready: () => undefined, answers: [5] }), ["sleep 1000"]);
});

// The tab hides, or the person signs out, once the wait is out.
const leaves = () => {
  let asked = 0;
  return () => (asked++ ? undefined : 4);
};

test("a wait that answers after the tab hid reads nothing", async () => {
  assert.deepEqual(await run({ ready: leaves(), held: () => 4, answers: [5] }), ["wait 4"]);
});

test("a wait that fails after the tab hid reads nothing", async () => {
  assert.deepEqual(await run({ ready: leaves(), held: () => 4, answers: [new TypeError("offline")] }), ["wait 4"]);
});

test("a revision that a refresh took during the wait reads nothing again", async () => {
  assert.deepEqual(await run({ held: () => 5, answers: [5] }), ["wait 4"]);
});

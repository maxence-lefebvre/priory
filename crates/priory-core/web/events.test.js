import { test } from "node:test";
import assert from "node:assert/strict";
import { follow } from "./events.js";

// Runs follow() over scripted answers to the long poll, and records what it did.
async function run({ ready = () => 4, answers, refreshed = true }) {
  const did = [];
  let turns = answers.length;
  await follow({
    live: () => turns-- > 0,
    ready,
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

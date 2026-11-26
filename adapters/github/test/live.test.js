import { test } from "node:test";
import assert from "node:assert/strict";

// A channel that does not hold the test process open once the test ends.
globalThis.BroadcastChannel = class extends BroadcastChannel {
  constructor(name) {
    super(name);
    this.unref();
  }
};
const { backoff, hidden } = await import("../lib/live.js");

test("a busy or lost server is called again later and later, up to a minute, and a 200 resets it", () => {
  const none = () => 0;
  const waits = [];
  let wait = 0;
  for (const status of [503, 503, 503, 0, 0, 0, 0, 0]) waits.push((wait = backoff(status, wait, none)));
  assert.deepEqual(waits, [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]);
  assert.equal(backoff(200, 60000, none), 0);
  assert.equal(backoff(0, 0, none), 5000);
  assert.equal(backoff(503, 0, () => 1), 1200);
});

test("only a tab the browser says is hidden counts as hidden", () => {
  globalThis.document = { visibilityState: "hidden" };
  assert.equal(hidden(), true);
  globalThis.document.visibilityState = "visible";
  assert.equal(hidden(), false);
  delete globalThis.document;
  assert.equal(hidden(), false);
});

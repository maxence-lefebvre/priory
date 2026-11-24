import { test } from "node:test";
import assert from "node:assert/strict";
import { store } from "./store.js";

test("a store notifies its listeners on a change only", () => {
  const value = store(1);
  let heard = 0;
  const stop = value.subscribe(() => heard++);
  value.set(1);
  value.set(2);
  assert.equal(value.get(), 2);
  assert.equal(heard, 1);
  stop();
  value.set(3);
  assert.equal(heard, 1);
});

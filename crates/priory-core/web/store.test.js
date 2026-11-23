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

test("a store compares with the function it was given", () => {
  const value = store({ a: 1 }, (x, y) => x.a === y.a);
  let heard = 0;
  value.subscribe(() => heard++);
  value.set({ a: 1 });
  assert.equal(heard, 0);
});

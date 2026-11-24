import { test } from "node:test";
import assert from "node:assert/strict";
import { followBack, go, route } from "./router.js";

test("go and Back each make a visit, also to the address already shown", (t) => {
  const page = new EventTarget();
  const pushed = [];
  Object.assign(globalThis, { history: { pushState: (...args) => pushed.push(args[2]) }, addEventListener: page.addEventListener.bind(page), removeEventListener: page.removeEventListener.bind(page) });
  t.after(() => { for (const name of ["history", "addEventListener", "removeEventListener"]) delete globalThis[name]; });
  let visits = 0;
  t.after(route.subscribe(() => visits++));
  go("/asks");
  go("/asks");
  assert.deepEqual([pushed, visits], [["/asks", "/asks"], 2]);
  const stop = followBack();
  page.dispatchEvent(new Event("popstate"));
  assert.equal(visits, 3);
  stop();
  page.dispatchEvent(new Event("popstate"));
  assert.equal(visits, 3);
});

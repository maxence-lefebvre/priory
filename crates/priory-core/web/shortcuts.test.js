import { test } from "node:test";
import assert from "node:assert/strict";
import { shortcut } from "./shortcuts.js";

const rows = [{ href: "/asks/1" }, { href: "/asks/2", current: true }, { href: "/asks/3" }];
function press(key, part = {}) {
  const done = [];
  shortcut({ key, preventDefault: () => {} }, { at: { ask: "2", listUrl: "/?view=needs" }, rows, onBody: true, go: (href) => done.push(href), add: () => done.push("add"), keys: () => false, ...part });
  return done;
}

test("J and K move along the rows, and stop at each end", () => {
  assert.deepEqual([press("j"), press("k")], [["/asks/3"], ["/asks/1"]]);
  assert.deepEqual(press("j", { rows: [{ href: "/asks/9", current: true }] }), ["/asks/9"]);
  assert.deepEqual(press("j", { rows: [] }), []);
});

test("Enter opens the current row only when no control has the focus", () => {
  assert.deepEqual(press("Enter"), ["/asks/2"]);
  assert.deepEqual(press("Enter", { onBody: false }), []);
});

test("Esc goes back to the list the ask came from, and does nothing without one", () => {
  assert.deepEqual(press("Escape"), ["/?view=needs"]);
  assert.deepEqual(press("Escape", { at: { ask: null, listUrl: "/" } }), []);
});

test("N adds, and a page's own keys come before the moves", () => {
  assert.deepEqual([press("n"), press("N")], [["add"], ["add"]]);
  assert.deepEqual(press("Escape", { keys: () => true }), []);
});

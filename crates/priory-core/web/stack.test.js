import { test } from "node:test";
import assert from "node:assert/strict";
import { checkState, prStack, stateWords } from "./stack.js";

const pull = (number, branch, base, state) => ({ pull_request: `github:acme/app#${number}`, number, branch, base, state });
const latest = { number: 59, checks: [{ name: "test", conclusion: "success" }, { name: "clippy", conclusion: "success" }] };

test("a stack of three reads 3 PRs and lists bottom first, with the lowest open one next to merge and the latest one's checks", () => {
  const stack = [pull(57, "a", "main", "merged"), pull(58, "b", "a", "open"), pull(59, "c", "b", "open")];

  assert.deepEqual(prStack(stack, latest), {
    label: "3 PRs",
    entries: [
      { number: 57, url: "https://github.com/acme/app/pull/57", branch: "a", base: "main", state: "merged", next: false, checks: null },
      { number: 58, url: "https://github.com/acme/app/pull/58", branch: "b", base: "a", state: "open", next: true, checks: null },
      { number: 59, url: "https://github.com/acme/app/pull/59", branch: "c", base: "b", state: "open", next: false, checks: "2 checks pass" },
    ],
  });
});

test("one pull request or none keeps the single PR look: no label and no stack list", () => {
  assert.deepEqual(prStack([pull(59, "c", "main", "open")], latest), { label: null, entries: [] });
  assert.deepEqual(prStack([], null), { label: null, entries: [] });
  assert.deepEqual(prStack(undefined), { label: null, entries: [] });
});

test("a skipped or neutral check does not count as failing", () => {
  const stack = [pull(58, "b", "main", "open"), pull(59, "c", "b", "open")];
  const checks = [{ name: "test", conclusion: "success" }, { name: "docs", conclusion: "skipped" }, { name: "lint", conclusion: "neutral" }];
  assert.equal(prStack(stack, { number: 59, checks }).entries[1].checks, "3 checks pass");
  assert.equal(prStack(stack, { number: 59, checks: [...checks, { name: "e2e", conclusion: "timed_out" }] }).entries[1].checks, "1 of 4 checks failing");
});

test("a check draws as passed, running, skipped or failed, never failed while unfinished", () => {
  assert.equal(checkState("success"), "passed");
  assert.equal(checkState(null), "running");
  assert.equal(checkState(""), "running");
  assert.equal(checkState("skipped"), "skipped");
  assert.equal(checkState("neutral"), "skipped");
  assert.equal(checkState("failure"), "failed");
  assert.equal(checkState("timed_out"), "failed");
});

test("review and check states read in plain words", () => {
  assert.equal(stateWords("changes_requested"), "changes requested");
  assert.equal(stateWords("success"), "passed");
  assert.equal(stateWords("timed_out"), "timed out");
  assert.equal(stateWords("approved"), "approved");
  assert.equal(stateWords(null), "");
});

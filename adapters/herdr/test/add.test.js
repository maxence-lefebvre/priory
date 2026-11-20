import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { addChoices, addStatus } from "../lib/add.js";

const fixture = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));

test("the Add chips offer the factory's bands in order, and each workflow and lane the tasks carry", () => {
  const queue = { ...fixture("queue.json"), waiting: [...fixture("queue.json").waiting, { lane: "docs/site", band: "now" }] };
  const tasks = [{ workflow: "review", lane: "none/misc" }, { workflow: "implement", lane: null }, { workflow: "review", lane: "app/web" }];
  assert.deepEqual(addChoices(queue, tasks), {
    workflow: ["implement", "review"],
    band: ["blocker", "now", "today", "later"],
    lane: ["app/web", "docs/site", "none/misc"],
  });
});

test("the Add line says what to type, that it sends, why priory refused, or the new task's key", () => {
  assert.deepEqual(addStatus({ text: "", sending: false, error: null, key: null }),
    { text: "Workflow, band and lane come from the factory's [request] table unless you pick one.", tone: "muted", ready: false });
  assert.deepEqual(addStatus({ text: "Fix the docs", sending: false, error: null, key: null }).ready, true);
  assert.deepEqual(addStatus({ text: "Fix the docs", sending: true, error: null, key: null }), { text: "Adding…", tone: "muted", ready: false });
  assert.deepEqual(addStatus({ text: "Fix the docs", sending: false, error: "unknown workflow `nope`", key: null }),
    { text: "Add did not go: unknown workflow `nope`.", tone: "warn", ready: true });
  assert.deepEqual(addStatus({ text: "", sending: false, error: null, key: "cli:2026-11-21-1" }), { text: "Added cli:2026-11-21-1.", tone: "ok", ready: false });
});

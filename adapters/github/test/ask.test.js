import { test } from "node:test";
import assert from "node:assert/strict";
import { ARM_MS, UNDO_MS, choose, due, observe, reply, undo } from "../lib/ask.js";

const ASK = { id: 88, revision: 1, state: "open", options: ["Rename it", "Keep the name"], command: null };
const COMMAND = { ...ASK, id: 90, options: ["Run it", "Skip"], command: { argv: ["make", "deploy"], target: "staging", runs: ["Run it"] } };
const NOW = Date.parse("2026-11-21T12:00:00Z");

test("an option waits 4 s with Undo, then sends with the revision the panel showed", () => {
  const waiting = choose(null, ASK, { option: "Rename it" }, NOW);

  assert.equal(waiting.phase, "waiting");
  assert.equal(due(waiting, NOW + UNDO_MS - 1), null);
  assert.equal(undo(waiting), null);
  assert.deepEqual(due(waiting, NOW + UNDO_MS), {
    draft: { ...waiting, phase: "posting" },
    path: "/api/asks/88/answer",
    body: { option: "Rename it", revision: 1 },
  });
});

test("a written answer sends as text, and a second choice while one waits changes nothing", () => {
  const waiting = choose(null, ASK, { text: "Rename it to flows_payload" }, NOW);

  assert.deepEqual(due(waiting, NOW + UNDO_MS).body, { text: "Rename it to flows_payload", revision: 1 });
  assert.equal(choose(waiting, ASK, { option: "Keep the name" }, NOW), waiting);
});

test("a command option arms with no Undo; only its side panel run sends, with run: true", () => {
  const armed = choose(null, COMMAND, { option: "Run it" }, NOW);

  assert.equal(armed.phase, "armed");
  assert.equal(due(armed, NOW + UNDO_MS), null);
  assert.equal(undo(armed), armed);
  assert.equal(choose(null, COMMAND, { option: "Skip" }, NOW).phase, "waiting");
});

test("an armed command disarms when its 4 s pass or the question moves to a new revision", () => {
  const armed = choose(null, COMMAND, { option: "Run it" }, NOW);

  assert.equal(observe(armed, COMMAND, NOW + ARM_MS - 1), armed);
  assert.equal(observe(armed, COMMAND, NOW + ARM_MS), null);
  assert.equal(observe(armed, { ...COMMAND, revision: 2 }, NOW + 1), null);
});

test("a 200 clears the draft", () => {
  const posting = due(choose(null, ASK, { option: "Rename it" }, NOW), NOW + UNDO_MS).draft;
  assert.equal(reply(posting, { status: 200, body: { ok: "ask 88 answered" } }, NOW).phase, "sent");
});

test("each refusal keeps the draft and says why in the panel's words", () => {
  const posting = due(choose(null, ASK, { option: "Rename it" }, NOW), NOW + UNDO_MS).draft;
  const cases = [
    [{ status: 409, body: { ask: { ...ASK, revision: 3 } } }, "refused", "The question changed. Read revision 3 and answer again."],
    [{ status: 409, body: { ask: { ...ASK, state: "answered", answer: "Merge", answered_at: "2026-11-21T11:58:00Z" } } }, "taken", "Already answered 2 min ago: Merge."],
    [{ status: 409, body: { ask: { ...ASK, state: "withdrawn", withdrawn_why: "its task was revived" } } }, "withdrawn", "Withdrawn: its task was revived."],
    [{ status: 422, body: { error: "step approve takes one of its options" } }, "unsent", "step approve takes one of its options."],
    [{ status: 503, body: { error: "no daemon" } }, "unsent", "The daemon is not running: nothing was sent."],
    [{ status: 403, body: {} }, "unsent", "Priory refused this extension's origin. Add it to [serve] extension_origins."],
    [{ status: 401, body: {} }, "signin", "The daemon refused this token. Paste the current one."],
  ];
  for (const [result, phase, notice] of cases) {
    const after = reply(posting, result, NOW);
    assert.equal(after.phase, phase, notice);
    assert.equal(after.notice, notice);
    assert.equal(after.option, "Rename it");
  }
});

test("a lost write is outcome-unknown until a read shows the answer, or the person sends again", () => {
  const posting = due(choose(null, ASK, { option: "Rename it" }, NOW), NOW + UNDO_MS).draft;
  const unknown = reply(posting, { status: 0, body: {}, unknown: true }, NOW);

  assert.equal(unknown.phase, "unknown");
  assert.equal(unknown.notice, "This may have reached Priory. Check the task, then send again if it is missing.");
  assert.equal(choose(unknown, ASK, { option: "Keep the name" }, NOW), unknown);
  assert.equal(observe(unknown, ASK, NOW), unknown);
  assert.equal(observe(unknown, { ...ASK, state: "answered", answer: "Rename it" }, NOW).phase, "sent");
  assert.equal(choose(unknown, ASK, { option: "Rename it", again: true }, NOW).phase, "waiting");
});

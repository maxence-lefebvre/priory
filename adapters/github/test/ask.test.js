import { test } from "node:test";
import assert from "node:assert/strict";
import { ARM_MS, UNDO_MS, choose, due, leftUnsent, observe, optionKey, reply, dismissable, undo } from "../lib/ask.js";

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
    body: { option: "Rename it", revision: 1, call: waiting.call },
  });
});

test("a written answer sends as text, and a second choice while one waits changes nothing", () => {
  const waiting = choose(null, ASK, { text: "Rename it to flows_payload" }, NOW);

  assert.deepEqual(due(waiting, NOW + UNDO_MS).body, { text: "Rename it to flows_payload", revision: 1, call: waiting.call });
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

test("a resend of the same choice keeps its call id, and another choice or revision gets a new one", () => {
  const lost = { ...choose(null, ASK, { option: "Rename it" }, NOW), phase: "unknown" };
  const again = choose(lost, ASK, { option: "Rename it", again: true }, NOW + 1);
  const other = choose(lost, ASK, { option: "Keep the name", again: true }, NOW + 1);
  const moved = choose(lost, { ...ASK, revision: 2 }, { option: "Rename it", again: true }, NOW + 1);

  assert.match(lost.call, /^[0-9a-f-]{36}$/);
  assert.equal(again.call, lost.call);
  assert.notEqual(other.call, lost.call);
  assert.notEqual(moved.call, lost.call);
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

test("a digit key picks its option only alone: a browser shortcut or a held key picks nothing", () => {
  const key = (key, extra = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, repeat: false, ...extra });

  assert.equal(optionKey(key("2")), 1);
  assert.equal(optionKey(key("9")), 8);
  for (const event of [key("2", { ctrlKey: true }), key("2", { metaKey: true }), key("2", { altKey: true }), key("2", { repeat: true }), key("0"), key("a")]) {
    assert.equal(optionKey(event), -1, JSON.stringify(event));
  }
});

test("an answer still waiting on Undo when the page changes comes back unsent, to send or discard", () => {
  const waiting = choose(null, ASK, { option: "Rename it" }, NOW);
  const kept = leftUnsent(waiting);

  assert.deepEqual(kept, { ...waiting, phase: "unsent", restored: true, notice: 'Not sent: "Rename it". The page changed before its 4 s Undo passed.' });
  assert.equal(leftUnsent(kept), kept);
  assert.equal(leftUnsent(choose(null, ASK, { text: "Rename it to flows_payload" }, NOW)).notice, 'Not sent: "Rename it to flows_payload". The page changed before its 4 s Undo passed.');
  const again = choose(kept, ASK, { option: "Rename it" }, NOW + 1);
  assert.equal(again.phase, "waiting");
  assert.equal(again.restored, undefined);
});

test("a restored draft drops once its ask is asked again at a new revision, and stays at the same one", () => {
  const kept = leftUnsent(choose(null, ASK, { option: "Rename it" }, NOW));

  assert.equal(observe(kept, ASK, NOW + 60000), kept);
  assert.equal(observe(kept, { ...ASK, revision: 2 }, NOW + 60000), null);
});

test("only a waiting answer is kept: one posting, armed, sent or undone is not", () => {
  const waiting = choose(null, ASK, { option: "Rename it" }, NOW);
  const posting = due(waiting, NOW + UNDO_MS).draft;

  for (const draft of [null, undo(waiting), posting, reply(posting, { status: 200 }, NOW), choose(null, COMMAND, { option: "Run it" }, NOW)]) {
    assert.equal(leftUnsent(draft), null, draft?.phase);
  }
});

test("a step's ask offers no dismissal, and an agent's ask does", () => {
  assert.equal(dismissable({ ...ASK, dismissable: false }), false);
  assert.equal(dismissable({ ...ASK, dismissable: true }), true);
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { ARM_MS, answerBody, answerPath, choose, observe, optionKey, reply, dismissable } from "../lib/ask.js";

const ASK = { id: 88, revision: 1, state: "open", options: ["Rename it", "Keep the name"], command: null };
const COMMAND = { ...ASK, id: 90, options: ["Run it", "Skip"], command: { argv: ["make", "deploy"], target: "staging", runs: ["Run it"] } };
const NOW = Date.parse("2026-11-21T12:00:00Z");
const posting = () => choose(choose(null, ASK, { option: "Rename it" }, NOW), ASK, { option: "Rename it" }, NOW + 1);

test("a first press arms an option, and a second on it within 4 s sends it with the revision the panel showed", () => {
  const armed = choose(null, ASK, { option: "Rename it" }, NOW);
  const sent = choose(armed, ASK, { option: "Rename it" }, NOW + ARM_MS - 1);

  assert.equal(armed.phase, "confirm");
  assert.equal(sent.phase, "posting");
  assert.equal(answerPath(sent.id), "/api/asks/88/answer");
  assert.deepEqual(answerBody(sent), { option: "Rename it", revision: 1, call: armed.call });
});

test("a press on another option, or after 4 s, arms again and sends nothing", () => {
  const armed = choose(null, ASK, { option: "Rename it" }, NOW);

  assert.equal(choose(armed, ASK, { option: "Keep the name" }, NOW + 1).phase, "confirm");
  assert.equal(choose(armed, ASK, { option: "Rename it" }, NOW + ARM_MS).phase, "confirm");
  assert.equal(observe(armed, ASK, NOW + ARM_MS - 1), armed);
  assert.equal(observe(armed, ASK, NOW + ARM_MS), null);
  assert.equal(observe(armed, { ...ASK, revision: 2 }, NOW + 1), null, "a new revision disarms it");
});

test("a written answer sends at once as text, and no press changes an answer that is sending", () => {
  const sent = choose(null, ASK, { text: "Rename it to flows_payload" }, NOW);

  assert.equal(sent.phase, "posting");
  assert.deepEqual(answerBody(sent), { text: "Rename it to flows_payload", revision: 1, call: sent.call });
  assert.equal(choose(sent, ASK, { option: "Keep the name" }, NOW), sent);
});

test("a command option arms for its side panel; only its run there sends, with run: true", () => {
  const armed = choose(null, COMMAND, { option: "Run it" }, NOW);

  assert.equal(armed.phase, "armed");
  assert.equal(choose(armed, COMMAND, { option: "Run it" }, NOW + 1).phase, "armed");
  assert.equal(answerBody(armed, true).run, true);
  assert.equal(observe(armed, COMMAND, NOW + ARM_MS), null);
});

test("a resend of the same choice keeps its call id, and another choice or revision gets a new one", () => {
  const lost = { ...posting(), phase: "unknown" };
  const again = choose(lost, ASK, { option: "Rename it", again: true }, NOW + 1);
  const other = choose(lost, ASK, { option: "Keep the name", again: true }, NOW + 1);
  const moved = choose(lost, { ...ASK, revision: 2 }, { option: "Rename it", again: true }, NOW + 1);

  assert.match(lost.call, /^[0-9a-f-]{36}$/);
  assert.equal(again.phase, "posting");
  assert.equal(again.call, lost.call);
  assert.notEqual(other.call, lost.call);
  assert.notEqual(moved.call, lost.call);
});

test("a 200 clears the draft", () => {
  assert.equal(reply(posting(), { status: 200, body: { ok: "ask 88 answered" } }, NOW).phase, "sent");
});

test("each refusal gives the choice back and says why in the panel's words", () => {
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
    const after = reply(posting(), result, NOW);
    assert.equal(after.phase, phase, notice);
    assert.equal(after.notice, notice);
    assert.equal(after.option, "Rename it");
    assert.equal(choose(after, ASK, { option: "Keep the name" }, NOW).phase, "confirm", "the options take presses again");
  }
});

test("a lost write is outcome-unknown until a read shows the answer, or the person sends again", () => {
  const unknown = reply(posting(), { status: 0, body: {}, unknown: true }, NOW);

  assert.equal(unknown.phase, "unknown");
  assert.equal(unknown.notice, "The connection dropped before Priory confirmed this. It may be stored: send it again, and it is stored once.");
  assert.equal(choose(unknown, ASK, { option: "Keep the name" }, NOW), unknown);
  assert.equal(observe(unknown, ASK, NOW), unknown);
  assert.equal(observe(unknown, { ...ASK, state: "answered", answer: "Rename it" }, NOW).phase, "sent");
});

test("a digit key picks its option only alone: a browser shortcut or a held key picks nothing", () => {
  const key = (key, extra = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, repeat: false, ...extra });

  assert.equal(optionKey(key("2")), 1);
  assert.equal(optionKey(key("9")), 8);
  for (const event of [key("2", { ctrlKey: true }), key("2", { metaKey: true }), key("2", { altKey: true }), key("2", { repeat: true }), key("0"), key("a")]) {
    assert.equal(optionKey(event), -1, JSON.stringify(event));
  }
});

test("a step's ask offers no dismissal, and an agent's ask does", () => {
  assert.equal(dismissable({ ...ASK, dismissable: false }), false);
  assert.equal(dismissable({ ...ASK, dismissable: true }), true);
});

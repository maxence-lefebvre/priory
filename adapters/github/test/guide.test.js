import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { answerHash, grade, parseGuide, scoreKey } from "../lib/guide.js";

const PR = { repo: "mlfbvr/priory", number: 164 };
const sha = (text) => `sha256:${createHash("sha256").update(text).digest("hex").slice(0, 16)}`;

const guide = (quiz) => ({
  version: 1,
  head: "9f3c2e1",
  map: ["Adds the Costs and Flows panels to Herdr."],
  steps: [{ title: "Back edges run down a lane", file: "adapters/herdr/flows.js", lines: "40-56" }],
  quiz,
});

const QUIZ = {
  pass: 1,
  questions: [
    { id: "1", question: "Where?", options: ["Up", "In a lane"], answer: sha("mlfbvr/priory#164:1:In a lane"), cite: "flows.rs:88" },
    { id: "2", question: "When?", options: ["Once", "Always"], answer: sha("mlfbvr/priory#164:2:Once") },
  ],
};

test("a guide block parses to its map, steps and quiz", () => {
  const parsed = parseGuide(JSON.stringify(guide(QUIZ)));
  assert.equal(parsed.error, undefined);
  assert.equal(parsed.guide.head, "9f3c2e1");
  assert.deepEqual(parsed.guide.steps[0], { title: "Back edges run down a lane", file: "adapters/herdr/flows.js", lines: "40-56" });
  assert.equal(parsed.guide.quiz.questions.length, 2);
});

test("a guide that does not parse or breaks the shape says why", () => {
  const cases = [
    ["{", /JSON/],
    [JSON.stringify({ ...guide(QUIZ), version: 2 }), /version/],
    [JSON.stringify({ ...guide(QUIZ), head: "not a sha" }), /head/],
    [JSON.stringify({ ...guide(QUIZ), map: [1] }), /map/],
    [JSON.stringify({ ...guide(QUIZ), steps: Array(9).fill(guide().steps[0]) }), /8 steps/],
    [JSON.stringify({ ...guide(QUIZ), steps: [{ title: "x", file: "a", lines: "x" }] }), /lines/],
    [JSON.stringify(guide({ pass: 1, questions: Array(5).fill(QUIZ.questions[0]) })), /4 questions/],
    [JSON.stringify(guide({ pass: 3, questions: QUIZ.questions })), /pass/],
    [JSON.stringify(guide({ pass: 1, questions: [{ ...QUIZ.questions[0], answer: "In a lane" }] })), /answer/],
    [JSON.stringify(guide({ pass: 1, questions: [{ ...QUIZ.questions[0], options: ["only"] }] })), /options/],
    [JSON.stringify({ ...guide(QUIZ), map: ["x".repeat(9000)] }), /8 KiB/],
    [JSON.stringify({ ...guide(QUIZ), quiz: null }), /quiz/],
    [JSON.stringify({ ...guide(QUIZ), steps: [null] }), /step/],
    ["null", /version/],
  ];
  for (const [text, why] of cases) {
    const parsed = parseGuide(text);
    assert.equal(parsed.guide, undefined, text.slice(0, 80));
    assert.match(parsed.error, why);
  }
});

test("an answer hash is the first 16 hex digits of SHA-256 of repo#number:id:option", async () => {
  assert.equal(await answerHash(PR, "1", "In a lane"), sha("mlfbvr/priory#164:1:In a lane"));
});

test("a quiz grades each answer against its hash and passes at its pass mark", async () => {
  const { guide: parsed } = parseGuide(JSON.stringify(guide(QUIZ)));

  const right = await grade(parsed, PR, { 1: "In a lane", 2: "Always" });
  const none = await grade(parsed, { ...PR, number: 165 }, { 1: "In a lane", 2: "Once" });

  assert.deepEqual(right, { right: 1, total: 2, passed: true, correct: { 1: true, 2: false } });
  assert.deepEqual(none, { right: 0, total: 2, passed: false, correct: { 1: false, 2: false } });
});

test("a score belongs to one head sha and one guide: a push or a new guide gives a new key", async () => {
  const text = JSON.stringify(guide(QUIZ));
  const key = await scoreKey(PR, "4c1d2e0aa", text);

  assert.equal(await scoreKey(PR, "4c1d2e0aa", text), key);
  assert.notEqual(await scoreKey(PR, "5d2e3f1bb", text), key);
  assert.notEqual(await scoreKey(PR, "4c1d2e0aa", `${text} `), key);
});

test("any mutation of a valid block parses or says why, and never throws", () => {
  const valid = JSON.stringify(guide(QUIZ));
  const swaps = ["null", "0", "[]", "{}", '""', "true"];
  let seed = 7;
  const random = (n) => (seed = (seed * 48271) % 2147483647) % n;
  for (let i = 0; i < 2000; i++) {
    const at = random(valid.length);
    const mutated = random(2)
      ? valid.slice(0, at) + valid.slice(at + 1 + random(4))
      : valid.replace(/"[^"]*"|\d+|\[\]/g, (match) => (random(12) ? match : swaps[random(swaps.length)]));
    const parsed = parseGuide(mutated);
    assert.ok(parsed.guide || typeof parsed.error === "string", mutated);
  }
});

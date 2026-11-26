import { afterEach, beforeEach, test } from "node:test";
import assert from "node:assert/strict";

const kept = new Map([["priory.token", "t"]]);
globalThis.localStorage = { getItem: (key) => kept.get(key) ?? null, setItem: (key, value) => kept.set(key, value), removeItem: (key) => kept.delete(key) };
const { acts, draft, disarmTask, pressTask, writeTask } = await import("./acts.js");
const { session, signIn } = await import("./session.js");

let answer;
let sent;
const realFetch = globalThis.fetch;
beforeEach(() => {
  sent = [];
  answer = async () => ({ status: 200, body: { ok: "done" } });
  globalThis.fetch = async (path, options) => {
    sent.push([path, JSON.parse(options.body)]);
    const { status, body } = await answer();
    return new Response(JSON.stringify(body), { status });
  };
  acts.set({ armed: null, busy: null, notices: new Map(), drafts: new Map() });
  if (!session.get().token) signIn("t");
});
afterEach(() => {
  disarmTask();
  globalThis.fetch = realFetch;
});
const draftOf = (task) => acts.get().drafts.get(task);

test("a Say draft outlives its page: task A, then B, then A again", () => {
  draft(1, "to a");
  draft(2, "to b");
  assert.deepEqual([draftOf(1), draftOf(2)], ["to a", "to b"]);
});

test("a Say that went clears its draft and says so", async () => {
  draft(1, "hello ");
  await writeTask(1, "say", { text: "hello" });
  assert.deepEqual(sent, [["/api/tasks/1/say", { text: "hello" }]]);
  assert.equal(draftOf(1), undefined);
  assert.equal(acts.get().notices.get(1), "Said: done.");
});

test("a Say that failed keeps its draft and says why", async () => {
  answer = async () => ({ status: 503, body: {} });
  draft(1, "hello");
  await writeTask(1, "say", { text: "hello" });
  assert.equal(draftOf(1), "hello");
  assert.match(acts.get().notices.get(1), /no priory daemon runs/);
  answer = async () => { throw new TypeError("offline"); };
  await writeTask(1, "say", { text: "hello" });
  assert.equal(draftOf(1), "hello");
  assert.match(acts.get().notices.get(1), /may have applied this/);
});

test("text typed while a Say is in flight stays as the next draft", async () => {
  draft(1, "hello");
  answer = async () => (draft(1, "hello, and more"), { status: 200, body: { ok: "done" } });
  await writeTask(1, "say", { text: "hello" });
  assert.equal(draftOf(1), "hello, and more");
});

test("Cancel arms on the first press and sends on the second, Resume sends at once", async () => {
  await pressTask(1, "cancel", { revision: 7 });
  assert.deepEqual([sent, acts.get().armed.task, acts.get().armed.act], [[], 1, "cancel"]);
  await pressTask(1, "cancel", { revision: 7 });
  assert.deepEqual([sent.map(([path]) => path), acts.get().armed], [["/api/tasks/1/cancel"], null]);
  await pressTask(1, "resume", { revision: 7 });
  assert.equal(sent.length, 2);
});

test("a press on another action moves the arm, and the arm runs out", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  pressTask(1, "cancel", { revision: 7 });
  pressTask(2, "cancel", { revision: 7 });
  assert.deepEqual([sent, acts.get().armed.task], [[], 2]);
  t.mock.timers.tick(60000);
  assert.equal(acts.get().armed, null);
  pressTask(2, "cancel", { revision: 7 });
  assert.deepEqual(sent, []);
});

test("a refused token signs out", async () => {
  answer = async () => ({ status: 401, body: {} });
  await writeTask(1, "resume", { revision: 7 });
  assert.deepEqual([session.get().token, session.get().signInError, kept.has("priory.token")], [null, "The daemon refused this token. Paste the current one.", false]);
});

test("Cancel and Release carry the revision their page drew, and the server takes none with the rest", async () => {
  for (const act of ["cancel", "cancel", "release", "resume", "revive", "revive"]) await pressTask(1, act, { revision: 281 });
  assert.deepEqual(sent, [["/api/tasks/1/cancel", { revision: 281 }], ["/api/tasks/1/release", { revision: 281 }], ["/api/tasks/1/resume", {}], ["/api/tasks/1/revive", {}]]);
});

test("an act with no revision on screen neither arms nor sends", async () => {
  for (const act of ["cancel", "cancel", "resume"]) await pressTask(1, act, {});
  assert.deepEqual([sent, acts.get().armed], [[], null]);
});

test("another runner's task goes by its generation", async () => {
  await pressTask("laptop:ENG-1", "resume", { generation: 3, body: { generation: 3 } });
  assert.deepEqual(sent, [["/api/tasks/laptop:ENG-1/resume", { generation: 3 }]]);
});

import { test } from "node:test";
import assert from "node:assert/strict";

globalThis.localStorage = { getItem: () => "t", setItem() {}, removeItem() {} };
const { api, signOut } = await import("./session.js");

// A server that tags each body, and answers 304 to the tag it gave, with the revision it is at now.
function server(revision) {
  const sent = [];
  globalThis.fetch = async (path, { headers }) => {
    sent.push(headers["If-None-Match"] ?? null);
    const tag = `"${path}"`;
    if (headers["If-None-Match"] === tag) return { status: 304, ok: false, headers: new Headers({ "Priory-Revision": String(revision.at) }) };
    return { status: 200, ok: true, headers: new Headers({ ETag: tag }), json: async () => ({ revision: revision.at, asks: [{ id: 1 }] }) };
  };
  return sent;
}

test("a listing that did not change keeps its body and takes the revision the 304 names", async () => {
  const revision = { at: 4 };
  const sent = server(revision);

  const first = await api("/api/asks");
  revision.at = 6;
  const second = await api("/api/asks");

  assert.deepEqual(sent, [null, '"/api/asks"']);
  assert.deepEqual(second, { revision: 6, asks: [{ id: 1 }] });
  assert.equal(second.asks, first.asks);
});

test("a sign-out forgets the tags, so the next session reads every body", async () => {
  const sent = server({ at: 1 });
  await api("/api/desk");
  signOut("bye");
  await api("/api/desk");
  assert.deepEqual(sent, [null, null]);
});

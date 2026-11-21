import { test } from "node:test";
import assert from "node:assert/strict";
import { address, call, request } from "../lib/request.js";

const TOKEN = "s3cret-token";

test("an address becomes an origin: https beyond loopback, http only on loopback", () => {
  assert.deepEqual(address(" xps13.tail1234.ts.net "), { origin: "https://xps13.tail1234.ts.net" });
  assert.deepEqual(address("127.0.0.1:7340"), { origin: "http://127.0.0.1:7340" });
  assert.deepEqual(address("http://localhost:7340/"), { origin: "http://localhost:7340" });
  assert.deepEqual(address("https://xps13.tail1234.ts.net/"), { origin: "https://xps13.tail1234.ts.net" });
});

test("an address with a path, a query, credentials or plain http beyond loopback is refused", () => {
  for (const bad of ["", "http://xps13.tail1234.ts.net", "https://h/api", "https://h/?token=x", "https://u:p@h", "ftp://h", "https://h#x"]) {
    assert.ok(address(bad).error, bad);
  }
});

test("a request carries the token only in its Authorization header, never in its URL", () => {
  const read = request("https://h", TOKEN, "/api/tasks?pr=github%3Ao%2Fr%237");
  const write = request("https://h", TOKEN, "/api/asks/3/answer", { option: "Merge", revision: 2 });

  for (const built of [read, write]) {
    assert.ok(!built.url.includes(TOKEN), built.url);
    assert.equal(built.init.headers.Authorization, `Bearer ${TOKEN}`);
    assert.equal(built.init.credentials, "omit");
    assert.equal(built.init.redirect, "error");
  }
  assert.equal(read.url, "https://h/api/tasks?pr=github%3Ao%2Fr%237");
  assert.equal(read.init.method, "GET");
  assert.equal(write.init.method, "POST");
  assert.equal(write.init.headers["Content-Type"], "application/json");
  assert.deepEqual(JSON.parse(write.init.body), { option: "Merge", revision: 2 });
});

test("a request refuses a path outside /api, so nothing else gets the token", () => {
  for (const bad of ["https://evil.example/api/x", "//evil.example/api", "/hooks/github", "api/tasks"]) {
    assert.throws(() => request("https://h", TOKEN, bad), /path/, bad);
  }
});

const reply = (status, body = {}) => async () => ({ status, json: async () => body });

test("a call reads the status and body, and a lost connection or a 504 is outcome-unknown", async () => {
  assert.deepEqual(await call(reply(200, { ok: "answered" }), "https://h", TOKEN, "/api/x"), { status: 200, body: { ok: "answered" } });
  assert.equal((await call(reply(504), "https://h", TOKEN, "/api/x", {})).unknown, true);
  const lost = await call(async () => { throw new TypeError("Failed to fetch"); }, "https://h", TOKEN, "/api/x", {});
  assert.deepEqual(lost, { status: 0, body: {}, unknown: true });
});

test("a read that fails on the network is unreachable, not unknown", async () => {
  const lost = await call(async () => { throw new TypeError("Failed to fetch"); }, "https://h", TOKEN, "/api/x");
  assert.deepEqual(lost, { status: 0, body: {}, unreachable: true });
});

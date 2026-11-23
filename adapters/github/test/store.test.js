import { test } from "node:test";
import assert from "node:assert/strict";
import { clear, get, put } from "../lib/store.js";

import "./idb.js";

test("sign-out's clear forgets the token and every pull request's last read and kept answer", async () => {
  await put("config", { origin: "http://127.0.0.1:7340", token: "secret" });
  await put("last:github:owner/repo#7", { task: { id: 12, brief: { request: "Fix the docs" } } });
  await put("draft:github:owner/repo#7", { id: 88, option: "Rename it" });

  await clear();

  for (const key of ["config", "last:github:owner/repo#7", "draft:github:owner/repo#7"]) assert.equal(await get(key), undefined, key);
});

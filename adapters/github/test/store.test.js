import { test } from "node:test";
import assert from "node:assert/strict";
import { clear, get, put } from "../lib/store.js";

// The few IndexedDB calls store.js makes, on one in-memory object store.
const data = new Map();
const later = (fn) => setTimeout(fn, 0);
globalThis.indexedDB = {
  open() {
    const opening = {};
    const db = {
      close() {},
      transaction() {
        const transaction = {};
        const request = (apply) => {
          const pending = {};
          later(() => {
            pending.result = apply();
            transaction.oncomplete?.();
          });
          return pending;
        };
        transaction.objectStore = () => ({
          get: (key) => request(() => data.get(key)),
          put: (value, key) => request(() => data.set(key, value) && key),
          delete: (key) => request(() => data.delete(key) && undefined),
          clear: () => request(() => data.clear()),
        });
        return transaction;
      },
    };
    later(() => {
      opening.result = db;
      opening.onsuccess();
    });
    return opening;
  },
};

test("sign-out's clear forgets the token and every pull request's last read and kept answer", async () => {
  await put("config", { origin: "http://127.0.0.1:7340", token: "secret" });
  await put("last:github:owner/repo#7", { task: { id: 12, brief: { request: "Fix the docs" } } });
  await put("draft:github:owner/repo#7", { id: 88, option: "Rename it" });

  await clear();

  for (const key of ["config", "last:github:owner/repo#7", "draft:github:owner/repo#7"]) assert.equal(await get(key), undefined, key);
});

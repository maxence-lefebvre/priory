import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const COPIES = {
  "structure.css": "design/tokens/structure.css",
  "themes.css": "design/tokens/themes.css",
  "mona-sans-200-900.woff2": "design/fonts/mona-sans-200-900.woff2",
  "jetbrains-mono-400-500.woff2": "design/fonts/jetbrains-mono-400-500.woff2",
};

test("the extension's vendor files are copies of design/, since an unpacked extension cannot reach outside its folder", () => {
  for (const [copy, source] of Object.entries(COPIES)) {
    const read = (path) => readFileSync(new URL(path, import.meta.url));
    assert.ok(read(`../vendor/${copy}`).equals(read(`../../../${source}`)), `copy ${source} to adapters/github/vendor/${copy}`);
  }
});
